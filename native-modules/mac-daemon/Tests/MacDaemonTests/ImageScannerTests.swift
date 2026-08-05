import Foundation
import ImageSandboxFFI
import Testing

@testable import MacDaemon

// Module 18. Everything here runs against `FakeImageClassifier` and
// `InlineClassificationDispatcher`, so no test loads the Rust library or needs a model artifact —
// the same arrangement `AccessibilityScannerTests` uses for the policy engine.

// MARK: - Helpers

/// A frame with plausible geometry. The contents do not matter: the fake classifier returns a
/// scripted outcome, and what this suite tests is the scheduling and mapping around it.
private func frame(width: Int = 320, height: Int = 200) -> CapturedFrame {
    CapturedFrame(
        pixels: [UInt8](repeating: 0x80, count: width * height * 4), width: width, height: height,
        captured: Date())
}

/// A scanner whose classification runs inline, so a verdict is available on the same call that
/// started it — deterministic, no waiting on a queue.
private func inlineScanner(
    _ classifier: FakeImageClassifier, minimumInterval: TimeInterval = 0,
    now: @escaping () -> Date = { Date() }
) -> ImageScanner {
    ImageScanner(
        classifier: classifier, dispatcher: InlineClassificationDispatcher(),
        minimumInterval: minimumInterval, now: now)
}

// MARK: - Mapping the Rust outcome

@Test func aBlockOutcomeBecomesABlockVerdictTaggedAsImageSourced() {
    let verdict = ImageMapping.scanVerdict(for: .block(score: 0.83))

    #expect(verdict.action == .block)
    #expect(verdict.rawAction == .block)
    #expect(verdict.source == .image)
    #expect(abs(verdict.score - 0.83) < 0.0001)
}

@Test func anAllowOutcomeKeepsTheScoreTheModelProduced() {
    // The margin under the threshold is the only signal that would show the operating point is
    // wrong for screen content, so it must survive the mapping rather than being flattened.
    let verdict = ImageMapping.scanVerdict(for: .allow(score: 0.44))

    #expect(verdict.action == .allow)
    #expect(abs(verdict.score - 0.44) < 0.0001)
}

@Test func anAllowThatNeverReachedTheModelScoresZero() {
    // `ScanVerdict.score` is not optional, so the absence has to land somewhere. Zero is right for
    // the verdict — nothing downstream branches on it — and the distinction survives in the
    // scanner's log line instead.
    #expect(ImageMapping.scanVerdict(for: .allow(score: nil)).score == 0)
}

@Test func aBlockNeverCarriesLocatedRegions() {
    // The tiled geometry knows which tile scored highest, but a 224-wide band is not a located
    // object. content-classification.md: an empty list on a non-allow verdict means "cover the
    // whole surface", never "nothing to do".
    #expect(ImageMapping.scanVerdict(for: .block(score: 0.9)).regions.isEmpty)
}

@Test func thereIsNoWarnBandOnTheImagePath() {
    // A probability has no warn band, and inventing one here would be this daemon making up an
    // operating point the measurement never established. `ProtectionMode` is where a block
    // legitimately becomes a warn, and that lives in ScanLoop.
    let outcomes: [ImageOutcome] = [
        .allow(score: nil), .allow(score: 0.0), .allow(score: 0.4649), .block(score: 0.4650),
        .block(score: 1.0),
    ]

    for outcome in outcomes {
        #expect(ImageMapping.scanVerdict(for: outcome).action != .warn)
    }
}

// MARK: - Scanning

@Test func aBlockingFrameProducesABlockVerdict() {
    let classifier = FakeImageClassifier(outcome: .block(score: 0.9))
    let scanner = inlineScanner(classifier)

    #expect(scanner.scan(frame()).action == .block)
}

@Test func theFramesGeometryIsHandedToTheClassifierUnchanged() {
    // A transposed or mis-sized hand-off does not fail — it classifies a different image and still
    // returns a plausible score, which is the silent class of bug this pins down.
    let classifier = FakeImageClassifier()
    let scanner = inlineScanner(classifier)

    scanner.scan(frame(width: 1512, height: 982))

    #expect(classifier.classifications.count == 1)
    #expect(classifier.classifications[0].width == 1512)
    #expect(classifier.classifications[0].height == 982)
    // Tightly packed BGRA: four bytes a pixel, no row padding.
    #expect(classifier.classifications[0].byteCount == 1512 * 982 * 4)
}

