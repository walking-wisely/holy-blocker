import Foundation

// Mirrors the Windows daemon's `scanner` module and keeps the same vocabulary so the two daemons
// stay comparable — see docs/components/mac-daemon/plan.md, module 9, and
// docs/architecture/edge-daemons.md for the cadence vocabulary this scheduler implements.

/// The action a scan verdict resolves to, after `ProtectionMode` has been applied.
public enum ScanAction: Equatable, Sendable {
    case allow
    case warn
    case block
}

/// What produced a `ScanVerdict`. `ScanLoop` tags verdicts by which cadence triggered the scan
/// that returned them (image classifier vs. OCR/text policy).
public enum ScanSource: Equatable, Sendable {
    case ocr
    case image
    case text
}

/// The user-facing protection level. Applied by `ScanLoop` to turn a `Scanner`'s `rawAction` into
/// the effective `action` — see [protection-modes.md](../../decisions/protection-modes.md).
public enum ProtectionMode: Equatable, Sendable {
    case full
    case warn
    case off
}

/// A box relative to the classified frame, not pixels — see
/// docs/architecture/content-classification.md's "Image Localization" section. Frame resolution
/// varies by display and Retina scale factor; a normalized box stays valid across any resize or
/// crop later in the pipeline. Callers that need screen or window coordinates convert using the
/// frame's known bounds at capture time.
public struct NormalizedRect: Equatable, Sendable {
    public let x: Double
    public let y: Double
    public let width: Double
    public let height: Double

    public init(x: Double, y: Double, width: Double, height: Double) {
        self.x = x
        self.y = y
        self.width = width
        self.height = height
    }
}

/// One located image detection. See content-classification.md — nothing populates this beyond an
/// empty list yet; a real detector model or the coarse-grid fallback described there is future
/// work for the `Scanner` implementation that eventually replaces `NullScanner`.
public struct ImageDetection: Equatable, Sendable {
    public let label: String
    public let confidence: Double
    public let box: NormalizedRect

    public init(label: String, confidence: Double, box: NormalizedRect) {
        self.label = label
        self.confidence = confidence
        self.box = box
    }
}

/// The result of scanning one frame.
public struct ScanVerdict: Equatable, Sendable {
    /// The effective action, after `ScanLoop` has applied `ProtectionMode`.
    public let action: ScanAction
    /// The action as returned by the scanner, before any mode downgrade.
    public let rawAction: ScanAction
    /// 0.0...1.0.
    public let score: Double
    public let source: ScanSource
    /// Located image detections. Empty for text-sourced verdicts and for image verdicts where
    /// nothing localized. An empty list on a non-allow verdict means "cover the whole surface",
    /// never "nothing to do" — see content-classification.md's "Image Localization" section.
    public let regions: [ImageDetection]

    public init(
        action: ScanAction, rawAction: ScanAction, score: Double, source: ScanSource,
        regions: [ImageDetection] = []
    ) {
        self.action = action
        self.rawAction = rawAction
        self.score = score
        self.source = source
        self.regions = regions
    }
}

/// One scan source (image classifier, OCR + text policy, ...). `ScanLoop` is the only caller;
/// implementations do the actual classification work.
public protocol Scanner {
    func scan(_ frame: CapturedFrame) -> ScanVerdict
}

/// Always allows, with no located regions. The only `Scanner` implementation until a real
/// classifier is wired in — see the plan's module 9 "open" notes: neither the mode→action policy
/// engine nor image localization is implemented here.
public struct NullScanner: Scanner {
    public init() {}

    public func scan(_ frame: CapturedFrame) -> ScanVerdict {
        ScanVerdict(action: .allow, rawAction: .allow, score: 0, source: .image, regions: [])
    }
}
