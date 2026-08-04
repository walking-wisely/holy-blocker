import Foundation
import Testing

@testable import MacDaemon

// MARK: - Test doubles

/// Records how many times `scan` was called and returns scripted verdicts in order, falling back
/// to a fixed verdict once the script is exhausted.
// `Scanner` alone is ambiguous here — `Foundation.NSScanner` is also visible in this file — so
// the conformance is qualified with the module name.
private final class SpyScanner: MacDaemon.Scanner, @unchecked Sendable {
    private(set) var callCount = 0
    private var responses: [ScanVerdict]
    private let fallback: ScanVerdict

    init(
        responses: [ScanVerdict] = [],
        fallback: ScanVerdict = ScanVerdict(action: .allow, rawAction: .allow, score: 0, source: .image)
    ) {
        self.responses = responses
        self.fallback = fallback
    }

    func scan(_ frame: CapturedFrame) -> ScanVerdict {
        callCount += 1
        if !responses.isEmpty {
            return responses.removeFirst()
        }
        return fallback
    }
}

private func makeFrame(
    byte: UInt8 = 0, width: Int = 4, height: Int = 4, captured: Date = Date(timeIntervalSince1970: 0)
) -> CapturedFrame {
    CapturedFrame(
        pixels: [UInt8](repeating: byte, count: width * height * 4), width: width, height: height,
        captured: captured)
}

private func date(_ seconds: TimeInterval) -> Date {
    Date(timeIntervalSince1970: seconds)
}

// MARK: - applyProtectionMode (pure)

@Suite("applyProtectionMode")
struct ApplyProtectionModeTests {
    private func verdict(_ raw: ScanAction) -> ScanVerdict {
        ScanVerdict(action: raw, rawAction: raw, score: 0.9, source: .text)
    }

    @Test("full mode passes the raw action through unchanged", arguments: [
        ScanAction.allow, .warn, .block,
    ])
    func fullPassesThrough(raw: ScanAction) {
        let result = applyProtectionMode(.full, to: verdict(raw))
        #expect(result.action == raw)
        #expect(result.rawAction == raw)
    }

    @Test("warn mode downgrades a block-range score to warn")
    func warnDowngradesBlock() {
        let result = applyProtectionMode(.warn, to: verdict(.block))
        #expect(result.action == .warn)
        #expect(result.rawAction == .block)
    }

    @Test("warn mode leaves warn and allow untouched")
    func warnLeavesLowerActionsAlone() {
        #expect(applyProtectionMode(.warn, to: verdict(.warn)).action == .warn)
        #expect(applyProtectionMode(.warn, to: verdict(.allow)).action == .allow)
    }

    @Test("preserves score, source, and regions")
    func preservesOtherFields() {
        let box = NormalizedRect(x: 0, y: 0, width: 1, height: 1)
        let detection = ImageDetection(label: "x", confidence: 0.5, box: box)
        let raw = ScanVerdict(
            action: .block, rawAction: .block, score: 0.75, source: .image, regions: [detection])

        let result = applyProtectionMode(.warn, to: raw)

        #expect(result.score == 0.75)
        #expect(result.source == .image)
        #expect(result.regions == [detection])
    }
}

// MARK: - Debounce

@Suite("ScanLoop debounce")
struct ScanLoopDebounceTests {
    @Test("only the last event in a burst triggers a forced OCR run")
    func coalescesBurstIntoOneTrigger() {
        // Cadences alone would never fire again after priming, so any extra scanner call must be
        // attributable to the debounced event.
        let config = ScanLoopConfig(
            debounceInterval: 0.1, imageInterval: 100, ocrMinInterval: 100, ocrMaxInterval: 200)
        let capture = FakeScreenCapture(frame: makeFrame())
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, config: config)

        _ = loop.tick(now: date(0))  // primes both cadences
        #expect(scanner.callCount == 2)

        loop.signalEvent(now: date(0.5))
        loop.signalEvent(now: date(0.55))
        loop.signalEvent(now: date(0.58))  // only this last one should matter

        _ = loop.tick(now: date(0.60))  // 0.02s since last event — still inside debounce window
        #expect(scanner.callCount == 2)

        _ = loop.tick(now: date(0.65))  // 0.07s — still inside the window
        #expect(scanner.callCount == 2)

        _ = loop.tick(now: date(0.70))  // 0.12s — window elapsed, event resolves
        #expect(scanner.callCount == 3)

        _ = loop.tick(now: date(0.80))  // resolved already; nothing else due
        #expect(scanner.callCount == 3)
    }

    @Test("a resolved event runs OCR immediately, bypassing the minimum interval")
    func eventBypassesMinInterval() {
        let config = ScanLoopConfig(
            debounceInterval: 0.1, imageInterval: 1000, ocrMinInterval: 1000, ocrMaxInterval: 2000)
        let capture = FakeScreenCapture(frame: makeFrame())
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, config: config)

        _ = loop.tick(now: date(0))
        #expect(scanner.callCount == 2)

        loop.signalEvent(now: date(1))
        let verdicts = loop.tick(now: date(1.2))  // event resolves well before either cadence is due

        #expect(scanner.callCount == 3)
        #expect(verdicts.count == 1)
    }
}

