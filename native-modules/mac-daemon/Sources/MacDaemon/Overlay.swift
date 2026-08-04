import CoreGraphics

/// A minimal, AppKit-free stand-in for `NSScreen`: one physical display's identity and frame.
///
/// Deliberately built on `CoreGraphics.CGRect` rather than importing AppKit — this file is the
/// pure part of module 10 (see docs/components/mac-daemon/plan.md, "Overlay") and must stay
/// testable without an `NSApplication` run loop. The AppKit-wiring task that builds real
/// `NSWindow`s reads `id`/`frame` off the real `NSScreen` list to build this value.
public struct ScreenConfiguration: Equatable, Sendable {
    /// Stable identity for one screen across a single planning call. A later AppKit-wiring task
    /// can source this from `NSScreen.deviceDescription[.screenNumber]` (the display's
    /// `CGDirectDisplayID`) — this file does not care what the identifier means, only that it is
    /// stable enough to match a placement back to the screen it was planned for.
    public let id: Int
    /// The screen's full frame, in the same coordinate space `NSScreen.frame` uses. Whole-surface
    /// cover means the overlay's frame equals this exactly — see `OverlayPlan.plan`.
    public let frame: CGRect

    public init(id: Int, frame: CGRect) {
        self.id = id
        self.frame = frame
    }
}

/// The three visual states from the plan's overlay table, plus "no overlay at all."
///
/// This is intentionally *not* `ScanAction` or `ScanVerdict` — `Overlay.swift` does not depend on
/// the concurrently-developed `Scanner.swift` types. `OverlayIntent.intent(forAction:)` below is
/// the seam a later merge can use to map a real `ScanVerdict.action` onto this enum without this
/// file needing to import `Scanner.swift`.
///
/// Region-level (partial-surface) cover is explicitly deferred per the plan's amendment about
/// `ScanVerdict.regions` — this enum only carries whole-screen intents today, but `.interstitial`
/// takes an associated value rather than being a bare case specifically so a future region-aware
/// case (e.g. `.interstitial(swallowsMouseEvents:regions:)` or a new case entirely) can be added
/// without an incompatible redesign of every call site.
public enum OverlayIntent: Equatable, Sendable {
    /// No overlay should exist. Maps from an `.allow` verdict.
    case none
    /// A hint that does not interrupt — `ignoresMouseEvents = true` on the resulting window.
    /// Maps from the pre-blur / passive state described in the plan's table.
    case passive
    /// A full interrupt that must swallow mouse events, per the plan's table: both the Warn
    /// interstitial and the Block cover use this case. `swallowsMouseEvents` is carried
    /// explicitly (rather than assumed `true` for every interstitial) so the pure function below
    /// faithfully reflects whatever it is told instead of silently overriding a caller.
    case interstitial(swallowsMouseEvents: Bool)
}

extension OverlayIntent {
    /// A minimal, local stand-in for `ScanAction` (see docs/components/mac-daemon/plan.md, module
    /// 9) so this mapping is expressible without importing the concurrently-developed
    /// `Scanner.swift`. Named `OverlayVerdictAction` rather than `ScanAction` specifically to
    /// avoid a redeclaration collision once `Scanner.swift` lands with the real type — a later
    /// integration pass can either delete this alias in favor of `ScanAction` or keep
    /// `intent(forAction:)` as a thin adapter between the two.
    public enum OverlayVerdictAction: Equatable, Sendable {
        case allow, warn, block
    }

    /// Maps an effective scan action to the overlay intent required for it, per the plan's table.
    /// Warn and Block share the mouse-swallowing interstitial requirement; only their visual
    /// treatment differs, which is out of scope for this pure placement function.
    public static func intent(forAction action: OverlayVerdictAction) -> OverlayIntent {
        switch action {
        case .allow: return .none
        case .warn: return .interstitial(swallowsMouseEvents: true)
        case .block: return .interstitial(swallowsMouseEvents: true)
        }
    }
}

