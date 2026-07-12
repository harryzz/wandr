// [wandr Phase 2 · Audio substitution] Replaces eleev's AudioToolbox-based Audio/AudioSource
// (excluded from the target — the sanctioned Audio seam). Same public API; v1 plays SILENTLY.
// TODO: route to wasi:audio (a short PCM blip per merge/add), gated on the @AppStorage audio flag.
import Foundation

enum AudioSource: String {
    case merge = "Merge"
    case add = "Add"

    static func play(condition: @escaping @autoclosure () -> Bool) {
        // Parity with eleev: evaluate the merge/move condition (side-effect-free); sound is a no-op v1.
        if condition() { play(from: .moved) }
        play(from: .merged)
    }
    static func play(from source: GameLogic.State) {
        // v1: silent. TODO: wasi:audio blip for .merged / .moved.
        switch source {
        case .merged: Audio.play(fileNamed: AudioSource.merge.rawValue)
        case .moved:  Audio.play(fileNamed: AudioSource.add.rawValue)
        default: ()
        }
    }
}

enum Audio {
    static func play(fileNamed file: String, of type: String = "mp3") {
        // v1: silent. TODO: decode /assets/audio/<file>.<type> and play via wasi:audio.
    }
}
