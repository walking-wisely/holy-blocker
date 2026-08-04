import Foundation
import TextPolicyFFI

// Module 9's first real `Scanner` — see docs/components/mac-daemon/plan.md, session 4 of the first
// live e2e pass. It reads the frontmost window's accessibility text (module 12) and scores it with
// the Rust policy engine (module 16), which is what makes the daemon's first end-to-end path text.
//
// Three shape decisions, all consequences of where this sits rather than preferences:
//
// 1. **The `frame` parameter is ignored.** `Scanner.scan(_:)` is frame-driven because the image
//    classifier is; an AX read is an independent cross-process walk that touches no pixels. Making
//    this scanner frame-driven would have meant changing `Scanner`/`ScanLoop` and their existing
//    tests to describe a source neither of them has. Note the flip side, recorded in the plan and
//    not fixed here: `ScanLoop.tick` gates on `!frame.isEmpty`, so this scanner does not run at all
//    without a Screen Recording grant even though it does not need one.
// 2. **It rate-limits its own walk.** `ScanLoop` calls one injected `Scanner` on both its ~500 ms
//    image cadence and its ~1–2 s OCR cadence, and a full AX walk on the faster one would spend
//    thousands of synchronous cross-process messages a minute rediscovering the same screen.
// 3. **The policy engine is behind `PolicyEngineHandle`.** `Scanner.swift` and `ScanLoop.swift`
//    stay UniFFI-agnostic, and every test below runs without loading the Rust library.

// MARK: - The policy seam

/// One text-policy evaluation, behind a protocol so this scanner is testable without the Rust
/// library — mirroring `CommandRunner`, `PermissionProbing`, `ScreenCapturing` and
/// `AXElementProbing`.
///
/// The generated `Verdict`/`SourceKind` types cross this boundary deliberately rather than being
/// mirrored into Swift-native ones. `packages/text-policy-ffi`'s API is a cross-platform contract
/// shared with Android; a mirror here would be a second place to update and a second chance for
/// this daemon to disagree with the policy it is supposed to be enforcing.
public protocol PolicyEngineHandle: Sendable {
    func evaluate(text: String, source: SourceKind) -> Verdict
}

/// The real engine, over UniFFI. The default is the crate's built-in starter dictionary and its
/// default thresholds (block 80, warn 50) — a placeholder list, exactly as `text-policy-ffi` ships
/// one; a real dictionary is loaded at runtime, not compiled in here.
///
/// Serialized because the generated `PolicyEngine` makes no thread-safety promise and this is
/// reachable from whichever thread the scan timer runs on.
public final class RealPolicyEngine: PolicyEngineHandle, @unchecked Sendable {
    private let lock = NSLock()
    private let engine: PolicyEngine

    public init(engine: PolicyEngine) {
        self.engine = engine
    }

    public convenience init() {
        self.init(engine: PolicyEngine.withBuiltinDictionary())
    }

    public func evaluate(text: String, source: SourceKind) -> Verdict {
        lock.lock()
        defer { lock.unlock() }
        return engine.evaluate(text: text, source: source)
    }
}

/// Test double for `PolicyEngineHandle`, in the library so integration harnesses can use it too —
/// same placement as `FakeCommandRunner` and `FakeAXElementProbe`.
public final class FakePolicyEngine: PolicyEngineHandle, @unchecked Sendable {
    private let lock = NSLock()
    private var _verdict: Verdict
    private var _evaluations: [(text: String, source: SourceKind)] = []

    public init(verdict: Verdict) {
        self._verdict = verdict
    }

    /// What the next evaluation returns. Settable so a test can watch a cached verdict be replaced.
    public var verdict: Verdict {
        get {
            lock.lock()
            defer { lock.unlock() }
            return _verdict
        }
        set {
            lock.lock()
            defer { lock.unlock() }
            _verdict = newValue
        }
    }

    /// Every evaluation asked for, in order.
    public var evaluations: [(text: String, source: SourceKind)] {
        lock.lock()
        defer { lock.unlock() }
        return _evaluations
    }

    public func evaluate(text: String, source: SourceKind) -> Verdict {
        lock.lock()
        defer { lock.unlock() }
        _evaluations.append((text: text, source: source))
        return _verdict
    }
}

// MARK: - Mapping the Rust verdict onto this daemon's vocabulary

/// The two lossy conversions between `packages/text-policy-ffi` and `Scanner.swift`, pulled out of
/// the scanner so they are testable as the pure functions they are.
enum PolicyMapping {
    /// The documented top of the Rust score range. `Verdict.score` is a `u32` 0–100 and each caller
    /// normalizes it itself — see the `text-policy-ffi` row in AGENTS.md.
    private static let maximumPolicyScore = 100.0