// MARK: - Image cadence

@Suite("ScanLoop image classifier cadence")
struct ScanLoopImageCadenceTests {
    @Test("runs roughly every imageInterval while eligible")
    func runsOnCadence() {
        let config = ScanLoopConfig(
            debounceInterval: 0.1, imageInterval: 0.5, ocrMinInterval: 100, ocrMaxInterval: 200)
        let capture = FakeScreenCapture(frame: makeFrame())
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, config: config)

        _ = loop.tick(now: date(0))  // primes image + ocr
        #expect(scanner.callCount == 2)

        _ = loop.tick(now: date(0.3))  // not due yet
        #expect(scanner.callCount == 2)

        _ = loop.tick(now: date(0.5))  // due
        #expect(scanner.callCount == 3)

        _ = loop.tick(now: date(0.6))  // just ran, not due again
        #expect(scanner.callCount == 3)

        _ = loop.tick(now: date(1.0))  // due again
        #expect(scanner.callCount == 4)
    }
}

// MARK: - OCR cadence + frame differencing

@Suite("ScanLoop OCR cadence and frame differencing")
struct ScanLoopOCRCadenceTests {
    @Test("skips a cadence-due OCR run when the frame is unchanged, but forces it at the max interval")
    func dedupsUnchangedFrameButForcesAtMax() {
        let config = ScanLoopConfig(
            debounceInterval: 0.1, imageInterval: 1000, ocrMinInterval: 1.0, ocrMaxInterval: 2.0)
        let capture = FakeScreenCapture(frame: makeFrame(byte: 1))
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, config: config)

        _ = loop.tick(now: date(0))  // primes both
        #expect(scanner.callCount == 2)

        // Frame unchanged, min interval elapsed — the dedup should skip this OCR run.
        _ = loop.tick(now: date(1.0))
        #expect(scanner.callCount == 2)

        // Still unchanged, but the max interval has now elapsed — OCR must run regardless.
        _ = loop.tick(now: date(2.0))
        #expect(scanner.callCount == 3)
    }

    @Test("a meaningful frame difference forces OCR once the minimum interval has elapsed, not before")
    func frameDifferenceRespectsMinInterval() {
        let config = ScanLoopConfig(
            debounceInterval: 0.1, imageInterval: 1000, ocrMinInterval: 1.0, ocrMaxInterval: 2.0)
        let capture = FakeScreenCapture(frame: makeFrame(byte: 1))
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, config: config)

        _ = loop.tick(now: date(0))  // primes both, fingerprint of byte=1 frame recorded
        #expect(scanner.callCount == 2)

        _ = loop.tick(now: date(2.0))  // still byte=1, max interval forces a run
        #expect(scanner.callCount == 3)

        capture.set(makeFrame(byte: 2))  // meaningfully different content

        // Below the minimum interval since the last OCR run — must not fire early even though the
        // frame changed.
        _ = loop.tick(now: date(2.5))
        #expect(scanner.callCount == 3)

        // Minimum interval has now elapsed and the frame is still different — must fire.
        _ = loop.tick(now: date(3.0))
        #expect(scanner.callCount == 4)
    }
}

// MARK: - Eligibility

@Suite("ScanLoop eligibility gating")
struct ScanLoopEligibilityTests {
    @Test("skips entirely when there is no eligible surface")
    func skipsNoSurface() {
        let capture = FakeScreenCapture(frame: makeFrame())
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, eligibility: .noSurface)

        let verdicts = loop.tick(now: date(0))

        #expect(verdicts.isEmpty)
        #expect(scanner.callCount == 0)
    }

    @Test("skips entirely when the display is asleep")
    func skipsAsleep() {
        let capture = FakeScreenCapture(frame: makeFrame())
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, eligibility: .asleep)

        #expect(loop.tick(now: date(0)).isEmpty)
        #expect(scanner.callCount == 0)
    }

    @Test("skips entirely when the session is locked")
    func skipsLocked() {
        let capture = FakeScreenCapture(frame: makeFrame())
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, eligibility: .locked)

        #expect(loop.tick(now: date(0)).isEmpty)
        #expect(scanner.callCount == 0)
    }

    @Test("skips when capture returns an empty frame")
    func skipsEmptyFrame() {
        let capture = FakeScreenCapture(frame: .empty(captured: date(0)))
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner)

        #expect(loop.tick(now: date(0)).isEmpty)
        #expect(scanner.callCount == 0)
    }

    @Test("resumes once eligibility is restored")
    func resumesAfterEligible() {
        let capture = FakeScreenCapture(frame: makeFrame())
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, eligibility: .locked)

        #expect(loop.tick(now: date(0)).isEmpty)

        loop.setEligibility(.eligible)
        let verdicts = loop.tick(now: date(0.1))

        #expect(!verdicts.isEmpty)
        #expect(scanner.callCount > 0)
    }
}

