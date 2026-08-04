import Foundation
import Testing

@testable import MacDaemon

// MARK: - PixelBufferCopy.depad

@Suite("PixelBufferCopy.depad")
struct PixelBufferDepadTests {
    @Test("copies a tightly packed buffer unchanged")
    func noPadding() {
        // 2x2 BGRA, bytesPerRow == width * 4 already.
        let source: [UInt8] = [
            1, 2, 3, 4, 5, 6, 7, 8,
            9, 10, 11, 12, 13, 14, 15, 16,
        ]
        let result = PixelBufferCopy.depad(source, bytesPerRow: 8, width: 2, height: 2)
        #expect(result == source)
    }

    @Test("drops the padding CoreVideo adds past width * 4")
    func stripsPadding() {
        // 2x2 BGRA logically, but each row is padded to 12 bytes (a 64-byte-aligned buffer would
        // pad further; this is the smallest case that still exercises the bug).
        let row0: [UInt8] = [1, 2, 3, 4, 5, 6, 7, 8, 0xAA, 0xAA, 0xAA, 0xAA]
        let row1: [UInt8] = [9, 10, 11, 12, 13, 14, 15, 16, 0xAA, 0xAA, 0xAA, 0xAA]
        let source = row0 + row1

        let result = PixelBufferCopy.depad(source, bytesPerRow: 12, width: 2, height: 2)

        #expect(result == [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
    }

    @Test("a naive copy at the padded stride would shear the frame — this is the bug being avoided")
    func naiveCopyWouldShear() {
        let row0: [UInt8] = [1, 2, 3, 4, 5, 6, 7, 8, 0xAA, 0xAA, 0xAA, 0xAA]
        let row1: [UInt8] = [9, 10, 11, 12, 13, 14, 15, 16, 0xAA, 0xAA, 0xAA, 0xAA]
        let source = row0 + row1

        let naive = Array(source[0..<16])  // width * height * 4, taken verbatim from the padded buffer.
        let correct = PixelBufferCopy.depad(source, bytesPerRow: 12, width: 2, height: 2)

        #expect(naive != correct)
    }

    @Test("returns nothing for a buffer too short for its declared stride")
    func tooShortBuffer() {
        #expect(PixelBufferCopy.depad([1, 2, 3], bytesPerRow: 8, width: 2, height: 2) == [])
    }

    @Test("returns nothing when bytesPerRow is narrower than the pixel row itself")
    func strideNarrowerThanRow() {
        #expect(
            PixelBufferCopy.depad(Array(repeating: 0, count: 32), bytesPerRow: 4, width: 2, height: 2)
                == [])
    }

    @Test("returns nothing for zero width or height")
    func degenerateDimensions() {
        #expect(PixelBufferCopy.depad([1, 2, 3, 4], bytesPerRow: 4, width: 0, height: 1) == [])
        #expect(PixelBufferCopy.depad([1, 2, 3, 4], bytesPerRow: 4, width: 1, height: 0) == [])
    }
}

// MARK: - FrameCache

@Suite("FrameCache")
struct FrameCacheTests {
    private func frame(_ tag: UInt8, at date: Date) -> CapturedFrame {
        CapturedFrame(pixels: [tag, tag, tag, tag], width: 1, height: 1, captured: date)
    }

    @Test("starts with nothing retained")
    func startsEmpty() {
        #expect(FrameCache().lastComplete == nil)
    }

    @Test("retains a complete frame")
    func retainsComplete() {
        var cache = FrameCache()
        let f = frame(1, at: Date(timeIntervalSince1970: 0))

        cache.receive(f, status: .complete)

        #expect(cache.lastComplete == f)
    }

    @Test("keeps serving the last complete frame while the stream goes idle")
    func idleServesLastComplete() {
        // A motionless screen is exactly the content this project most needs to catch, and
        // .idle deliveries carry no surface — so the cache must not be cleared by them.
        var cache = FrameCache()
        let complete = frame(1, at: Date(timeIntervalSince1970: 0))
        cache.receive(complete, status: .complete)

        cache.receive(frame(9, at: Date(timeIntervalSince1970: 1)), status: .idle)
        cache.receive(frame(9, at: Date(timeIntervalSince1970: 2)), status: .blank)
        cache.receive(frame(9, at: Date(timeIntervalSince1970: 3)), status: .suspended)

        #expect(cache.lastComplete == complete)
    }

