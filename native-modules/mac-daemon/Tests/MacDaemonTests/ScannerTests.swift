import Foundation
import Testing

@testable import MacDaemon

@Suite("NullScanner")
struct NullScannerTests {
    @Test("always allows with no located regions")
    func alwaysAllows() {
        let scanner = NullScanner()
        let frame = CapturedFrame(
            pixels: [0, 0, 0, 255], width: 1, height: 1, captured: Date(timeIntervalSince1970: 0))

        let verdict = scanner.scan(frame)

        #expect(verdict.action == .allow)
        #expect(verdict.rawAction == .allow)
        #expect(verdict.score == 0)
        #expect(verdict.regions.isEmpty)
    }

    @Test("allows even against an empty frame")
    func allowsEmptyFrame() {
        let scanner = NullScanner()
        let verdict = scanner.scan(.empty(captured: Date()))

        #expect(verdict.action == .allow)
        #expect(verdict.regions.isEmpty)
    }
}

@Suite("ScanVerdict / NormalizedRect / ImageDetection value semantics")
struct ScanVerdictValueTests {
    @Test("ScanVerdict defaults regions to empty")
    func defaultsRegionsEmpty() {
        let verdict = ScanVerdict(action: .allow, rawAction: .allow, score: 0, source: .text)
        #expect(verdict.regions.isEmpty)
    }

    @Test("ImageDetection carries a frame-relative box")
    func imageDetectionBox() {
        let box = NormalizedRect(x: 0.1, y: 0.2, width: 0.3, height: 0.4)
        let detection = ImageDetection(label: "example", confidence: 0.9, box: box)

        #expect(detection.box == box)
        #expect(detection.confidence == 0.9)
    }

    @Test("equal verdicts compare equal")
    func equatable() {
        let a = ScanVerdict(action: .block, rawAction: .block, score: 0.9, source: .image)
        let b = ScanVerdict(action: .block, rawAction: .block, score: 0.9, source: .image)
        #expect(a == b)
    }
}
