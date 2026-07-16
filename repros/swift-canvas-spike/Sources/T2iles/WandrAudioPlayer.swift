// [wandr Phase 2 · Audio substitution] Real playback for eleev's Audio/AudioSource seam, via
// wasi:audio/pcm (task 108's proven playback path — the audio-decode-probe FLAC precedent, now
// device-verified for a UI game too). Swift has no MP3 decoder available on wasm (Symphonia — the
// guest decoder the wasi:audio design assumes — is Rust-only), so the merge/add SFX ship
// pre-decoded to mono 48kHz PCM16 WAV (converted once, offline, from eleev's real Kenney CC0 mp3s
// — see Audio/Merge.wav, Audio/Add.wav) instead of decoding at runtime. One playback stream is
// opened lazily and kept open for the app's lifetime; each play() call queues its samples onto it
// — overlapping SFX play back-to-back rather than mixed, an acceptable simplification for short UI
// blips.
import Foundation
import CSwiftSpike
#if canImport(WASILibc)
import WASILibc
#endif

final class WandrAudioPlayer {
    static let shared = WandrAudioPlayer()

    private var playback: wasi_audio_pcm_own_playback_t?
    private var cache: [String: [Float]] = [:]

    func play(fileNamed name: String) {
        let samples: [Float]
        if let cached = cache[name] {
            samples = cached
        } else {
            guard let loaded = Self.loadMonoPCM16Wav("/assets/\(name).wav") else {
                wlog("audio: failed to load /assets/\(name).wav")
                return
            }
            cache[name] = loaded
            samples = loaded
        }
        guard let pb = ensurePlayback() else { return }
        var mutableSamples = samples
        mutableSamples.withUnsafeMutableBufferPointer { buf in
            var list = swift_spike_list_f32_t(ptr: buf.baseAddress, len: buf.count)
            _ = wasi_audio_pcm_method_playback_write(wasi_audio_pcm_borrow_playback(pb), &list)
        }
    }

    private func ensurePlayback() -> wasi_audio_pcm_own_playback_t? {
        if let playback { return playback }
        var config = wasi_audio_pcm_stream_config_t(
            sample_rate: 48000,
            channel_layout: UInt8(WASI_AUDIO_PCM_CHANNEL_LAYOUT_MONO),
            format: UInt8(WASI_AUDIO_PCM_FORMAT_PCM_F32),
            class_: UInt8(WASI_AUDIO_PCM_STREAM_CLASS_MEDIA)
        )
        var ret = wasi_audio_pcm_own_playback_t()
        var err = wasi_audio_pcm_audio_error_t()
        guard wasi_audio_pcm_static_playback_open(&config, &ret, &err) else {
            wlog("audio: playback open failed, err=\(err)")
            return nil
        }
        _ = wasi_audio_pcm_method_playback_start(wasi_audio_pcm_borrow_playback(ret), &err)
        playback = ret
        return ret
    }

    // Minimal RIFF/WAVE PCM16 parser: walk chunks to find "data", ignore everything else (fmt
    // is fixed at asset-prep time — mono/48kHz/16-bit, see build-t2iles.sh's ffmpeg conversion).
    private static func loadMonoPCM16Wav(_ path: String) -> [Float]? {
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
        samples.reserveCapacity(dataLen / 2)
        var i = start
        while i + 1 < start + dataLen {
            let raw = Int16(bitPattern: UInt16(bytes[i]) | (UInt16(bytes[i + 1]) << 8))
            samples.append(Float(raw) / 32768.0)
            i += 2
        }
        return samples
    }
}