    @Test("a later complete frame replaces the earlier one")
    func replacesOnNewComplete() {
        var cache = FrameCache()
        let first = frame(1, at: Date(timeIntervalSince1970: 0))
        let second = frame(2, at: Date(timeIntervalSince1970: 1))

        cache.receive(first, status: .complete)
        cache.receive(second, status: .complete)

        #expect(cache.lastComplete == second)
    }

    @Test("can be seeded with a prior frame")
    func seededCache() {
        let seed = frame(7, at: Date(timeIntervalSince1970: 0))
        #expect(FrameCache(lastComplete: seed).lastComplete == seed)
    }
}

// MARK: - FrameAnalysis.isAllBlack

@Suite("FrameAnalysis.isAllBlack")
struct FrameAnalysisTests {
    @Test("reports an all-black frame")
    func allBlack() {
        let frame = CapturedFrame(
            pixels: Array(repeating: 0, count: 4 * 4), width: 2, height: 2,
            captured: Date(timeIntervalSince1970: 0))
        #expect(FrameAnalysis.isAllBlack(frame))
    }

    @Test("does not flag a frame with real content")
    func notBlack() {
        var pixels = Array(repeating: UInt8(0), count: 4 * 4)
        pixels[2] = 200  // one red channel byte, BGRA order.
        let frame = CapturedFrame(
            pixels: pixels, width: 2, height: 2, captured: Date(timeIntervalSince1970: 0))
        #expect(!FrameAnalysis.isAllBlack(frame))
    }

    @Test("ignores the alpha channel")
    func ignoresAlpha() {
        // Fully opaque but black BGR — still a DRM-style black frame.
        let pixels: [UInt8] = [0, 0, 0, 255, 0, 0, 0, 255]
        let frame = CapturedFrame(
            pixels: pixels, width: 2, height: 1, captured: Date(timeIntervalSince1970: 0))
        #expect(FrameAnalysis.isAllBlack(frame))
    }

    @Test("tolerates near-black noise up to the threshold")
    func toleratesNoise() {
        let pixels: [UInt8] = [3, 2, 4, 255, 1, 0, 3, 255]
        let frame = CapturedFrame(
            pixels: pixels, width: 2, height: 1, captured: Date(timeIntervalSince1970: 0))
        #expect(FrameAnalysis.isAllBlack(frame, threshold: 4))
    }

    @Test("an empty frame is not reported as black")
    func emptyFrameIsNotBlack() {
        #expect(!FrameAnalysis.isAllBlack(.empty(captured: Date(timeIntervalSince1970: 0))))
    }
}

// MARK: - CapturedFrame

@Suite("CapturedFrame.isEmpty")
struct CapturedFrameTests {
    @Test("the empty factory produces zero dimensions")
    func emptyFactory() {
        let frame = CapturedFrame.empty(captured: Date(timeIntervalSince1970: 0))
        #expect(frame.isEmpty)
        #expect(frame.width == 0)
        #expect(frame.height == 0)
        #expect(frame.pixels.isEmpty)
    }

    @Test("a real frame is not empty")
    func realFrame() {
        let frame = CapturedFrame(
            pixels: [0, 0, 0, 0], width: 1, height: 1, captured: Date(timeIntervalSince1970: 0))
        #expect(!frame.isEmpty)
    }
}

// MARK: - FakeScreenCapture

@Suite("FakeScreenCapture")
struct FakeScreenCaptureTests {
    @Test("defaults to an empty frame")
    func defaultsEmpty() {
        #expect(FakeScreenCapture().currentFrame().isEmpty)
    }

    @Test("serves whatever frame was set")
    func servesSetFrame() {
        let capture = FakeScreenCapture()
        let frame = CapturedFrame(
            pixels: [1, 2, 3, 4], width: 1, height: 1, captured: Date(timeIntervalSince1970: 0))

        capture.set(frame)

        #expect(capture.currentFrame() == frame)
    }
}
