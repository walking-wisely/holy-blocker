import Foundation
import ImageSandboxFFI

// Module 18 — the daemon's second real `Scanner`, and the first thing that looks at a pixel.
//
// `AccessibilityScanner` reads what applications *declare*; this reads what is actually drawn. That
// makes it the answer to two gaps the first live e2e pass recorded (see
// docs/components/mac-daemon/backlog.md): content in a window that never has focus, and content
// with no text at all. It is not the whole answer — a captured frame covers only the displays being
// captured — but unlike the AX walk it does not care which window is frontmost.
//
// Three shape decisions, and the first is the one that matters:
//
// 1. **Classification does not run on the calling thread.** `ScanLoop.tick` is driven by a `Timer`
//    on the main run loop, which is the same thread AppKit draws the overlay on. A frame yields up
//    to 15 ONNX passes; blocking the main thread for that, twice a second, is a visible stutter in
//    the interstitial it is trying to put up. So `scan(_:)` dispatches the work and returns the
//    most recent completed verdict. The cost is one cadence of latency (~500 ms) before a new
//    verdict lands, which is well inside the debounce the loop already applies.
// 2. **A tick with work in flight repeats the last verdict, never `.allow`** — the same rule
//    `AccessibilityScanner` established and for the same reason: `.allow` between classifications
//    would tear the overlay down and rebuild it over content that never stopped being blocked.
// 3. **The classifier is behind `ImageClassifying`**, so this file and its tests never load the
//    Rust library, mirroring `PolicyEngineHandle`.

// MARK: - The classifier seam

/// One frame classification, behind a protocol so this scanner is testable without the Rust
/// library and without a model artifact — mirroring `PolicyEngineHandle`, `CommandRunner`,
/// `PermissionProbing`, `ScreenCapturing` and `AXElementProbing`.
///
/// The generated `ImageOutcome` crosses this boundary deliberately rather than being mirrored into
/// a Swift-native type, for the same reason `Verdict` does on the text path: the crate's API is the
/// contract, and a mirror here would be a second place to update.
public protocol ImageClassifying: Sendable {
    func classify(pixels: [UInt8], width: Int, height: Int) -> ImageOutcome
}

/// The real classifier, over UniFFI.
///
/// Serialized because the generated `ImageGuard` makes no thread-safety promise and this is
/// reachable from the scanner's background queue.
public final class RealImageClassifier: ImageClassifying, @unchecked Sendable {
    private let lock = NSLock()
    private let guardHandle: ImageGuard

    public init(guardHandle: ImageGuard) {
        self.guardHandle = guardHandle
    }

    /// Loads a model from disk. Throws rather than degrading silently: a daemon that reports image
    /// scanning as on while classifying nothing is the failure this constructor exists to prevent.
    /// Callers that want the degraded mode ask for it explicitly with `disabled()`.
    ///
    /// Both thresholds are required and have no built-in fallback: a threshold belongs to a model
    /// *and* a geometry, and the caller is responsible for supplying values calibrated for
    /// `modelPath`.
    public convenience init(modelPath: String, sexyThreshold: Float, explicitThreshold: Float) throws {
        self.init(
            guardHandle: try ImageGuard.withModel(
                modelPath: modelPath, sexyThreshold: sexyThreshold,
                explicitThreshold: explicitThreshold))
    }

    /// No model: allows every frame. What runs when no artifact has been provisioned.
    public static func disabled() -> RealImageClassifier {
        RealImageClassifier(guardHandle: ImageGuard.disabled())
    }

    /// The score at or above which this classifier blocks, for logging beside a verdict.
    public var threshold: Float {
        lock.lock()
        defer { lock.unlock() }
        return guardHandle.threshold()
    }

    /// The score at or above which this classifier warns.
    public var sexyThreshold: Float {
        lock.lock()
        defer { lock.unlock() }
        return guardHandle.sexyThreshold()
    }

    public func classify(pixels: [UInt8], width: Int, height: Int) -> ImageOutcome {
        lock.lock()
        defer { lock.unlock() }
        // `CapturedFrame.pixels` is BGRA and tightly packed — `PixelBufferCopy.depad` guarantees
        // both. Passing the wrong layout does not fail; it scores a colour-swapped image, which is
        // why the parameter is explicit on the Rust side rather than assumed.
        return guardHandle.classifyFrame(
            pixels: Data(pixels), width: UInt32(width), height: UInt32(height), layout: .bgra)
    }
}

