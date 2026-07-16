// [wandr Phase 2 · Audio substitution] Real playback for eleev's Audio/AudioSource seam, via
// wasi:audio/pcm. Swift has no MP3 decoder available on wasm (Symphonia — the guest decoder the
// wasi:audio design assumes — is Rust-only), so the merge/add SFX ship pre-decoded to mono 44.1kHz
// PCM16 WAV (converted once, offline, from eleev's real Kenney CC0 mp3s — see Audio/Merge.wav,
// Audio/Add.wav) instead of decoding at runtime. Duplicated to stereo at load time — confirmed
// on-device: this HAL's MMAP path only accepted writes (nonzero `accepted`) once switched from
// mono to stereo. 48kHz is refused too — 44.1kHz is what this device's path actually accepts.
//
// Playback architecture synthesizes the two proven Rust/Kotlin references (neither alone fits a
// multi-write one-shot SFX):
//  - wandr.tetris (short one-shot game sounds — the closest match): feed the ring EVERY tick,
//    writing silence when idle, so it never underruns mid-clip. Confirmed on-device why this
//    matters: a track left idle even briefly drops into a permanent ~1.1s
//    underrun→remove-from-active-list→retry churn (AudioFlinger "BUFFER TIMEOUT: remove track ...
//    due to underrun" on repeat) that a bare write() afterward doesn't cleanly recover from.
//  - wandr-app's playToneAndRelease ("the right way" per its own comment): write, start, then
//    CLOSE the track after enough real time has passed for the clip to actually finish playing —
//    NOT immediately (a bare "pending is empty" check fires as soon as the ring has *accepted*
//    the bytes, well before the device has played them out) and NOT never (its comment warns an
//    always-open track "kept audioserver pumping at ~8-9% CPU forever").
// So: feed silence every tick while a clip is draining out (borrowed from tetris, needed because
// our clips are longer than the ring and take several writes), then close once enough ticks have
// passed for the clip's own duration to elapse (borrowed from wandr-app, sized per-clip rather
// than a fixed 400ms).
import Foundation
import CSwiftSpike
#if canImport(WASILibc)
import WASILibc
#endif

final class WandrAudioPlayer {
    static let shared = WandrAudioPlayer()

    private var playback: wasi_audio_pcm_own_playback_t?
    private var cache: [String: [Float]] = [:]
    // Samples still to be written, in trigger order. Drained a bit every tick by pump();
    // pump() pads with silence when this is empty so the ring is fed every tick regardless.
    private var pending: [Float] = []
    // Extra ticks to keep feeding (silence) after `pending` drains, before closing the track —
    // sized in play() from the clip's own duration (mirrors wandr-app's 400ms post-write close,
    // but per-clip rather than fixed). The ring can only ever hold ~80ms, so this only works at
    // all if the caller ticks fast (~33ms) while `isActive`; a 150ms idle-cadence gap always
    // exceeds the ring, silence or not.
    private var graceTicks = 0
    private static let tickMs = 33.0
    private static let closeMarginTicks = 3

    /// True while pump() needs fast (~33ms) frame-pacing to keep the ring fed. False once fully
    /// idle — pump() becomes a cheap no-op and the caller can drop back to slow/idle cadence.
    var isActive: Bool { !pending.isEmpty || graceTicks > 0 }

    func play(fileNamed name: String) {
        let samples: [Float]
        if let cached = cache[name] {
            samples = cached
        } else {
            guard let loaded = Self.loadStereoPCM16Wav("/assets/\(name).wav") else {
                wlog("audio: failed to load /assets/\(name).wav")
                return
            }
            cache[name] = loaded
            samples = loaded
        }
        pending.append(contentsOf: samples)
        let clipMs = Double(samples.count) / 2.0 / Self.sampleRate * 1000.0 // stereo-interleaved -> ms
        let ticksForClip = Int((clipMs / Self.tickMs).rounded(.up)) + Self.closeMarginTicks
        graceTicks = max(graceTicks, ticksForClip)
    }

