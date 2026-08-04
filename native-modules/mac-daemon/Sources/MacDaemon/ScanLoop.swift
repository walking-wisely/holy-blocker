import Foundation

// The pure, time-injected scheduler bridging `EventHooks` callbacks and `Scanner`. Mirrors the
// Windows daemon's `scan_loop` — see docs/architecture/edge-daemons.md for the cadence vocabulary
// and docs/components/mac-daemon/plan.md module 9 for the full spec.
//
// `tick(now:)` takes the clock as an explicit parameter — nothing in this file reads the wall
// clock — so debounce and cadence rules are unit-testable with a fake clock and no real capture.

// MARK: - Configuration

/// Timing knobs for the scheduler. Defaults follow edge-daemons.md's recommended cadence.
public struct ScanLoopConfig: Equatable, Sendable {
    /// Only the last event in a burst within this window produces a capture.
    public let debounceInterval: TimeInterval
    /// Image classifier cadence — "every ~500 ms while the surface is eligible".
    public let imageInterval: TimeInterval
    /// OCR does not run more often than this unless forced by an event or a frame difference.
    public let ocrMinInterval: TimeInterval
    /// OCR always runs at least this often, even against an unchanged frame.
    public let ocrMaxInterval: TimeInterval

    public init(
        debounceInterval: TimeInterval = 0.15,
        imageInterval: TimeInterval = 0.5,
        ocrMinInterval: TimeInterval = 1.0,
        ocrMaxInterval: TimeInterval = 2.0
    ) {
        self.debounceInterval = debounceInterval
        self.imageInterval = imageInterval
        self.ocrMinInterval = ocrMinInterval
        self.ocrMaxInterval = ocrMaxInterval
    }
}

// MARK: - Eligibility

/// Whether the current surface should be scanned at all this tick. `ScanLoop` never inspects the
/// system itself — the caller (which owns the AX/session/display glue) feeds this in.
public struct SurfaceEligibility: Equatable, Sendable {
    public let hasEligibleSurface: Bool
    public let displayAsleep: Bool
    public let sessionLocked: Bool

    public init(
        hasEligibleSurface: Bool = true, displayAsleep: Bool = false, sessionLocked: Bool = false
    ) {
        self.hasEligibleSurface = hasEligibleSurface
        self.displayAsleep = displayAsleep
        self.sessionLocked = sessionLocked
    }

    public var isEligible: Bool {
        hasEligibleSurface && !displayAsleep && !sessionLocked
    }

    public static let eligible = SurfaceEligibility()
    public static let noSurface = SurfaceEligibility(hasEligibleSurface: false)
    public static let asleep = SurfaceEligibility(displayAsleep: true)
    public static let locked = SurfaceEligibility(sessionLocked: true)
}

// MARK: - Frame differencing

/// A cheap, deterministic stand-in for "did the screen change" — sampled rather than a full
/// byte-for-byte compare, since this runs against real Retina-sized frames in production. Only
/// used to gate OCR; the image classifier runs on a fixed cadence regardless (see module 9).
struct FrameFingerprint: Equatable {
    let width: Int
    let height: Int
    let sampleHash: Int

    /// Samples every `stride`-th byte so the cost stays roughly constant regardless of frame
    /// size, rather than hashing every pixel on every tick.
    static func make(from frame: CapturedFrame, stride sampleStride: Int = 97) -> FrameFingerprint {
        guard !frame.isEmpty else { return FrameFingerprint(width: 0, height: 0, sampleHash: 0) }

        var hasher = Hasher()
        hasher.combine(frame.width)
        hasher.combine(frame.height)
        var index = 0
        while index < frame.pixels.count {
            hasher.combine(frame.pixels[index])
            index += sampleStride
        }
        return FrameFingerprint(width: frame.width, height: frame.height, sampleHash: hasher.finalize())
    }
}

// MARK: - Mode application

/// Turns a scanner's raw verdict into the effective one for the given `ProtectionMode`.
///
/// Per docs/decisions/protection-modes.md: `warn` mode downgrades a block-range score to warn;
/// `full` mode passes the raw action through unchanged. `off` mode is handled by the caller —
/// `ScanLoop` never calls the scanner at all in `off` mode, so this function is never reached for
/// it.
///
/// This is plain Swift threshold logic, not a call into `packages/text-policy-ffi`. The plan flags
/// consuming the Rust policy engine over UniFFI for this mapping as an open, cross-platform
/// decision — see module 9's "Where the mode→action decision should live" note — and explicitly
/// scopes this module to the plain-Swift mapping until that is decided.
func applyProtectionMode(_ mode: ProtectionMode, to verdict: ScanVerdict) -> ScanVerdict {
    let effective: ScanAction
    switch mode {
    case .full:
        effective = verdict.rawAction
    case .warn:
        effective = verdict.rawAction == .block ? .warn : verdict.rawAction
    case .off:
        // Never reached in practice — ScanLoop skips scanning entirely in `.off`. Fall back to
        // `.allow` rather than leaking a raw block/warn action if this is ever called directly.
        effective = .allow
    }
    return ScanVerdict(
        action: effective, rawAction: verdict.rawAction, score: verdict.score, source: verdict.source,
        regions: verdict.regions)
}

