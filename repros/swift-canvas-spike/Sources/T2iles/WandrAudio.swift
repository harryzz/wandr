// [wandr Phase 2 · Audio substitution] Replaces eleev's AudioToolbox-based Audio/AudioSource
// (excluded from the target — the sanctioned Audio seam). eleev's own (unmodified) views
// reference `Audio`/`AudioSource` BY BARE NAME — same-module visibility, no import — so these
// two types stay in this target; the actual playback engine (WandrAudioPlayer, generic
// wasi:audio/pcm) lives once in WandrRuntime.
import Foundation
import WandrRuntime

enum AudioSource: String {
    case merge = "Merge"
    case add = "Add"

    static func play(condition: @escaping @autoclosure () -> Bool) {
        // Parity with eleev: evaluate the merge/move condition (side-effect-free); sound is a no-op v1.
        if condition() { play(from: .moved) }
        play(from: .merged)
    }
    static func play(from source: GameLogic.State) {
        switch source {
        case .merged: Audio.play(fileNamed: AudioSource.merge.rawValue)
        case .moved:  Audio.play(fileNamed: AudioSource.add.rawValue)
        default: ()
        }
    }
}

enum Audio {
    // Assets are pre-decoded WAV (see WandrAudioPlayer) — `type` kept for API parity with eleev's
    // call sites (which pass "mp3") but is otherwise unused; the file on disk is always .wav.
    static func play(fileNamed file: String, of type: String = "mp3") {
        WandrAudioPlayer.shared.play(fileNamed: file)
    }
}
