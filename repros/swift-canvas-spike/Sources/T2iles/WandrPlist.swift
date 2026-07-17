// [wandr Phase 2] Store-class seam substitution for eleev's Utils/Plist/PlistConfiguration.
// eleev's own (unmodified) views reference `PlistConfiguration` BY BARE NAME — same-module
// visibility, no import — so the type itself must stay in this target; the actual POSIX
// `/assets` read (the generic Bundle.main-traps-on-wasm workaround every wandr app needs) lives
// once in WandrRuntime's `wandrReadAsset`. This file is just the eleev-API-shaped wrapper
// (`init?(name:)` + `getItem(named:)`) so eleev's views compile UNMODIFIED.
import Foundation
import WandrRuntime

struct PlistConfiguration {
    let name: String
    let xml: Data

    init?(name: String) {
        guard let data = wandrReadAsset(name: name, ext: "plist") else { return nil }
        self.name = name
        self.xml = data
    }

    func getItem(named name: String) -> [String: [String: String]]? {
        return try? PropertyListSerialization.propertyList(
            from: xml, options: .mutableContainersAndLeaves, format: nil
        ) as? [String: [String: String]]
    }
}
