import Testing
@testable import MacDaemon

// See docs/components/mac-daemon/plan.md, session 5 of the first live e2e pass. `overlayIntent
// (forVerdict:)` is the integration pass `OverlayIntent.OverlayVerdictAction`'s doc comment named:
// `Scanner.swift`'s real `ScanAction` now exists, so this bridges it onto the overlay's intent
// without either file importing the other.
@Suite("overlayIntent(forVerdict:)")
struct AgentRenderLoopTests {
    @Test("no verdict yet produces no overlay")
    func noVerdictProducesNone() {
        #expect(overlayIntent(forVerdict: nil) == .none)
    }

    @Test("an allow verdict produces no overlay")
    func allowProducesNone() {
        let verdict = ScanVerdict(action: .allow, rawAction: .allow, score: 0, source: .text, regions: [])
        #expect(overlayIntent(forVerdict: verdict) == .none)
    }

    @Test("a warn verdict produces a mouse-swallowing interstitial")
    func warnProducesInterstitial() {
        let verdict = ScanVerdict(action: .warn, rawAction: .warn, score: 0.6, source: .text, regions: [])
        #expect(overlayIntent(forVerdict: verdict) == .interstitial(swallowsMouseEvents: true))
    }

    @Test("a block verdict produces a mouse-swallowing interstitial")
    func blockProducesInterstitial() {
        let verdict = ScanVerdict(action: .block, rawAction: .block, score: 0.9, source: .text, regions: [])
        #expect(overlayIntent(forVerdict: verdict) == .interstitial(swallowsMouseEvents: true))
    }

    @Test("only the effective action is read, never rawAction")
    func readsEffectiveActionNotRaw() {
        // A warn-mode downgrade: `ScanLoop.applyProtectionMode` produces exactly this shape when a
        // raw block is downgraded to an effective warn. The overlay must follow the effective
        // action — the whole point of the downgrade is to show less than a full block would.
        let downgraded = ScanVerdict(action: .warn, rawAction: .block, score: 0.9, source: .text, regions: [])
        #expect(overlayIntent(forVerdict: downgraded) == .interstitial(swallowsMouseEvents: true))
    }
}