/// Test double for `ImageClassifying`, in the library so integration harnesses can use it too —
/// same placement as `FakePolicyEngine` and `FakeAXElementProbe`.
public final class FakeImageClassifier: ImageClassifying, @unchecked Sendable {
    private let lock = NSLock()
    private var _outcome: ImageOutcome
    private var _classifications: [(width: Int, height: Int, byteCount: Int)] = []

    public init(outcome: ImageOutcome = .allow(score: 0.0)) {
        self._outcome = outcome
    }

    /// What the next classification returns. Settable so a test can watch a cached verdict be
    /// replaced.
    public var outcome: ImageOutcome {
        get {
            lock.lock()
            defer { lock.unlock() }
            return _outcome
        }
        set {
            lock.lock()
            defer { lock.unlock() }
            _outcome = newValue
        }
    }

    /// Every classification asked for, in order. Records the frame's shape rather than its pixels —
    /// this daemon does not hold on to what was on screen.
    public var classifications: [(width: Int, height: Int, byteCount: Int)] {
        lock.lock()
        defer { lock.unlock() }
        return _classifications
    }

    public func classify(pixels: [UInt8], width: Int, height: Int) -> ImageOutcome {
        lock.lock()
        defer { lock.unlock() }
        _classifications.append((width: width, height: height, byteCount: pixels.count))
        return _outcome
    }
}

// MARK: - Running the work off the calling thread

/// Where a classification runs. Injected so tests are deterministic: the fake runs the work inline,
/// so a test can assert on the verdict from the very next `scan(_:)` without waiting on a queue.
public protocol ClassificationDispatching: Sendable {
    func dispatch(_ work: @escaping @Sendable () -> Void)
}

/// The real one: a single serial background queue, so at most one inference is ever in flight and
/// the main thread is never the one doing it.
public struct BackgroundClassificationQueue: ClassificationDispatching {
    private let queue: DispatchQueue

    public init(
        queue: DispatchQueue = DispatchQueue(
            label: "com.holyblocker.daemon.image-scan", qos: .userInitiated)
    ) {
        self.queue = queue
    }

    public func dispatch(_ work: @escaping @Sendable () -> Void) {
        queue.async(execute: work)
    }
}

/// Runs the work immediately on the calling thread. For tests, and for the `image-scan` CLI verb,
/// which has no run loop to keep responsive.
public struct InlineClassificationDispatcher: ClassificationDispatching {
    public init() {}

    public func dispatch(_ work: @escaping @Sendable () -> Void) {
        work()
    }
}

// MARK: - Mapping the Rust outcome onto this daemon's vocabulary

/// The conversion between `packages/image-sandbox-ffi` and `Scanner.swift`, pulled out of the
/// scanner so it is testable as the pure function it is — the same shape as `PolicyMapping`.
enum ImageMapping {
    /// `ImageOutcome` onto a `ScanVerdict`.
    ///
    /// **This used to say a probability has no warn band — that was wrong in the direction that
    /// mattered.** It was true of the two-class model this daemon shipped with, which could only
    /// ever say safe-vs-explicit; it stopped being true once the classifier gained a `sexy` class
    /// with its own measured-or-not threshold to compare against (`image-sandbox`'s `sandbox.rs`).
    /// `ImageOutcome.warn` now maps onto `.warn` directly, the same as the text path's `Blur`
    /// narrowing does, and `ProtectionMode` still applies on top in `ScanLoop` — a warn can still
    /// be downgraded to allow, the same as a block can.
    ///
    /// **`regions` is always empty.** The tiled geometry knows which tile scored highest and could
    /// in principle report it, but a tile is a 224-wide band of the frame rather than a located
    /// object, and `content-classification.md` is explicit that an empty list on a non-allow verdict
    /// means "cover the whole surface". Populating it with tile bounds would claim a localization
    /// the model does not do.
    static func scanVerdict(for outcome: ImageOutcome) -> ScanVerdict {
        switch outcome {
        case .allow(let score):
            // A frame that never reached the model has no score. Reported as 0.0 because
            // `ScanVerdict.score` is not optional — the distinction is preserved in the log line
            // the scanner emits, not in the verdict, since nothing downstream acts on it.
            return ScanVerdict(
                action: .allow, rawAction: .allow, score: Double(score ?? 0), source: .image,
                regions: [])
        case .warn(let score):
            return ScanVerdict(
                action: .warn, rawAction: .warn, score: Double(score), source: .image, regions: [])
        case .block(let score):
            return ScanVerdict(
                action: .block, rawAction: .block, score: Double(score), source: .image, regions: [])
        }
    }
}

