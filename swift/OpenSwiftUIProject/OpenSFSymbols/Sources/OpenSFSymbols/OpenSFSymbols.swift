//
//  OpenSFSymbols.swift
//  OpenSFSymbols
//
//  Open, cross-platform counterpart to Apple's Darwin-only `SFSymbols` framework — the piece
//  OpenSwiftUI needs to render `Image(systemName:)` off-Apple (Linux/Windows/wasm), where
//  CUICatalog / SF Pro are unavailable.
//
//  Layer 1 (this file) resolves the chain **SF-Symbol name → font → unicode-id**: an SF name maps
//  to an `IconRef` giving the open icon *font*, its file, the glyph name, and its *codepoint* in
//  that font. That is directly render-ready — layer 3 loads the font file and draws the glyph at
//  the codepoint (`draw_glyphs`). This file draws nothing itself.
//
//  Design points:
//   • Font is a *parameter* (`fontPriority`) — a symbol may resolve from whichever configured font
//     has it, so one font missing an icon can be covered by another (cross-font fallback).
//   • The full SF-Symbol name universe is known. A name with no substitution is surfaced
//     (`missingNames`, or `requireIconRef` throws) so gaps are loud — never rendered blank.

/// A resolved icon glyph: the font, its file, the glyph name, and the codepoint to draw.
public struct IconRef: Equatable, Hashable, Sendable {
    /// Open icon font id, e.g. `"tabler"`.
    public let font: String
    /// The font file to load, e.g. `"tabler-icons.ttf"`.
    public let fontFile: String
    /// Glyph name within the font, e.g. `"menu-2"`.
    public let glyph: String
    /// Codepoint of the glyph in `font` (typically a PUA scalar), e.g. `0xEC42`.
    public let codepoint: UInt32
    /// Where the mapping came from — hand-verified vs. best-effort name normalization.
    public let source: Source
    public enum Source: String, Sendable { case curated, auto }

    /// The codepoint as a `Unicode.Scalar` (all icon-font codepoints are valid scalars).
    public var scalar: Unicode.Scalar { Unicode.Scalar(codepoint)! }

    /// Font family name to resolve by (host-side `match_family_style`). Derived from `fontFile`
    /// (e.g. `"tabler-icons.ttf"` → `"tabler-icons"`), matching the TTF's own family name.
    public var fontFamily: String {
        (fontFile.hasSuffix(".ttf") || fontFile.hasSuffix(".otf")) ? String(fontFile.dropLast(4)) : fontFile
    }

    public init(font: String, fontFile: String, glyph: String, codepoint: UInt32, source: Source = .curated) {
        self.font = font; self.fontFile = fontFile; self.glyph = glyph
        self.codepoint = codepoint; self.source = source
    }
}

public struct OpenSFSymbols: Sendable {
    /// Preferred font order. When a symbol maps to more than one font, the first present here wins
    /// — i.e. "which font do I prefer to draw from". Configurable.
    public var fontPriority: [String]

    public init(fontPriority: [String] = OpenSFSymbols.availableFonts) {
        self.fontPriority = fontPriority
    }

    /// Fonts compiled into this build (id → file), in generator priority order.
    public static let fonts: [(id: String, file: String)] = OpenSFSymbolsData.fontsBlob
        .split(separator: "\n").compactMap { line in
            let f = line.split(separator: "\t", maxSplits: 1)
            return f.count == 2 ? (String(f[0]), String(f[1])) : nil
        }
    public static var availableFonts: [String] { fonts.map(\.id) }

    // MARK: - Name universe

    /// Every SF-Symbol name known to this build (SF Symbols 7).
    public static let allSymbolNames: Set<String> = Set(
        OpenSFSymbolsData.namesBlob.split(separator: "\n").map(String.init)
    )
    public func isKnownSymbol(_ name: String) -> Bool { Self.allSymbolNames.contains(name) }

    // MARK: - Mapping table (parsed once)

    static let fontFileByID: [String: String] = Dictionary(uniqueKeysWithValues: fonts.map { ($0.id, $0.file) })

    static let table: [String: [IconRef]] = {
        var out: [String: [IconRef]] = [:]
        for line in OpenSFSymbolsData.mappingBlob.split(separator: "\n") {
            let parts = line.split(separator: "\t", maxSplits: 1)
            guard parts.count == 2 else { continue }
            var refs: [IconRef] = []
            for cand in parts[1].split(separator: "|") {
                let f = cand.split(separator: ":")
                guard f.count == 4, let cp = UInt32(f[2], radix: 16) else { continue }
                let font = String(f[0])
                let src: IconRef.Source = (f[3] == "override") ? .curated : .auto
                refs.append(IconRef(font: font, fontFile: fontFileByID[font] ?? "",
                                    glyph: String(f[1]), codepoint: cp, source: src))
            }
            if !refs.isEmpty { out[String(parts[0])] = refs }
        }
        return out
    }()

    /// All candidate glyphs for a symbol, across fonts (empty if none).
    public func candidates(for name: String) -> [IconRef] { Self.table[name] ?? [] }

    /// The best glyph for a symbol given `fontPriority`, or `nil` if there is no substitution.
    public func iconRef(for name: String) -> IconRef? {
        let cands = candidates(for: name)
        guard !cands.isEmpty else { return nil }
        for font in fontPriority {
            if let hit = cands.first(where: { $0.font == font }) { return hit }
        }
        return cands.first
    }

    // MARK: - Strict resolution (investigation — fail loudly)

    public enum ResolveError: Error, CustomStringConvertible {
        case unknownSymbol(String)
        case noSubstitution(String)
        public var description: String {
            switch self {
            case let .unknownSymbol(n):  return "OpenSFSymbols: '\(n)' is not a known SF Symbol name"
            case let .noSubstitution(n): return "OpenSFSymbols: no open-icon substitution for SF Symbol '\(n)' — add it to Data/overrides.json"
            }
        }
    }
    public func requireIconRef(for name: String) throws -> IconRef {
        guard isKnownSymbol(name) else { throw ResolveError.unknownSymbol(name) }
        guard let ref = iconRef(for: name) else { throw ResolveError.noSubstitution(name) }
        return ref
    }

    // MARK: - Coverage (investigation)

    public static var mappedNames: [String] { table.keys.sorted() }
    public static var missingNames: [String] { allSymbolNames.subtracting(table.keys).sorted() }

    public static func coverageSummary() -> String {
        let mapped = table.count
        let curated = table.values.filter { $0.contains { $0.source == .curated } }.count
        return """
        OpenSFSymbols coverage (SF Symbols 7):
          fonts       : \(availableFonts.joined(separator: ", "))
          names total : \(allSymbolNames.count)
          mapped      : \(mapped)  (curated: \(curated), auto: \(mapped - curated))
          missing     : \(allSymbolNames.count - mapped)
        """
    }
}
