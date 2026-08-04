import CoreVideo
import Foundation
import ScreenCaptureKit

// `CGWindowListCreateImage` is obsoleted as of macOS 15 and must not be used here, including as a
// fallback for a failed SCStream setup — see the deprecation notice linked below.

// MARK: - The shared frame type

/// The same logical frame the Windows `capture` module produces, so the downstream scan interface
/// is shared across platforms.
public struct CapturedFrame: Equatable, Sendable {
    /// BGRA, row-major, tightly packed — `width * height * 4` bytes, no row padding.
    public let pixels: [UInt8]
    public let width: Int
    public let height: Int
    public let captured: Date

    public init(pixels: [UInt8], width: Int, height: Int, captured: Date) {
        self.pixels = pixels
        self.width = width
        self.height = height
        self.captured = captured
    }

    /// Capture unavailable. Callers check this rather than the module throwing — the same
    /// fail-open contract as the Windows module.
    public static func empty(captured: Date) -> CapturedFrame {
        CapturedFrame(pixels: [], width: 0, height: 0, captured: captured)
    }

    public var isEmpty: Bool { width == 0 || height == 0 }
}

// MARK: - Pure logic

/// Strips CoreVideo's row padding from a raw pixel buffer copy.
///
/// `CVPixelBufferGetBytesPerRow` is commonly aligned past `width * 4` (64-byte alignment is
/// typical). Copying `bytesPerRow * height` bytes and treating the result as a tightly packed
/// `width * height` image produces a frame that shears further out of alignment on every row while
/// still decoding as *something* and still scoring on the classifier — the bug is silent, not a
/// crash. See Apple's `CVPixelBuffer` reference, `CVPixelBufferGetBytesPerRow`.
public enum PixelBufferCopy {
    /// BGRA is 4 bytes per pixel; `bytesPerRow` may still exceed this per-row.
    private static let bytesPerPixel = 4

    /// Copies `source` (one `bytesPerRow`-strided buffer of `height` rows) into a tightly packed
    /// `width * height * 4` buffer. Returns `[]` for dimensions or a stride that cannot be valid,
    /// so a caller only has to check for emptiness rather than catch an error.
    public static func depad(_ source: [UInt8], bytesPerRow: Int, width: Int, height: Int) -> [UInt8]
    {
        let rowBytes = width * bytesPerPixel
        guard width > 0, height > 0, bytesPerRow >= rowBytes,
            source.count >= bytesPerRow * height
        else { return [] }

        var result = [UInt8]()
        result.reserveCapacity(rowBytes * height)
        for row in 0..<height {
            let start = row * bytesPerRow
            result.append(contentsOf: source[start..<(start + rowBytes)])
        }
        return result
    }
}

/// Mirrors `SCStreamFrameInfo.status`, kept as our own type so the retention logic below is
/// testable without linking ScreenCaptureKit.
public enum FrameDeliveryStatus: Equatable, Sendable {
    case complete
    case idle
    case blank
    case suspended
    case stopped
}

/// Retains the last `.complete` frame across deliveries that carry no new surface.
///
/// `SCStream` is change-driven: a motionless screen emits `.idle` frames with a nil surface rather
/// than repeating the last one. A still image is precisely the content this project most needs to
/// catch, so the cache must keep serving what it last received on `.complete` rather than going
/// empty the moment the screen stops changing. See `SCStreamFrameInfo.status`.
public struct FrameCache: Sendable {
    public private(set) var lastComplete: CapturedFrame?

    public init(lastComplete: CapturedFrame? = nil) {
        self.lastComplete = lastComplete
    }

    /// Feed one delivery. Only `.complete` updates what is retained.
    public mutating func receive(_ frame: CapturedFrame, status: FrameDeliveryStatus) {
        guard status == .complete else { return }
        lastComplete = frame
    }
}

/// Analysis that turns a frame's pixels into a signal, rather than just a decode target.
public enum FrameAnalysis {
    /// True when every sampled channel (B, G, R — alpha ignored) is at or below `threshold`.
    ///
    /// DRM/HDCP-protected surfaces (Netflix, Apple TV+, and similar) are excluded from
    /// ScreenCaptureKit by design and deliver solid black. That is a coverage limit, not a bug to
    /// fix — but an all-black frame from a window that is actively playing video is a signal in
    /// itself, worth surfacing rather than silently discarding.
    public static func isAllBlack(_ frame: CapturedFrame, threshold: UInt8 = 4) -> Bool {
        guard !frame.isEmpty, frame.pixels.count >= 4 else { return false }

        var index = 0
        while index + 3 < frame.pixels.count {
            if frame.pixels[index] > threshold || frame.pixels[index + 1] > threshold
                || frame.pixels[index + 2] > threshold
            {
                return false
            }
            index += 4
        }
        return true
    }
}

// MARK: - The capture edge

/// One capture source, behind a protocol so everything above it is testable without
/// ScreenCaptureKit — mirroring `CommandRunner` and `PermissionProbing`.
public protocol ScreenCapturing: Sendable {
    /// The most recent frame available. Empty when capture has not started or is unavailable —
    /// callers check `isEmpty` rather than the call throwing. Never prompts.
    func currentFrame() -> CapturedFrame
}

