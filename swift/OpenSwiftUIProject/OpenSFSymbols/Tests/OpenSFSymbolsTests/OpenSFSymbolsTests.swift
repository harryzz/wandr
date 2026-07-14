import XCTest
@testable import OpenSFSymbols

final class OpenSFSymbolsTests: XCTestCase {
    func testNameUniverseLoaded() {
        XCTAssertGreaterThan(OpenSFSymbols.allSymbolNames.count, 6000)
        XCTAssertTrue(OpenSFSymbols.allSymbolNames.contains("text.justify"))
        XCTAssertTrue(OpenSFSymbols.allSymbolNames.contains("arrow.counterclockwise.circle"))
    }

    func testFontRegistryLoaded() {
        XCTAssertTrue(OpenSFSymbols.availableFonts.contains("tabler"))
        XCTAssertEqual(OpenSFSymbols.fonts.first(where: { $0.id == "tabler" })?.file, "tabler-icons.ttf")
    }

    func testChainSFNameToFontToUnicode() {
        // The whole point: SF name -> (font, codepoint), directly render-ready.
        let s = OpenSFSymbols()
        let ham = s.iconRef(for: "text.justify")
        XCTAssertEqual(ham?.font, "tabler")
        XCTAssertEqual(ham?.fontFile, "tabler-icons.ttf")
        XCTAssertEqual(ham?.glyph, "menu-2")
        XCTAssertEqual(ham?.codepoint, 0xEC42)          // real Tabler codepoint
        XCTAssertEqual(ham?.source, .curated)

        let reset = s.iconRef(for: "arrow.counterclockwise.circle")
        XCTAssertEqual(reset?.glyph, "rotate")
        XCTAssertEqual(reset?.codepoint, 0xEB16)
    }

    func testFontPriorityAndFallback() {
        // With a font that lacks a symbol first, resolution falls back to a font that has it.
        let s = OpenSFSymbols(fontPriority: ["nonexistent-font", "tabler"])
        XCTAssertEqual(s.iconRef(for: "text.justify")?.font, "tabler")
    }

    func testStrictResolveThrowsForInvestigation() {
        let s = OpenSFSymbols()
        XCTAssertThrowsError(try s.requireIconRef(for: "not.a.real.symbol.zzz")) { e in
            guard case OpenSFSymbols.ResolveError.unknownSymbol = e else { return XCTFail("wrong \(e)") }
        }
        let missing = OpenSFSymbols.missingNames.first!
        XCTAssertThrowsError(try s.requireIconRef(for: missing)) { e in
            guard case OpenSFSymbols.ResolveError.noSubstitution = e else { return XCTFail("wrong \(e)") }
        }
    }

    func testCoverageMathAndCodepointValidity() {
        XCTAssertEqual(OpenSFSymbols.mappedNames.count + OpenSFSymbols.missingNames.count,
                       OpenSFSymbols.allSymbolNames.count)
        // every mapped candidate has a valid Unicode scalar codepoint
        for name in OpenSFSymbols.mappedNames.prefix(200) {
            for ref in OpenSFSymbols().candidates(for: name) {
                XCTAssertNotNil(Unicode.Scalar(ref.codepoint))
            }
        }
        print(OpenSFSymbols.coverageSummary())
    }
}