/// A local, AppKit-free stand-in for the handful of `NSWindow.Level` constants this module needs.
/// See https://developer.apple.com/documentation/appkit/nswindow/level.
public enum OverlayWindowLevel: Equatable, Sendable {
    /// Above normal windows and, per the plan, clears the menu bar and Dock — required so the
    /// overlay covers native-fullscreen content. Maps to `NSWindow.Level.screenSaver`.
    case screenSaver
}

/// A local, AppKit-free stand-in for the `NSWindow.CollectionBehavior` option set. See
/// https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior.
public struct OverlayCollectionBehavior: OptionSet, Equatable, Sendable {
    public let rawValue: Int

    public init(rawValue: Int) {
        self.rawValue = rawValue
    }

    /// The overlay follows the user across Spaces instead of being pinned to whichever Space was
    /// active when it was created. Maps to `NSWindow.CollectionBehavior.canJoinAllSpaces`.
    public static let canJoinAllSpaces = OverlayCollectionBehavior(rawValue: 1 << 0)
    /// Required, alongside `.canJoinAllSpaces`, for the overlay to appear over native-fullscreen
    /// video — with only one of the two the overlay silently fails to show. Maps to
    /// `NSWindow.CollectionBehavior.fullScreenAuxiliary`.
    public static let fullScreenAuxiliary = OverlayCollectionBehavior(rawValue: 1 << 1)

    /// What every planned overlay in this file requires, per the plan's Requirements list.
    public static let requiredForOverlay: OverlayCollectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
}

/// One planned overlay window, scoped to a single screen. `OverlayPlan.plan` returns a set of
/// these; a later AppKit-wiring task reads every field here to construct the real `NSWindow` —
/// this struct is the full configuration contract between the pure planning logic and that task.
public struct OverlayPlacement: Equatable, Sendable {
    /// The `ScreenConfiguration.id` this placement covers. One placement per screen, per the
    /// plan's "One overlay per NSScreen" requirement.
    public let screenID: Int
    /// Whole-surface cover: always equal to the source screen's full frame. Region-level
    /// (partial-surface) cover is explicitly deferred — see `OverlayIntent`'s doc comment.
    public let frame: CGRect
    /// `true` only for the passive/pre-blur state; `false` for both interstitial states, which
    /// must swallow mouse events per the plan's table.
    public let ignoresMouseEvents: Bool
    public let windowLevel: OverlayWindowLevel
    public let collectionBehavior: OverlayCollectionBehavior

    public init(
        screenID: Int,
        frame: CGRect,
        ignoresMouseEvents: Bool,
        windowLevel: OverlayWindowLevel,
        collectionBehavior: OverlayCollectionBehavior
    ) {
        self.screenID = screenID
        self.frame = frame
        self.ignoresMouseEvents = ignoresMouseEvents
        self.windowLevel = windowLevel
        self.collectionBehavior = collectionBehavior
    }
}

/// Module 10's pure part: *which* overlays should exist for a given intent and screen
/// configuration. AppKit-free and time-free, so it is fully unit-testable without an
/// `NSApplication` run loop — see docs/components/mac-daemon/plan.md, module 10.
public enum OverlayPlan {
    /// Plans one `OverlayPlacement` per screen when `intent` is not `.none`, each covering that
    /// screen's full frame (whole-surface cover only — region-level cover is deferred, see
    /// `OverlayIntent`). Returns an empty array for `.none` or an empty screen list.
    public static func plan(intent: OverlayIntent, screens: [ScreenConfiguration]) -> [OverlayPlacement] {
        guard intent != .none else { return [] }

        let ignoresMouseEvents: Bool
        switch intent {
        case .none:
            // Unreachable — guarded above — but exhaustive so a future case forces a decision here.
            ignoresMouseEvents = false
        case .passive:
            ignoresMouseEvents = true
        case .interstitial(let swallowsMouseEvents):
            ignoresMouseEvents = !swallowsMouseEvents
        }

        return screens.map { screen in
            OverlayPlacement(
                screenID: screen.id,
                frame: screen.frame,
                ignoresMouseEvents: ignoresMouseEvents,
                windowLevel: .screenSaver,
                collectionBehavior: .requiredForOverlay)
        }
    }
}