    /// Rust's five actions onto this daemon's three.
    ///
    /// `Blur → .warn` is the only one that loses information, and it is a real product decision
    /// rather than a rename: this daemon has no blur visual — `Overlay` covers a whole screen or
    /// nothing — so the closest thing that exists is the interstitial. Upgrading to `.block` would
    /// be stricter than the policy asked for and downgrading to `.allow` would show nothing at all.
    /// Revisit when `ScanVerdict.regions` has something populating it and a partial cover exists.
    ///
    /// `Log → .allow` is not lossy in the same way: it is a report-only action by definition, and
    /// the reporting side is `DaemonIPC`, which carries `rawAction` alongside the effective one.
    static func scanAction(for action: Action) -> ScanAction {
        switch action {
        case .block: return .block
        case .blur: return .warn
        case .warn: return .warn
        case .log: return .allow
        case .allow: return .allow
        }
    }

    /// `u32` 0–100 onto `ScanVerdict.score`'s documented 0.0–1.0.
    ///
    /// Clamped rather than trusted: the range is a contract of the Rust crate, not something this
    /// side can enforce, and a widened range should degrade to a saturated score rather than emit a
    /// `ScanVerdict` that violates its own bound.
    static func normalizedScore(_ score: UInt32) -> Double {
        min(Double(score), maximumPolicyScore) / maximumPolicyScore
    }
}

// MARK: - The scanner

/// Scores the frontmost window's accessibility text against the Rust text policy.
///
/// Not thread-safe: one scan at a time, from `ScanLoop`'s thread, matching `SystemAXProbe`'s own
/// single-walk contract.
public final class AccessibilityScanner: Scanner {
    /// Matches `ScanLoopConfig.ocrMinInterval` — the OCR cadence is the right tier for a scan this
    /// expensive, and the loop's faster image cadence is what this gate is here to absorb.
    public static let defaultMinimumWalkInterval: TimeInterval = 1.0

    private let probe: AXElementProbing
    private let policy: PolicyEngineHandle
    private let limits: AXWalkLimits
    private let minimumWalkInterval: TimeInterval
    private let now: () -> Date

    private var lastWalkAt: Date?
    /// Repeated on a gated tick. See `scan(_:)`.
    private var lastVerdict = AccessibilityScanner.allowVerdict

    private static let allowVerdict = ScanVerdict(
        action: .allow, rawAction: .allow, score: 0, source: .text, regions: [])

    public init(
        probe: AXElementProbing,
        policy: PolicyEngineHandle,
        limits: AXWalkLimits = .standard,
        minimumWalkInterval: TimeInterval = AccessibilityScanner.defaultMinimumWalkInterval,
        now: @escaping () -> Date = { Date() }
    ) {
        self.probe = probe
        self.policy = policy
        self.limits = limits
        self.minimumWalkInterval = minimumWalkInterval
        self.now = now
    }

    /// Walks the frontmost window's accessibility tree and scores what it found. `frame` is ignored
    /// — see the file header.
    ///
    /// A tick inside the rate-limit window repeats the last verdict rather than allowing. This is
    /// not a caching nicety: `ScanLoop.lastVerdict` and the overlay above it are driven by what
    /// comes back here, so an `.allow` between walks would tear the interstitial down and rebuild
    /// it several times a second over content that never stopped being blocked.
    public func scan(_ frame: CapturedFrame) -> ScanVerdict {
        let instant = now()
        guard shouldWalk(at: instant) else { return lastVerdict }
        // Stamped before the walk's outcome is known: a walk that found no root still spent the AX
        // round trip, and a browser with no focused window is a case to back off from, not to
        // retry on the next 500 ms tick.
        lastWalkAt = instant

        let verdict = evaluateFocusedText()
        lastVerdict = verdict
        return verdict
    }

    private func shouldWalk(at instant: Date) -> Bool {
        guard let last = lastWalkAt else { return true }
        return instant.timeIntervalSince(last) >= minimumWalkInterval
    }

    private func evaluateFocusedText() -> ScanVerdict {
        let text = AccessibilityText.extractFocusedText(probe: probe, limits: limits)
        // Empty means "this surface exposed no text", never "there is no text on screen" — see
        // `AccessibilityText`'s header. Allowing is this daemon's fail-open contract, and scoring
        // an empty string would spend an FFI call to reach the same answer.
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return Self.allowVerdict
        }

        let verdict = policy.evaluate(text: text, source: .accessibilityTree)
        let action = PolicyMapping.scanAction(for: verdict.action)
        // `action` equals `rawAction` on purpose: `ScanLoop` owns the `ProtectionMode` downgrade
        // and applies it to whatever comes back from here. Pre-applying it would apply it twice.
        return ScanVerdict(
            action: action, rawAction: action,
            score: PolicyMapping.normalizedScore(verdict.score), source: .text, regions: [])
    }
}