@Test func anEmptyFrameIsNeverHandedToTheClassifier() {
    // The pre-first-frame state. Classifying nothing would burn the interval slot and, on the real
    // classifier, cost an FFI round trip per tick for as long as capture is starved.
    let classifier = FakeImageClassifier(outcome: .block(score: 0.9))
    let scanner = inlineScanner(classifier)

    let verdict = scanner.scan(.empty(captured: Date()))

    #expect(classifier.classifications.isEmpty)
    #expect(verdict.action == .allow)
}

@Test func aScanBeforeAnyClassificationCompletesAllows() {
    // With a dispatcher that never runs the work, the scanner has no verdict yet. Allowing is
    // right here — there is no evidence of anything — and is distinct from the rate-limit case
    // below, where a real previous verdict exists.
    struct NeverDispatcher: ClassificationDispatching {
        func dispatch(_ work: @escaping @Sendable () -> Void) {}
    }
    let scanner = ImageScanner(
        classifier: FakeImageClassifier(outcome: .block(score: 0.9)),
        dispatcher: NeverDispatcher(), minimumInterval: 0)

    #expect(scanner.scan(frame()).action == .allow)
}

// MARK: - Rate limiting, and why it repeats rather than allows

@Test func aTickInsideTheIntervalRepeatsTheLastVerdictRatherThanAllowing() {
    // The rule `AccessibilityScanner` established, and the reason it is load-bearing: `ScanLoop`
    // and the overlay above it are driven by what comes back here, so an `.allow` between
    // classifications would tear the interstitial down and rebuild it over content that never
    // stopped being blocked.
    var instant = Date(timeIntervalSince1970: 1000)
    let classifier = FakeImageClassifier(outcome: .block(score: 0.9))
    let scanner = inlineScanner(classifier, minimumInterval: 0.5, now: { instant })

    #expect(scanner.scan(frame()).action == .block)

    // Well inside the interval, and the classifier now says allow. The cached block must persist.
    classifier.outcome = .allow(score: 0.0)
    instant = instant.addingTimeInterval(0.1)

    #expect(scanner.scan(frame()).action == .block)
    #expect(classifier.classifications.count == 1)
}

@Test func theIntervalElapsingLetsANewVerdictReplaceTheCachedOne() {
    var instant = Date(timeIntervalSince1970: 1000)
    let classifier = FakeImageClassifier(outcome: .block(score: 0.9))
    let scanner = inlineScanner(classifier, minimumInterval: 0.5, now: { instant })

    #expect(scanner.scan(frame()).action == .block)

    classifier.outcome = .allow(score: 0.01)
    instant = instant.addingTimeInterval(0.6)

    #expect(scanner.scan(frame()).action == .allow)
    #expect(classifier.classifications.count == 2)
}

@Test func aClassificationAlreadyInFlightIsNotJoinedByAnother() {
    // The main-thread guarantee depends on this: with inference taking longer than the cadence,
    // ticks must not queue up behind each other or the backlog grows without bound.
    final class HoldingDispatcher: ClassificationDispatching, @unchecked Sendable {
        private let lock = NSLock()
        private var pending: [@Sendable () -> Void] = []

        func dispatch(_ work: @escaping @Sendable () -> Void) {
            lock.lock()
            defer { lock.unlock() }
            pending.append(work)
        }

        var count: Int {
            lock.lock()
            defer { lock.unlock() }
            return pending.count
        }

        func runAll() {
            lock.lock()
            let work = pending
            pending = []
            lock.unlock()
            work.forEach { $0() }
        }
    }

    let dispatcher = HoldingDispatcher()
    let classifier = FakeImageClassifier(outcome: .block(score: 0.9))
    // Interval 0, so only the in-flight guard can be what stops the second dispatch.
    let scanner = ImageScanner(
        classifier: classifier, dispatcher: dispatcher, minimumInterval: 0)

    scanner.scan(frame())
    scanner.scan(frame())
    scanner.scan(frame())

    #expect(dispatcher.count == 1)

    dispatcher.runAll()
    #expect(classifier.classifications.count == 1)
    #expect(scanner.completedClassifications == 1)
    #expect(scanner.scan(frame()).action == .block)
}

