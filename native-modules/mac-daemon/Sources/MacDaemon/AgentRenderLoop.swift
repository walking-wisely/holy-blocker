import Foundation

// Session 5 of the first live e2e pass (see docs/components/mac-daemon/plan.md, "The first live
// e2e pass — text-only, split into six sessions"). `Overlay.swift` was deliberately built without
// importing `Scanner.swift` so modules 9 and 10 could land as independent, non-conflicting
// sessions — see `OverlayIntent.OverlayVerdictAction`'s doc comment, which named this exact
// integration pass as the place either to delete that stand-in or keep it as a thin adapter. Both
// types now live in the same target, so the bridge goes here rather than into either file, and
// neither one gains a dependency on the other.

/// Maps a scan verdict's *effective* action onto the overlay intent it requires. `nil` — no tick
/// has produced a verdict yet, e.g. before the render loop's first scan — is treated the same as
/// an explicit `.allow`: no overlay.
///
/// Reads `verdict.action`, never `verdict.rawAction`. `ScanLoop.applyProtectionMode` already
/// resolved `ProtectionMode` into `action` before this daemon sees it; re-deriving the overlay from
/// `rawAction` would silently undo a warn-mode downgrade the scan loop already made.
public func overlayIntent(forVerdict verdict: ScanVerdict?) -> OverlayIntent {
    guard let action = verdict?.action else { return .none }

    let mapped: OverlayIntent.OverlayVerdictAction
    switch action {
    case .allow: mapped = .allow
    case .warn: mapped = .warn
    case .block: mapped = .block
    }
    return OverlayIntent.intent(forAction: mapped)
}
