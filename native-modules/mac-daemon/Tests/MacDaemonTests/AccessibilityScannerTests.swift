import Foundation
import Testing

@testable import MacDaemon
@testable import TextPolicyFFI

// Module 9's first real `Scanner` — see docs/components/mac-daemon/plan.md, session 4 of the first
// live e2e pass. Everything here runs against `FakeAXElementProbe` and `FakePolicyEngine`: no AX
// tree, no Rust library, no clock. `TextPolicyFFITests.swift` is what proves the real engine loads.

// MARK: - Fixtures

/// A one-node tree whose only element carries `text`.
private func singleNodeProbe(_ text: String) -> FakeAXElementProbe {
    FakeAXElementProbe(
        root: AXNodeID(1), nodes: [AXNodeID(1): .init(text: [text], children: [])])
}

private let epoch = Date(timeIntervalSince1970: 1_000_000)

/// A frame the scanner is expected to ignore entirely.
private let anyFrame = CapturedFrame(
    pixels: [0, 0, 0, 255], width: 1, height: 1, captured: epoch)

// MARK: - The 5 → 3 action mapping

@Suite("AccessibilityScanner — Action → ScanAction")
struct AccessibilityScannerMappingTests {
    @Test("Rust Block becomes a Swift block")
    func blockMapsToBlock() {
        #expect(PolicyMapping.scanAction(for: .block) == .block)
    }

    @Test("Rust Warn becomes a Swift warn")
    func warnMapsToWarn() {
        #expect(PolicyMapping.scanAction(for: .warn) == .warn)
    }

    /// Its own test because it is an information-loss decision, not a rename. The Rust policy can
    /// ask for a blur — a partial cover that leaves the surface usable — and this daemon has no
    /// blur visual: `Overlay` covers a whole screen or nothing. Downgrading to `.warn` shows the
    /// interstitial instead, which is the closest thing that exists. Upgrading to `.block` would
    /// be stricter than the policy asked for; dropping to `.allow` would show nothing at all.
    /// Revisit once a region-level overlay exists (see `ScanVerdict.regions`).
    @Test("Rust Blur degrades to a warn, because no blur visual exists yet")
    func blurDegradesToWarn() {
        #expect(PolicyMapping.scanAction(for: .blur) == .warn)
    }

    @Test("Rust Log is a report-only action and allows")
    func logMapsToAllow() {
        #expect(PolicyMapping.scanAction(for: .log) == .allow)
    }

    @Test("Rust Allow allows")
    func allowMapsToAllow() {
        #expect(PolicyMapping.scanAction(for: .allow) == .allow)
    }
}

// MARK: - Score normalization

@Suite("AccessibilityScanner — score normalization")
struct AccessibilityScannerScoreTests {
    @Test("0 and 100 are the ends of the range")
    func endpoints() {
        #expect(PolicyMapping.normalizedScore(0) == 0.0)
        #expect(PolicyMapping.normalizedScore(100) == 1.0)
    }

    @Test("the u32 0–100 scale maps linearly onto 0.0–1.0")
    func linear() {
        #expect(PolicyMapping.normalizedScore(50) == 0.5)
        #expect(PolicyMapping.normalizedScore(7) == 0.07)
    }

    /// The 0–100 range is a contract of `packages/text-policy-ffi`, not something this side can
    /// enforce. Clamping means a future Rust change that widens the range degrades to a saturated
    /// score rather than a `ScanVerdict` that violates its own documented 0.0–1.0 bound.
    @Test("a score above the documented range clamps rather than escaping 0.0–1.0")
    func clamps() {
        #expect(PolicyMapping.normalizedScore(101) == 1.0)
        #expect(PolicyMapping.normalizedScore(UInt32.max) == 1.0)
    }
}

// MARK: - Scanning

@Suite("AccessibilityScanner — scanning")
struct AccessibilityScannerScanTests {
    /// A block is acted on, not only drawn over (`WindowSuppression`), so the verdict has to carry
    /// which application produced it. The rate-limit gate is the case that matters: a cached block
    /// repeated on a gated tick must keep pointing at the application that earned it, or the
    /// response lands on whatever the user switched to in the meantime.
    @Test("reports the walked application, and keeps it through a gated tick")
    func reportsWalkedApplication() {
        let probe = FakeAXElementProbe(
            root: AXNodeID(1),
            nodes: [AXNodeID(1): .init(text: ["some on-screen text"], children: [])],
            lastWalkedApplication: "com.apple.TextEdit")
        let scanner = AccessibilityScanner(
            probe: probe, policy: FakePolicyEngine(verdict: Verdict(action: .block, score: 90)),
            now: { epoch })

        _ = scanner.scan(anyFrame)
        #expect(scanner.lastVerdictApplication == "com.apple.TextEdit")

        // Inside the rate-limit window: the verdict is repeated, and so is its attribution.
        _ = scanner.scan(anyFrame)
        #expect(scanner.lastVerdictApplication == "com.apple.TextEdit")
    }