    /// Feed the ring, once per onFrame, while `isActive` (call every frame — cheap no-op
    /// otherwise). Matches wandr.tetris's proven per-tick keep-alive; closes the track once
    /// `graceTicks` reaches zero, matching wandr-app's proven delayed-close.
    /// write-then-start ([[feedback_aaudio_gotchas]] #5) — prime the ring before starting.
    func pump() {
        guard isActive, let pb = ensurePlayback() else { return }
        let borrowed = wasi_audio_pcm_borrow_playback(pb)
        if pending.isEmpty {
            graceTicks -= 1
            // ~10ms of silence — enough to keep the ring topped up between ticks without
            // over-buffering (real SFX data must not queue up behind a large silence pad).
            var silence = [Float](repeating: 0, count: 960)
            silence.withUnsafeMutableBufferPointer { buf in
                var list = swift_spike_list_f32_t(ptr: buf.baseAddress, len: buf.count)
                _ = wasi_audio_pcm_method_playback_write(borrowed, &list)
            }
        } else {
            let accepted = pending.withUnsafeMutableBufferPointer { buf -> UInt32 in
                var list = swift_spike_list_f32_t(ptr: buf.baseAddress, len: buf.count)
                return wasi_audio_pcm_method_playback_write(borrowed, &list)
            }
            if accepted > 0 {
                // write() returns FRAMES accepted, not samples — each stereo frame is 2 interleaved
                // samples (L,R). Removing only `accepted` samples (not accepted*2) desyncs the
                // buffer by half every call; it drifts until an odd 1-sample remainder is left that
                // can never form a complete frame, and write() rejects it forever (confirmed
                // on-device: pendingLeft stuck at 1, accepted=0, looping — this was that bug).
                pending.removeFirst(min(Int(accepted) * 2, pending.count))
            }
        }
        if !started {
            var err = wasi_audio_pcm_audio_error_t()
            _ = wasi_audio_pcm_method_playback_start(borrowed, &err)
            started = true
        }
        if !isActive {
            wasi_audio_pcm_playback_drop_own(pb)
            playback = nil
            started = false
        }
    }

    /// Drop any open track and discard whatever was queued — called on a background/foreground
    /// transition (see WandrReactor's onLifecycleChanged). Matches wandr.audio.player's
    /// reopen_at_current: a track can go stale across a role transition (the OS may reclaim the
    /// AAudio stream, or the ring depth the host grants differs by role), so a bare write() into
    /// the old handle afterward doesn't reliably work — the next play() call reopens fresh.
    func reset() {
        if let pb = playback {
            wasi_audio_pcm_playback_drop_own(pb)
        }
        playback = nil
        started = false
        pending.removeAll()
        graceTicks = 0
    }

    private var started = false

    // 44.1kHz, not 48kHz — a known-bad rate on this device/HAL for this path.
    private static let sampleRate = 44100.0

    private func ensurePlayback() -> wasi_audio_pcm_own_playback_t? {
        if let playback { return playback }
        var config = wasi_audio_pcm_stream_config_t(
            sample_rate: UInt32(Self.sampleRate),
            channel_layout: UInt8(WASI_AUDIO_PCM_CHANNEL_LAYOUT_STEREO),
            format: UInt8(WASI_AUDIO_PCM_FORMAT_PCM_F32),
            class_: UInt8(WASI_AUDIO_PCM_STREAM_CLASS_MEDIA)
        )
        var ret = wasi_audio_pcm_own_playback_t()
        var err = wasi_audio_pcm_audio_error_t()
        guard wasi_audio_pcm_static_playback_open(&config, &ret, &err) else {
            wlog("audio: playback open failed, err=\(err)")
            return nil
        }
        playback = ret
        return ret
    }

    // Minimal RIFF/WAVE PCM16 parser: walk chunks to find "data", ignore everything else (fmt
    // is fixed at asset-prep time — mono/44.1kHz/16-bit, see build-t2iles.sh's ffmpeg conversion).
    // Each decoded mono sample is duplicated into interleaved L/R (see file header).
    private static func loadStereoPCM16Wav(_ path: String) -> [Float]? {
        guard let file = fopen(path, "rb") else { return nil }
        defer { fclose(file) }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        while true {
            let read = buffer.withUnsafeMutableBytes { fread($0.baseAddress, 1, $0.count, file) }
            if read <= 0 { break }
            data.append(contentsOf: buffer[0..<read])
        }
        let bytes = [UInt8](data)
        guard bytes.count >= 12,
              Array(bytes[0..<4]) == Array("RIFF".utf8),
              Array(bytes[8..<12]) == Array("WAVE".utf8) else { return nil }

        var offset = 12
        var dataStart: Int?
        var dataLen = 0
        while offset + 8 <= bytes.count {
            let id = Array(bytes[offset..<(offset + 4)])
            let size = Int(bytes[offset + 4]) | Int(bytes[offset + 5]) << 8
                     | Int(bytes[offset + 6]) << 16 | Int(bytes[offset + 7]) << 24
            let bodyStart = offset + 8
            if id == Array("data".utf8) {
                dataStart = bodyStart
                dataLen = size
                break
            }
            offset = bodyStart + size + (size % 2)
        }
        guard let start = dataStart, dataLen > 0, start + dataLen <= bytes.count else { return nil }

        var samples: [Float] = []
        samples.reserveCapacity((dataLen / 2) * 2)
        var i = start
        while i + 1 < start + dataLen {
            let raw = Int16(bitPattern: UInt16(bytes[i]) | (UInt16(bytes[i + 1]) << 8))
            let f = Float(raw) / 32768.0
            samples.append(f); samples.append(f) // L, R
            i += 2
        }
        return samples
    }
}