// MARK: - The scanner

/// Classifies the captured frame with the Rust image sandbox.
///
/// Not thread-safe for concurrent `scan(_:)` calls — one scan at a time, from `ScanLoop`'s thread,
/// matching every other `Scanner`. Its internal state *is* guarded, because the classification it
/// dispatches completes on another thread.
public final class ImageScanner: Scanner, @unchecked Sendable {
    /// Matches `ScanLoopConfig.imageInterval`. The loop already paces the image cadence, so this is
    /// a floor rather than the primary gate — it exists so a caller that ticks faster (the CLI
    /// verb, a future `EventHooks` burst) cannot queue inference faster than it completes.
    public static let defaultMinimumInterval: TimeInterval = 0.5

    private let classifier: ImageClassifying
    private let dispatcher: ClassificationDispatching
    private let minimumInterval: TimeInterval
    private let now: () -> Date

    private let state = State()

    /// Everything touched from both the calling thread and the classification queue.
    private final class State: @unchecked Sendable {
        private let lock = NSLock()
        private var _verdict = ImageScanner.allowVerdict
        private var _inFlight = false
        private var _lastStartedAt: Date?
        private var _completedCount = 0

        var verdict: ScanVerdict {
            lock.lock()
            defer { lock.unlock() }
            return _verdict
        }

        var completedCount: Int {
            lock.lock()
            defer { lock.unlock() }
            return _completedCount
        }

        /// Claims the slot if nothing is in flight and the interval has elapsed. Returns whether
        /// the caller should start a classification — one atomic test-and-set, so two ticks landing
        /// close together cannot both dispatch.
        func claim(at instant: Date, minimumInterval: TimeInterval) -> Bool {
            lock.lock()
            defer { lock.unlock() }
            guard !_inFlight else { return false }
            if let last = _lastStartedAt, instant.timeIntervalSince(last) < minimumInterval {
                return false
            }
            _inFlight = true
            _lastStartedAt = instant
            return true
        }

        func finish(with verdict: ScanVerdict) {
            lock.lock()
            defer { lock.unlock() }
            _verdict = verdict
            _inFlight = false
            _completedCount += 1
        }
    }

    private static let allowVerdict = ScanVerdict(
        action: .allow, rawAction: .allow, score: 0, source: .image, regions: [])

    public init(
        classifier: ImageClassifying,
        dispatcher: ClassificationDispatching = BackgroundClassificationQueue(),
        minimumInterval: TimeInterval = ImageScanner.defaultMinimumInterval,
        now: @escaping () -> Date = { Date() }
    ) {
        self.classifier = classifier
        self.dispatcher = dispatcher
        self.minimumInterval = minimumInterval
        self.now = now
    }

    /// How many classifications have completed. For the log line and for tests; not part of the
    /// `Scanner` contract.
    public var completedClassifications: Int { state.completedCount }

    /// Returns the most recent completed verdict, starting a new classification if one is due.
    ///
    /// This never blocks on inference. The verdict it returns describes a frame from up to one
    /// cadence ago, which is the deliberate trade described in the file header.
    public func scan(_ frame: CapturedFrame) -> ScanVerdict {
        // An empty frame is the pre-first-frame state. `ScanLoop` already gates on it, but the CLI
        // verb and any future caller do not, and dispatching a classification of nothing would burn
        // the interval slot.
        guard !frame.isEmpty else { return state.verdict }
        guard state.claim(at: now(), minimumInterval: minimumInterval) else { return state.verdict }

        let classifier = self.classifier
        let state = self.state
        // The pixels are copied into the closure rather than the frame being captured by reference:
        // `CapturedFrame` is a value type, so this is the existing buffer, and the caller is free to
        // hand the next tick's frame to `ScreenCapture` while this one is still being scored.
        let pixels = frame.pixels
        let width = frame.width
        let height = frame.height

        dispatcher.dispatch {
            let outcome = classifier.classify(pixels: pixels, width: width, height: height)
            state.finish(with: ImageMapping.scanVerdict(for: outcome))
        }

        return state.verdict
    }
}