    @Test("scores the walked text through the policy engine")
    func scoresWalkedText() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("some on-screen text"), policy: policy, now: { epoch })

        let verdict = scanner.scan(anyFrame)

        #expect(policy.evaluations.map(\.text) == ["some on-screen text"])
        #expect(verdict.action == .block)
        #expect(verdict.score == 0.9)
    }

    /// The Rust engine scales a score by how much its source can be trusted, so passing the wrong
    /// `SourceKind` silently changes every verdict this daemon produces.
    @Test("tags the text as coming from the accessibility tree")
    func tagsSourceKind() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .allow, score: 0))
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("text"), policy: policy, now: { epoch })

        _ = scanner.scan(anyFrame)

        #expect(policy.evaluations.map(\.source) == [.accessibilityTree])
    }

    @Test("reports itself as a text-sourced verdict with no located regions")
    func reportsTextSource() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("text"), policy: policy, now: { epoch })

        let verdict = scanner.scan(anyFrame)

        #expect(verdict.source == .text)
        #expect(verdict.regions.isEmpty)
    }

    /// `ScanLoop` owns the `ProtectionMode` downgrade and applies it to whatever comes back here.
    /// A scanner that pre-applied a mode would have it applied twice.
    @Test("leaves action equal to rawAction — ScanLoop applies the protection mode")
    func doesNotApplyProtectionMode() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("text"), policy: policy, now: { epoch })

        let verdict = scanner.scan(anyFrame)

        #expect(verdict.action == verdict.rawAction)
    }

    /// `Scanner.scan` is frame-driven because the image classifier is; AX text is not. The frame
    /// parameter is ignored, including when capture failed outright — an empty frame must not stop
    /// the text path, which touches no pixels.
    @Test("ignores the frame, including an empty one")
    func ignoresFrame() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("text"), policy: policy, now: { epoch })

        let verdict = scanner.scan(.empty(captured: epoch))

        #expect(verdict.action == .block)
        #expect(policy.evaluations.count == 1)
    }

    @Test("passes the walk limits through to the walk")
    func honoursLimits() {
        // A two-node chain, walked with a depth bound of 1: only the root's text is read.
        let probe = FakeAXElementProbe(
            root: AXNodeID(1),
            nodes: [
                AXNodeID(1): .init(text: ["root"], children: [AXNodeID(2)]),
                AXNodeID(2): .init(text: ["child"], children: []),
            ])
        let policy = FakePolicyEngine(verdict: Verdict(action: .allow, score: 0))
        let scanner = AccessibilityScanner(
            probe: probe, policy: policy, limits: AXWalkLimits(maxDepth: 1, maxNodes: 2000),
            now: { epoch })

        _ = scanner.scan(anyFrame)

        #expect(policy.evaluations.map(\.text) == ["root"])
    }
}

// MARK: - Fallbacks

@Suite("AccessibilityScanner — fallbacks")
struct AccessibilityScannerFallbackTests {
    /// No frontmost application, or one exposing nothing to read. This is the common case, not an
    /// error: it is every walk against an app with no AX tree at all.
    @Test("no root allows, and never reaches the policy engine")
    func noRootAllows() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        let scanner = AccessibilityScanner(
            probe: FakeAXElementProbe(root: nil), policy: policy, now: { epoch })

        let verdict = scanner.scan(anyFrame)

        #expect(verdict.action == .allow)
        #expect(verdict.score == 0)
        #expect(policy.evaluations.isEmpty)
    }

    /// An empty result means "this surface exposed no text", never "there is no text on screen" —
    /// see `AccessibilityText`'s header. Allowing is the fail-open contract the rest of the daemon
    /// uses; scoring an empty string would just spend an FFI call to reach the same answer.
    @Test("an empty tree allows, and never reaches the policy engine")
    func emptyTreeAllows() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        let scanner = AccessibilityScanner(
            probe: FakeAXElementProbe(
                root: AXNodeID(1), nodes: [AXNodeID(1): .init(text: [], children: [])]),
            policy: policy, now: { epoch })

        let verdict = scanner.scan(anyFrame)

        #expect(verdict.action == .allow)
        #expect(policy.evaluations.isEmpty)
    }

    @Test("a whitespace-only tree allows, and never reaches the policy engine")
    func whitespaceOnlyTreeAllows() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("   \n\t  "), policy: policy, now: { epoch })

        let verdict = scanner.scan(anyFrame)

        #expect(verdict.action == .allow)
        #expect(policy.evaluations.isEmpty)
    }
}

// MARK: - The real engine