// MARK: - ScanLoop

/// The scheduler. Owns no thread and reads no clock — `tick(now:)` is a pure function of its
/// current state, the injected `now`, and whatever `capture`/`scanner` return.
public final class ScanLoop {
    private let capture: ScreenCapturing
    private let scanner: Scanner
    private let config: ScanLoopConfig

    private var mode: ProtectionMode
    private var eligibility: SurfaceEligibility

    /// Set by `signalEvent`, cleared once the debounce window has elapsed with no further event —
    /// trailing-edge debounce, so only the last event in a burst resolves.
    private var pendingEventAt: Date?

    private var lastImageScanAt: Date?
    private var lastOCRScanAt: Date?
    private var lastOCRFingerprint: FrameFingerprint?

    /// The most recent verdict produced by any cadence, worst-action-wins across ties within a
    /// tick. Convenience for callers that just want "what's the state right now".
    public private(set) var lastVerdict: ScanVerdict?

    public init(
        capture: ScreenCapturing, scanner: Scanner, config: ScanLoopConfig = ScanLoopConfig(),
        mode: ProtectionMode = .full, eligibility: SurfaceEligibility = .eligible
    ) {
        self.capture = capture
        self.scanner = scanner
        self.config = config
        self.mode = mode
        self.eligibility = eligibility
    }

    public func setMode(_ mode: ProtectionMode) {
        self.mode = mode
    }

    public func setEligibility(_ eligibility: SurfaceEligibility) {
        self.eligibility = eligibility
    }

    /// Register a fast-wakeup event from `EventHooks` (foreground change, window move/resize,
    /// scroll) at `now`. Repeated calls before the debounce window elapses simply move the
    /// pending timestamp forward — the discarded intermediate events never produce a capture.
    public func signalEvent(now: Date) {
        pendingEventAt = now
    }

    /// Advance the scheduler by one tick. Returns the verdicts produced this tick, in cadence
    /// order (image first, then OCR/text) — usually zero or one, occasionally both if both
    /// cadences come due on the same tick. Never touches the wall clock itself.
    @discardableResult
    public func tick(now: Date) -> [ScanVerdict] {
        guard mode != .off else { return [] }
        guard eligibility.isEligible else { return [] }

        let frame = capture.currentFrame()
        guard !frame.isEmpty else { return [] }

        // Resolve any pending debounced event exactly once, on the first tick at or after the
        // quiet window — this is the "immediately after a foreground change" trigger for OCR.
        var eventResolvedThisTick = false
        if let pending = pendingEventAt, now.timeIntervalSince(pending) >= config.debounceInterval {
            eventResolvedThisTick = true
            pendingEventAt = nil
        }

        var verdicts: [ScanVerdict] = []

        if shouldRunImageClassifier(now: now) {
            lastImageScanAt = now
            let verdict = scanner.scan(frame)
            verdicts.append(applyProtectionMode(mode, to: verdict))
        }

        let fingerprint = FrameFingerprint.make(from: frame)
        if shouldRunOCR(now: now, fingerprint: fingerprint, forcedByEvent: eventResolvedThisTick) {
            lastOCRScanAt = now
            lastOCRFingerprint = fingerprint
            let verdict = scanner.scan(frame)
            verdicts.append(applyProtectionMode(mode, to: verdict))
        }

        if let worst = verdicts.max(by: { severity($0.action) < severity($1.action) }) {
            lastVerdict = worst
        }

        return verdicts
    }

    private func shouldRunImageClassifier(now: Date) -> Bool {
        guard let last = lastImageScanAt else { return true }
        return now.timeIntervalSince(last) >= config.imageInterval
    }

    /// OCR fires when forced (foreground change or the hard upper-bound cadence), or when the
    /// minimum interval has elapsed *and* the frame actually changed. A cadence-due tick against
    /// an unchanged frame is skipped outright — the frame-differencing dedup the plan calls for —
    /// because `SCStream` going idle already tells us nothing on screen is different, which is
    /// exactly what OCR would otherwise waste cycles rediscovering.
    private func shouldRunOCR(now: Date, fingerprint: FrameFingerprint, forcedByEvent: Bool) -> Bool {
        guard let last = lastOCRScanAt else { return true }

        let elapsed = now.timeIntervalSince(last)
        let dueByMaxInterval = elapsed >= config.ocrMaxInterval
        let dueByMinInterval = elapsed >= config.ocrMinInterval
        let frameChanged = fingerprint != lastOCRFingerprint

        return forcedByEvent || dueByMaxInterval || (dueByMinInterval && frameChanged)
    }

    private func severity(_ action: ScanAction) -> Int {
        switch action {
        case .allow: return 0
        case .warn: return 1
        case .block: return 2
        }
    }
}