// MARK: - Protection mode

@Suite("ScanLoop protection mode")
struct ScanLoopProtectionModeTests {
    @Test("off mode never calls the scanner")
    func offModeNeverScans() {
        let capture = FakeScreenCapture(frame: makeFrame())
        let scanner = SpyScanner()
        let loop = ScanLoop(capture: capture, scanner: scanner, mode: .off)

        loop.signalEvent(now: date(0))
        let verdicts = loop.tick(now: date(1))

        #expect(verdicts.isEmpty)
        #expect(scanner.callCount == 0)
    }

    @Test("full mode passes a block verdict through unchanged")
    func fullModePassesBlockThrough() {
        let capture = FakeScreenCapture(frame: makeFrame())
        let blockVerdict = ScanVerdict(action: .block, rawAction: .block, score: 0.95, source: .image)
        let scanner = SpyScanner(fallback: blockVerdict)
        let loop = ScanLoop(capture: capture, scanner: scanner, mode: .full)

        let verdicts = loop.tick(now: date(0))

        #expect(verdicts.allSatisfy { $0.action == .block })
    }

    @Test("warn mode downgrades a block verdict to warn but keeps rawAction as block")
    func warnModeDowngradesBlock() {
        let capture = FakeScreenCapture(frame: makeFrame())
        let blockVerdict = ScanVerdict(action: .block, rawAction: .block, score: 0.95, source: .image)
        let scanner = SpyScanner(fallback: blockVerdict)
        let loop = ScanLoop(capture: capture, scanner: scanner, mode: .warn)

        let verdicts = loop.tick(now: date(0))

        #expect(verdicts.allSatisfy { $0.action == .warn && $0.rawAction == .block })
    }

    @Test("switching mode at runtime changes the very next tick")
    func modeChangeAppliesImmediately() {
        let config = ScanLoopConfig(
            debounceInterval: 0.1, imageInterval: 0.01, ocrMinInterval: 0.01, ocrMaxInterval: 0.02)
        let capture = FakeScreenCapture(frame: makeFrame())
        let blockVerdict = ScanVerdict(action: .block, rawAction: .block, score: 0.95, source: .image)
        let scanner = SpyScanner(fallback: blockVerdict)
        let loop = ScanLoop(capture: capture, scanner: scanner, config: config, mode: .full)

        let firstTick = loop.tick(now: date(0))
        #expect(firstTick.allSatisfy { $0.action == .block })

        loop.setMode(.warn)
        let secondTick = loop.tick(now: date(1))
        #expect(secondTick.allSatisfy { $0.action == .warn })
    }
}

// MARK: - lastVerdict

@Suite("ScanLoop lastVerdict")
struct ScanLoopLastVerdictTests {
    @Test("keeps the most severe verdict produced in a tick that runs both cadences")
    func worstOfBothCadences() {
        let capture = FakeScreenCapture(frame: makeFrame())
        let warnVerdict = ScanVerdict(action: .warn, rawAction: .warn, score: 0.5, source: .image)
        let blockVerdict = ScanVerdict(action: .block, rawAction: .block, score: 0.95, source: .ocr)
        // Image cadence runs first, then OCR, on the very first (priming) tick.
        let scanner = SpyScanner(responses: [warnVerdict, blockVerdict])
        let loop = ScanLoop(capture: capture, scanner: scanner, mode: .full)

        let verdicts = loop.tick(now: date(0))

        #expect(verdicts.count == 2)
        #expect(loop.lastVerdict?.action == .block)
    }

    @Test("starts nil before any tick has run", .disabled("documentation of initial state, not a behavior to assert on its own"))
    func initiallyNil() {}
}

// MARK: - FrameFingerprint

@Suite("FrameFingerprint")
struct FrameFingerprintTests {
    @Test("identical frames fingerprint the same")
    func identicalFramesMatch() {
        let a = FrameFingerprint.make(from: makeFrame(byte: 7))
        let b = FrameFingerprint.make(from: makeFrame(byte: 7))
        #expect(a == b)
    }

    @Test("different pixel content fingerprints differently")
    func differentContentDiffers() {
        let a = FrameFingerprint.make(from: makeFrame(byte: 7))
        let b = FrameFingerprint.make(from: makeFrame(byte: 8))
        #expect(a != b)
    }

    @Test("different dimensions fingerprint differently")
    func differentDimensionsDiffers() {
        let a = FrameFingerprint.make(from: makeFrame(width: 4, height: 4))
        let b = FrameFingerprint.make(from: makeFrame(width: 8, height: 8))
        #expect(a != b)
    }
}