/// Everything above runs against `FakePolicyEngine`. These two run the same scanner over the real
/// Rust library, which is the only thing that proves `RealPolicyEngine` wraps the generated
/// `PolicyEngine` correctly rather than merely compiling against it. Still not a test of policy
/// behaviour — `packages/text-policy` owns that.
@Suite("AccessibilityScanner — over the real policy engine")
struct AccessibilityScannerRealEngineTests {
    @Test("blocks on text the shipped starter dictionary scores as a block")
    func blocksOnDictionaryTerm() {
        // "explicit act" is a term in `text-policy-ffi`'s own starter dictionary, used here for the
        // same reason its Rust tests use it: it is the shipped fixture, not a real list.
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("contains explicit act here"), policy: RealPolicyEngine(),
            now: { epoch })

        let verdict = scanner.scan(anyFrame)

        #expect(verdict.action == .block)
        #expect(verdict.score > 0)
        #expect(verdict.score <= 1.0)
    }

    @Test("allows ordinary text")
    func allowsOrdinaryText() {
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("a perfectly ordinary sentence"), policy: RealPolicyEngine(),
            now: { epoch })

        #expect(scanner.scan(anyFrame).action == .allow)
    }
}

// MARK: - The rate-limit gate

@Suite("AccessibilityScanner — rate-limit gate")
struct AccessibilityScannerRateLimitTests {
    /// `ScanLoop` calls one injected `Scanner` on *both* its ~500 ms image cadence and its ~1–2 s
    /// OCR cadence, so without this gate every image tick would drag a full cross-process AX walk
    /// of up to 2000 nodes behind it.
    @Test("a second scan inside the interval does not walk the tree again")
    func gatesRepeatedWalks() {
        let probe = singleNodeProbe("text")
        let policy = FakePolicyEngine(verdict: Verdict(action: .allow, score: 0))
        var instant = epoch
        let scanner = AccessibilityScanner(
            probe: probe, policy: policy, minimumWalkInterval: 1.0, now: { instant })

        _ = scanner.scan(anyFrame)
        instant = epoch.addingTimeInterval(0.5)
        _ = scanner.scan(anyFrame)

        #expect(policy.evaluations.count == 1)
        #expect(probe.visited == [AXNodeID(1)])
    }

    /// The load-bearing half of the gate. A rate-limited tick must repeat the last verdict rather
    /// than allow: `ScanLoop.lastVerdict` and the overlay above it are driven by what comes back
    /// here, so returning `.allow` between walks would tear the interstitial down and rebuild it
    /// several times a second over content that never stopped being blocked.
    @Test("a gated scan repeats the last verdict rather than allowing")
    func gatedScanRepeatsLastVerdict() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        var instant = epoch
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("text"), policy: policy, minimumWalkInterval: 1.0,
            now: { instant })

        let walked = scanner.scan(anyFrame)
        instant = epoch.addingTimeInterval(0.5)
        let gated = scanner.scan(anyFrame)

        #expect(walked.action == .block)
        #expect(gated == walked)
    }

    @Test("the first scan always walks")
    func firstScanWalks() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("text"), policy: policy, minimumWalkInterval: 1.0,
            now: { epoch })

        #expect(scanner.scan(anyFrame).action == .block)
        #expect(policy.evaluations.count == 1)
    }

    @Test("the interval elapsing re-opens the gate")
    func intervalReopensGate() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .allow, score: 0))
        var instant = epoch
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("text"), policy: policy, minimumWalkInterval: 1.0,
            now: { instant })

        _ = scanner.scan(anyFrame)
        instant = epoch.addingTimeInterval(1.0)
        _ = scanner.scan(anyFrame)

        #expect(policy.evaluations.count == 2)
    }

    /// A walk that found no root still cost the AX round trip. Not stamping the gate would let a
    /// browser with no focused window be hammered on every image tick — the exact case the gate
    /// exists for, since a thin first walk against a browser is expected rather than exceptional.
    @Test("a walk that found nothing still closes the gate")
    func fruitlessWalkStillClosesGate() {
        let probe = FakeAXElementProbe(root: nil)
        let policy = FakePolicyEngine(verdict: Verdict(action: .allow, score: 0))
        var instant = epoch
        let scanner = AccessibilityScanner(
            probe: probe, policy: policy, minimumWalkInterval: 1.0, now: { instant })

        _ = scanner.scan(anyFrame)
        instant = epoch.addingTimeInterval(0.5)
        _ = scanner.scan(anyFrame)

        #expect(probe.rootRequests == 1)
    }

    /// Every scan of a browser's first, thin walk would otherwise be remembered forever. Clearing
    /// on a fresh walk is what lets the overlay come down once the blocked content is gone.
    @Test("a fresh walk replaces the cached verdict")
    func freshWalkReplacesCache() {
        let policy = FakePolicyEngine(verdict: Verdict(action: .block, score: 90))
        var instant = epoch
        let scanner = AccessibilityScanner(
            probe: singleNodeProbe("text"), policy: policy, minimumWalkInterval: 1.0,
            now: { instant })

        #expect(scanner.scan(anyFrame).action == .block)
        policy.verdict = Verdict(action: .allow, score: 0)
        instant = epoch.addingTimeInterval(1.0)

        #expect(scanner.scan(anyFrame).action == .allow)
    }
}