/// Test double for `ScreenCapturing`, in the library so integration harnesses can use it too.
public final class FakeScreenCapture: ScreenCapturing, @unchecked Sendable {
    private let lock = NSLock()
    private var frame: CapturedFrame

    public init(frame: CapturedFrame = .empty(captured: Date(timeIntervalSince1970: 0))) {
        self.frame = frame
    }

    public func set(_ frame: CapturedFrame) {
        lock.lock()
        defer { lock.unlock() }
        self.frame = frame
    }

    public func currentFrame() -> CapturedFrame {
        lock.lock()
        defer { lock.unlock() }
        return frame
    }
}

/// Real capture over `ScreenCaptureKit`. Fails open on every path — no shareable content, no
/// display, a stream error — by leaving `currentFrame()` at empty rather than throwing.
public final class SCShareableContentCapture: NSObject, ScreenCapturing, SCStreamOutput,
    @unchecked Sendable
{
    private static let captureQueueLabel = "com.holyblocker.macdaemon.screencapture"

    private let lock = NSLock()
    private var cache = FrameCache()
    private var stream: SCStream?
    private let captureQueue = DispatchQueue(label: SCShareableContentCapture.captureQueueLabel)

    /// Excluded from the filter so the daemon's own overlay is never captured and re-scanned — a
    /// feedback loop that would otherwise re-trigger on the blurred backdrop it just drew.
    private let ownBundleIdentifier: String?

    public init(excludingBundleIdentifier: String? = Bundle.main.bundleIdentifier) {
        self.ownBundleIdentifier = excludingBundleIdentifier
        super.init()
    }

    /// Enumerates shareable content and starts the stream against the first display. Frames after
    /// this call are pushed to `currentFrame()` from `stream(_:didOutputSampleBuffer:of:)`.
    public func start() async throws {
        let content = try await SCShareableContent.excludingDesktopWindows(
            false, onScreenWindowsOnly: true)
        guard let display = content.displays.first else { return }

        let excludedApplications = content.applications.filter {
            $0.bundleIdentifier == ownBundleIdentifier
        }
        let filter = SCContentFilter(
            display: display, excludingApplications: excludedApplications, exceptingWindows: [])

        let configuration = SCStreamConfiguration()
        // Pixel dimensions, not points: SCStreamConfiguration defaults to point dimensions, which
        // on a Retina display is half the real resolution and yields a blurry downscale. A wrong
        // aspect ratio here also silently changes what packages/image-sandbox's
        // resize-then-centre-crop sees — the same class of error its parity fixture caught on the
        // Rust side. See SCStreamConfiguration docs.
        configuration.width = display.width
        configuration.height = display.height
        configuration.scalesToFit = false

        let stream = SCStream(filter: filter, configuration: configuration, delegate: nil)
        try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: captureQueue)
        try await stream.startCapture()
        self.stream = stream
    }

    public func stop() async throws {
        try await stream?.stopCapture()
        stream = nil
    }

    public func currentFrame() -> CapturedFrame {
        lock.lock()
        defer { lock.unlock() }
        return cache.lastComplete ?? CapturedFrame.empty(captured: Date())
    }

    public func stream(
        _ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard type == .screen, let status = Self.frameStatus(of: sampleBuffer) else { return }

        guard status == .complete else {
            // Nothing new to retain — FrameCache.receive drops non-complete deliveries itself, but
            // skip the CVPixelBuffer work entirely since there is nothing there to read.
            return
        }
        guard let imageBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }

        CVPixelBufferLockBaseAddress(imageBuffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(imageBuffer, .readOnly) }

        let width = CVPixelBufferGetWidth(imageBuffer)
        let height = CVPixelBufferGetHeight(imageBuffer)
        let bytesPerRow = CVPixelBufferGetBytesPerRow(imageBuffer)
        guard let base = CVPixelBufferGetBaseAddress(imageBuffer) else { return }

        let raw = [UInt8](
            UnsafeBufferPointer(
                start: base.assumingMemoryBound(to: UInt8.self), count: bytesPerRow * height))
        let tight = PixelBufferCopy.depad(raw, bytesPerRow: bytesPerRow, width: width, height: height)
        guard !tight.isEmpty else { return }

        let frame = CapturedFrame(pixels: tight, width: width, height: height, captured: Date())
        lock.lock()
        cache.receive(frame, status: .complete)
        lock.unlock()
    }

    /// Reads `SCStreamFrameInfo.status` out of the sample buffer's attachments, mapped to our own
    /// `FrameDeliveryStatus` so the retention logic above stays free of ScreenCaptureKit types.
    private static func frameStatus(of sampleBuffer: CMSampleBuffer) -> FrameDeliveryStatus? {
        guard
            let attachments = CMSampleBufferGetSampleAttachmentsArray(
                sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
            let statusRaw = attachments.first?[.status] as? Int,
            let status = SCFrameStatus(rawValue: statusRaw)
        else { return nil }

        switch status {
        case .complete: return .complete
        case .idle: return .idle
        case .blank: return .blank
        case .suspended: return .suspended
        case .stopped: return .stopped
        case .started: return .idle
        @unknown default: return .idle
        }
    }
}
