import AppKit

// Found in the first live e2e pass: the overlay alone does not cover content, it covers *pixels*.
// Click the desktop and every window unfocuses, the frontmost-window scan finds nothing, and the
// cover tears down over content that never moved. Swipe four fingers up and Mission Control
// composites live window previews above a `.screenSaver` window. Both are the same lesson — the
// window server can always re-arrange what is underneath a picture drawn on top of it.
//
// So a block does two things: it draws the interstitial *and* removes the offending application
// from the screen. Hiding is chosen over closing deliberately — closing a window discards unsaved
// work, and a blocker that loses a half-written document gets uninstalled. A hidden application is
// gone from the screen, gone from Mission Control's previews, and one click away in the Dock when
// the user is entitled to it back.

// MARK: - The decision

public enum SuppressionCommand: Equatable, Sendable {
    case none
    case hide(bundleIdentifier: String)
}

public struct SuppressionPolicy: Sendable {
    /// Applications that must never be hidden whatever they are showing.
    ///
    /// Ourselves, because hiding this process takes the overlay off screen — the exact opposite of
    /// the intent. Finder, because hiding it takes the desktop with it. The Dock and
    /// SystemUIServer for the same reason one level down: they are the shell, not content.
    public static let defaultProtected: Set<String> = [
        "com.holyblocker.daemon",
        "com.apple.finder",
        "com.apple.dock",
        "com.apple.systemuiserver",
        "com.apple.loginwindow",
    ]

    /// How long to leave an application alone after asking it to hide. The scan cadence is ~1s and
    /// a hide can legitimately not take effect; without this, an application that refuses would be
    /// asked once a second forever.
    public static let defaultCooldown: TimeInterval = 5

    public var protectedBundleIdentifiers: Set<String>
    public var cooldown: TimeInterval

    public init(
        protectedBundleIdentifiers: Set<String> = SuppressionPolicy.defaultProtected,
        cooldown: TimeInterval = SuppressionPolicy.defaultCooldown
    ) {
        self.protectedBundleIdentifiers = protectedBundleIdentifiers
        self.cooldown = cooldown
    }
}

/// Pure: what to do about `target` given a verdict and what was already done to it.
public enum SuppressionDecision {
    public static func command(
        action: ScanAction, target: String?, policy: SuppressionPolicy, lastHiddenAt: Date?,
        now: Date
    ) -> SuppressionCommand {
        // Only a block. A warn is an interstitial the user is meant to be able to think past, and
        // taking their window away is not a weaker response than showing them a question.
        guard action == .block, let target else { return .none }
        guard !policy.protectedBundleIdentifiers.contains(target) else { return .none }
        if let lastHiddenAt, now.timeIntervalSince(lastHiddenAt) < policy.cooldown { return .none }
        return .hide(bundleIdentifier: target)
    }
}

// MARK: - The edge

/// Removing an application from the screen. Behind a protocol for the same reason as
/// `CommandRunner` and `ScreenCapturing`: the decision above is testable, this is not.
public protocol ApplicationHiding: Sendable {
    /// Returns whether an application with that identifier was found and asked to hide.
    func hide(bundleIdentifier: String) -> Bool
}

/// The real one. `NSRunningApplication.hide()` needs no TCC grant of any kind — it is ordinary
/// application-level window management, not accessibility control of another process.
/// See https://developer.apple.com/documentation/appkit/nsrunningapplication/hide().
public struct WorkspaceApplicationHider: ApplicationHiding {
    public init() {}

    public func hide(bundleIdentifier: String) -> Bool {
        let matches = NSWorkspace.shared.runningApplications.filter {
            $0.bundleIdentifier == bundleIdentifier
        }
        guard !matches.isEmpty else { return false }
        // `hide()` is per-`NSRunningApplication`; an application can legitimately have more than
        // one instance, and leaving the second one showing would defeat the whole point.
        return matches.map { $0.hide() }.contains(true)
    }
}

public final class FakeApplicationHider: ApplicationHiding, @unchecked Sendable {
    private let lock = NSLock()
    private var _hidden: [String] = []
    private var _succeeds = true

    public init() {}

    /// Every hide asked for, in order — including ones that returned false.
    public var hidden: [String] {
        lock.lock()
        defer { lock.unlock() }
        return _hidden
    }

    public var succeeds: Bool {
        get {
            lock.lock()
            defer { lock.unlock() }
            return _succeeds
        }
        set {
            lock.lock()
            defer { lock.unlock() }
            _succeeds = newValue
        }
    }

    public func hide(bundleIdentifier: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        _hidden.append(bundleIdentifier)
        return _succeeds
    }
}

// MARK: - The stateful wrapper

/// Applies `SuppressionDecision` and remembers what it did, so the cooldown means something.
public final class WindowSuppressor {
    private let policy: SuppressionPolicy
    private let hider: ApplicationHiding
    private let now: () -> Date

    private var lastHiddenAt: [String: Date] = [:]

    public init(
        policy: SuppressionPolicy = SuppressionPolicy(),
        hider: ApplicationHiding = WorkspaceApplicationHider(),
        now: @escaping () -> Date = { Date() }
    ) {
        self.policy = policy
        self.hider = hider
        self.now = now
    }

    @discardableResult
    public func apply(action: ScanAction, target: String?) -> SuppressionCommand {
        let instant = now()
        let command = SuppressionDecision.command(
            action: action, target: target, policy: policy,
            lastHiddenAt: target.flatMap { lastHiddenAt[$0] }, now: instant)

        guard case .hide(let bundleIdentifier) = command else { return command }
        // Stamped only on success: a refusal that started the cooldown would buy the application
        // five quiet seconds on screen, which is exactly the window this exists to close.
        if hider.hide(bundleIdentifier: bundleIdentifier) {
            lastHiddenAt[bundleIdentifier] = instant
        }
        return command
    }
}
