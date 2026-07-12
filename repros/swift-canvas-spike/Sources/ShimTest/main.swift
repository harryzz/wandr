// [wandr Phase 2] Foundation test: eleev-style imports + @AppStorage (Store) + AudioServices (Audio)
// all compile with ZERO OpenSwiftUI/OpenCombine references — proving the shim set.
import SwiftUI
import Combine
import AudioToolbox
import Foundation

final class Model: ObservableObject {
    @Published var n: Int = 0
}
struct ContentView: View {
    @ObservedObject var model: Model
    @State private var flag = false
    @AppStorage("isAudioEnabled") var isAudioEnabled: Bool = true   // Store shim
    var body: some View {
        VStack {
            Text("n = \(model.n)")
            Text(isAudioEnabled ? "audio on" : "audio off")
        }
    }
}

// Audio shim exercise (eleev's Audio.play pattern)
func playMerge() {
    var sound: SystemSoundID = 0
    let url = URL(fileURLWithPath: "/assets/Merge.mp3")
    AudioServicesCreateSystemSoundID(url as CFURL, &sound)
    AudioServicesPlaySystemSound(sound)
}

print("shim-test: SwiftUI + Combine + @AppStorage + AudioToolbox all compiled")