@Test func completedClassificationsCountsOnlyFinishedWork() {
    let classifier = FakeImageClassifier()
    let scanner = inlineScanner(classifier)

    #expect(scanner.completedClassifications == 0)
    scanner.scan(frame())
    #expect(scanner.completedClassifications == 1)
}

// MARK: - The dispatcher seam

@Test func theBackgroundQueueRunsTheWorkOffTheCallingThread() async {
    // The whole reason this seam exists: `ScanLoop.tick` is driven by a main-run-loop Timer, and
    // up to 15 ONNX passes on that thread twice a second is a visible stutter in the overlay.
    let classifier = FakeImageClassifier(outcome: .block(score: 0.9))
    let scanner = ImageScanner(
        classifier: classifier, dispatcher: BackgroundClassificationQueue(), minimumInterval: 0)

    // The first scan returns before any classification has completed.
    #expect(scanner.scan(frame()).action == .allow)

    // ...and the verdict lands shortly afterwards, without the caller having blocked.
    var attempts = 0
    while scanner.completedClassifications == 0 && attempts < 100 {
        try? await Task.sleep(nanoseconds: 10_000_000)
        attempts += 1
    }

    #expect(scanner.completedClassifications == 1)
    #expect(scanner.scan(frame()).action == .block)
}

// MARK: - ScanLoop driving two scanners

@Test func eachCadenceDrivesItsOwnScanner() {
    // The split this module adds: before it, one `Scanner` served both cadences, which would have
    // meant running the image classifier on the ~1s text cadence as well as the ~500ms image one.
    let capture = FakeScreenCapture(frame: frame())
    let imageScanner = CountingScanner(verdict: .allow)
    let textScanner = CountingScanner(verdict: .allow)
    let loop = ScanLoop(
        capture: capture, imageScanner: imageScanner, textScanner: textScanner)

    let start = Date(timeIntervalSince1970: 1000)
    // The first tick brings both cadences due.
    loop.tick(now: start)
    #expect(imageScanner.calls == 1)
    #expect(textScanner.calls == 1)

    // 0.6s later only the image cadence is due (ocrMinInterval is 1.0 and the frame is unchanged).
    loop.tick(now: start.addingTimeInterval(0.6))
    #expect(imageScanner.calls == 2)
    #expect(textScanner.calls == 1)
}

@Test func theWorstVerdictAcrossBothScannersWins() {
    // A blocking image beside allowing text must block. `ScanLoop` already reduced worst-wins
    // across cadences; this pins that it still does now that the two are different objects.
    let capture = FakeScreenCapture(frame: frame())
    let loop = ScanLoop(
        capture: capture,
        imageScanner: CountingScanner(verdict: .block),
        textScanner: CountingScanner(verdict: .allow))

    loop.tick(now: Date(timeIntervalSince1970: 1000))

    #expect(loop.lastVerdict?.action == .block)
}

@Test func theSingleScannerInitStillDrivesBothCadences() {
    // The convenience init the scheduling tests use. It has to keep meaning what it did.
    let scanner = CountingScanner(verdict: .allow)
    let loop = ScanLoop(capture: FakeScreenCapture(frame: frame()), scanner: scanner)

    loop.tick(now: Date(timeIntervalSince1970: 1000))

    #expect(scanner.calls == 2)
}

/// Counts calls and returns a fixed action. Local to this suite; the scheduling suite has its own
/// doubles keyed to what it measures.
///
/// `MacDaemon.Scanner` is qualified because `Foundation.Scanner` exists and wins the unqualified
/// lookup — conforming to the wrong one compiles here and fails at every call site instead.
private final class CountingScanner: MacDaemon.Scanner {
    private(set) var calls = 0
    private let action: ScanAction

    init(verdict action: ScanAction) {
        self.action = action
    }

    func scan(_ frame: CapturedFrame) -> ScanVerdict {
        calls += 1
        return ScanVerdict(
            action: action, rawAction: action, score: action == .block ? 0.9 : 0, source: .image)
    }
}
