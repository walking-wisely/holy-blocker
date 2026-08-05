import Foundation
import Testing

@testable import MacDaemon

// The overlay alone is a picture drawn on top of content that is still there and still being
// composited: clicking the desktop unfocuses everything and the cover drops, and Mission Control
// renders live previews above a `.screenSaver` window. Hiding the application removes it from both.
// See docs/components/mac-daemon/backlog.md.

@Suite("SuppressionDecision")
struct SuppressionDecisionTests {
    private let policy = SuppressionPolicy(
        protectedBundleIdentifiers: ["com.holyblocker.daemon", "com.apple.finder"], cooldown: 5)
    private let epoch = Date(timeIntervalSince1970: 1_000_000)

    @Test("hides the application a block verdict came from")
    func blockHides() {
        #expect(
            SuppressionDecision.command(
                action: .block, target: "com.apple.TextEdit", policy: policy, lastHiddenAt: nil,
                now: epoch) == .hide(bundleIdentifier: "com.apple.TextEdit"))
    }

    @Test("does nothing for a warn or allow verdict")
    func lesserActionsDoNothing() {
        for action in [ScanAction.warn, .allow] {
            #expect(
                SuppressionDecision.command(
                    action: action, target: "com.apple.TextEdit", policy: policy, lastHiddenAt: nil,
                    now: epoch) == SuppressionCommand.none)
        }
    }

    /// A warn is an interstitial the user is meant to be able to think past; hiding their window is
    /// the block response and nothing weaker should reach for it.
    @Test("does nothing when there is no identified target")
    func noTarget() {
        #expect(
            SuppressionDecision.command(
                action: .block, target: nil, policy: policy, lastHiddenAt: nil, now: epoch)
                == SuppressionCommand.none)
    }

    /// Hiding ourselves would take the overlay off screen, which is the opposite of the point, and
    /// hiding Finder takes the desktop with it.
    @Test("never hides a protected bundle identifier")
    func protectedBundles() {
        for identifier in ["com.holyblocker.daemon", "com.apple.finder"] {
            #expect(
                SuppressionDecision.command(
                    action: .block, target: identifier, policy: policy, lastHiddenAt: nil, now: epoch)
                    == SuppressionCommand.none)
        }
    }

    /// The scan cadence is ~1s and `hide()` can legitimately fail or be undone by the user. Without
    /// a cooldown a refusing application would be asked to hide on every tick forever.
    @Test("does not re-hide the same application inside the cooldown")
    func cooldownSuppressesRepeat() {
        let justHidden = epoch.addingTimeInterval(-1)
        #expect(
            SuppressionDecision.command(
                action: .block, target: "com.apple.TextEdit", policy: policy,
                lastHiddenAt: justHidden, now: epoch) == SuppressionCommand.none)
    }

    @Test("hides again once the cooldown has elapsed")
    func cooldownExpires() {
        let stale = epoch.addingTimeInterval(-5)
        #expect(
            SuppressionDecision.command(
                action: .block, target: "com.apple.TextEdit", policy: policy, lastHiddenAt: stale,
                now: epoch) == .hide(bundleIdentifier: "com.apple.TextEdit"))
    }
}

@Suite("WindowSuppressor")
struct WindowSuppressorTests {
    private func suppressor(clock: @escaping () -> Date) -> (WindowSuppressor, FakeApplicationHider) {
        let hider = FakeApplicationHider()
        let suppressor = WindowSuppressor(
            policy: SuppressionPolicy(
                protectedBundleIdentifiers: ["com.holyblocker.daemon"], cooldown: 5),
            hider: hider,
            now: clock)
        return (suppressor, hider)
    }

    @Test("asks the hider once, then respects the cooldown")
    func hidesOncePerCooldown() {
        var now = Date(timeIntervalSince1970: 1_000_000)
        let (suppressor, hider) = suppressor(clock: { now })

        suppressor.apply(action: .block, target: "com.apple.TextEdit")
        suppressor.apply(action: .block, target: "com.apple.TextEdit")
        #expect(hider.hidden == ["com.apple.TextEdit"])

        now = now.addingTimeInterval(6)
        suppressor.apply(action: .block, target: "com.apple.TextEdit")
        #expect(hider.hidden == ["com.apple.TextEdit", "com.apple.TextEdit"])
    }

    /// Two applications blocking in turn must not share one cooldown — the second is a different
    /// window of content, not a repeat of the first.
    @Test("tracks the cooldown per application")
    func cooldownIsPerApplication() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let (suppressor, hider) = suppressor(clock: { now })

        suppressor.apply(action: .block, target: "com.apple.TextEdit")
        suppressor.apply(action: .block, target: "com.apple.Safari")
        #expect(hider.hidden == ["com.apple.TextEdit", "com.apple.Safari"])
    }

    /// A failed hide must not stamp the cooldown, or one refusal buys the application five quiet
    /// seconds on screen.
    @Test("a refused hide is retried on the next tick")
    func refusedHideIsRetried() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let hider = FakeApplicationHider()
        hider.succeeds = false
        let suppressor = WindowSuppressor(
            policy: SuppressionPolicy(protectedBundleIdentifiers: [], cooldown: 5),
            hider: hider,
            now: { now })

        suppressor.apply(action: .block, target: "com.apple.TextEdit")
        suppressor.apply(action: .block, target: "com.apple.TextEdit")
        #expect(hider.hidden == ["com.apple.TextEdit", "com.apple.TextEdit"])
    }
}
