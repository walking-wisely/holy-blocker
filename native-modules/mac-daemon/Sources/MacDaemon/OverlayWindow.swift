import AppKit

// Module 10's AppKit half (see docs/components/mac-daemon/plan.md, "Overlay"). Everything here is
// mechanical mapping onto real AppKit types and live `NSWindow`s; every decision worth testing
// lives in `Overlay.swift`, which stays AppKit-free on purpose. That split is why this file has no
// unit tests — the same exemption `SCShareableContentCapture` and `SystemAXProbe` take.

// MARK: - Process lifecycle

/// The GUI-session process setup an overlay needs before any `NSWindow` will appear.
public enum AppLifecycle {
    /// Brings up the shared `NSApplication` and marks the process an accessory: it draws windows
    /// but has no Dock icon and no menu bar, which is what a background daemon that occasionally
    /// covers the screen wants.
    ///
    /// This must run before the first `OverlayController.apply` — an `NSWindow` created in a
    /// process with no `NSApplication` never appears, and does so without erroring. The caller is
    /// then responsible for running the main run loop (`NSApplication.run()`); this function
    /// deliberately does not, so the caller keeps control of when the thread is surrendered.
    ///
    /// See https://developer.apple.com/documentation/appkit/nsapplication/activationpolicy.
    @MainActor
    public static func configureAccessoryApp() {
        NSApplication.shared.setActivationPolicy(.accessory)
    }
}

// MARK: - Reading the real screen configuration

extension ScreenConfiguration {
    /// The live display list, in `NSScreen.screens` order (index 0 is the screen with the menu
    /// bar).
    @MainActor
    public static func current() -> [ScreenConfiguration] {
        NSScreen.screens.compactMap(ScreenConfiguration.init(screen:))
    }

    /// Reads the stable identity and full frame `OverlayPlan` plans against off a real `NSScreen`.
    ///
    /// The identity is the display's `CGDirectDisplayID`, which AppKit exposes only through the
    /// untyped device description under the documented `"NSScreenNumber"` key — there is no
    /// `NSDeviceDescriptionKey` constant for it. See
    /// https://developer.apple.com/documentation/appkit/nsscreen/devicedescription.
    ///
    /// Returns `nil` if the key is absent, which drops that screen from the plan rather than
    /// inventing an identity that would collide with another screen's on the next reconcile.
    @MainActor
    public init?(screen: NSScreen) {
        guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else {
            return nil
        }
        // `frame`, not `visibleFrame`: whole-surface cover includes the menu bar and Dock strips,
        // which the `.screenSaver` window level puts the overlay above anyway.
        self.init(id: number.intValue, frame: screen.frame)
    }
}

// MARK: - AppKit mappings

extension OverlayWindowLevel {
    /// See https://developer.apple.com/documentation/appkit/nswindow/level.
    var appKitLevel: NSWindow.Level {
        switch self {
        case .screenSaver: return .screenSaver
        }
    }
}

extension OverlayCollectionBehavior {
    /// See https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior.
    ///
    /// `.canJoinAllSpaces` and `.fullScreenAuxiliary` are both required for the overlay to appear
    /// over native-fullscreen content, and with only one of them it fails by simply not showing —
    /// hence `OverlayCollectionBehavior.requiredForOverlay` carrying both as a unit.
    var appKitBehavior: NSWindow.CollectionBehavior {
        var behavior: NSWindow.CollectionBehavior = []
        if contains(.canJoinAllSpaces) { behavior.insert(.canJoinAllSpaces) }
        if contains(.fullScreenAuxiliary) { behavior.insert(.fullScreenAuxiliary) }
        return behavior
    }
}

// MARK: - The live window set

/// Owns one borderless `NSWindow` per screen and keeps that set matching the current
/// `OverlayIntent` and display configuration.
///
/// Main-actor isolated because every `NSWindow` and `NSScreen` access here must be — the scan loop
/// that calls `apply(intent:)` runs on the main run loop for exactly this reason.
@MainActor
public final class OverlayController {
    private var windows: [Int: NSWindow] = [:]
    private var intent: OverlayIntent = .none
    private var screenObserver: NSObjectProtocol?

    public init() {}

    /// Whether an overlay is currently on screen. Reported by the agent's status output; also the
    /// cheap assertion a live verification run checks.
    public var isShowing: Bool { !windows.isEmpty }

    /// Starts watching for display connect/disconnect/rearrange and reconciles once.
    ///
    /// Screens can change while an overlay is up, and a window whose screen went away is a window
    /// covering nothing — or, worse, a screen with no window is uncovered content. The
    /// notification is posted on the main thread, which is where `reconcile()` must run.
    /// See https://developer.apple.com/documentation/appkit/nsapplication/didchangescreenparametersnotification.
    public func start() {
        guard screenObserver == nil else { return }
        screenObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            // `queue: .main` guarantees the main thread but not main-actor isolation as far as the
            // compiler is concerned; this is the documented way to bridge that gap.
            MainActor.assumeIsolated { self?.reconcile() }
        }
        reconcile()
    }

    /// Tears every overlay down and stops observing. Idempotent.
    public func stop() {
        if let observer = screenObserver {
            NotificationCenter.default.removeObserver(observer)
            screenObserver = nil
        }
        apply(intent: .none)
    }

    /// Sets the intent and reconciles the window set to match it. Cheap to call repeatedly with an
    /// unchanged intent — an already-correct window is reconfigured in place, never rebuilt.
    public func apply(intent: OverlayIntent) {
        self.intent = intent
        reconcile()
    }

    private func reconcile() {
        let planned = OverlayPlan.plan(intent: intent, screens: ScreenConfiguration.current())
        let diff = OverlayReconciliation.diff(existing: Array(windows.keys), planned: planned)

        for screenID in diff.close {
            windows.removeValue(forKey: screenID)?.orderOut(nil)
        }
        for placement in diff.update {
            if let window = windows[placement.screenID] { configure(window, for: placement) }
        }
        for placement in diff.create {
            let window = makeWindow(for: placement)
            windows[placement.screenID] = window
            window.orderFrontRegardless()
        }
    }

    private func makeWindow(for placement: OverlayPlacement) -> NSWindow {
        let window = NSWindow(
            contentRect: placement.frame,
            styleMask: .borderless,
            backing: .buffered,
            defer: false)
        // Teardown is `orderOut` plus dropping the reference, so the window's lifetime is ARC's;
        // leaving AppKit's legacy auto-release behaviour on would over-release it.
        window.isReleasedWhenClosed = false
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = false
        window.contentView = OverlayContentView(frame: NSRect(origin: .zero, size: placement.frame.size))
        configure(window, for: placement)
        return window
    }

    private func configure(_ window: NSWindow, for placement: OverlayPlacement) {
        // `setFrame` rather than recreating: a display rearrangement keeps the screen's identity
        // and only moves it.
        window.setFrame(placement.frame, display: true)
        window.level = placement.windowLevel.appKitLevel
        window.collectionBehavior = placement.collectionBehavior.appKitBehavior
        window.ignoresMouseEvents = placement.ignoresMouseEvents
    }
}

// MARK: - What the overlay draws

/// The overlay's backdrop: a live blur of whatever is underneath, plus the dark tint the proxy's
/// injected web overlay uses, so the native and web interstitials read as the same product
/// surface (see docs/product/flows/warn-interstitial.md — `backdrop-filter: blur(24px)` over
/// `rgba(0,0,0,0.55)`).
///
/// `NSVisualEffectView` blurs the *composited desktop behind the window*, so no captured frame is
/// read, copied or retained to draw this. That is a stronger position than the plan's "the frame
/// is a texture that never leaves the device": here there is no frame at all.
///
/// The verse and the two-button deliberate choice the flow doc describes are **not** built here.
/// They need the verse table (docs/decisions/verse-selection.md) and a dismissal path back to the
/// scan loop, neither of which exists on this platform yet; this session's scope is the window,
/// its level, and its Spaces behaviour. The label below is a placeholder for that card.
final class OverlayContentView: NSView {
    private let blur = NSVisualEffectView()
    private let tint = NSView()
    private let label = NSTextField(labelWithString: "Holy Blocker")

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        autoresizingMask = [.width, .height]

        blur.blendingMode = .behindWindow
        // `.fullScreenUI` is the heaviest stock blur and the closest match to a 24px backdrop
        // filter; `.active` keeps it blurring while the overlay's own window is not key.
        blur.material = .fullScreenUI
        blur.state = .active
        blur.autoresizingMask = [.width, .height]
        addSubview(blur)

        tint.wantsLayer = true
        tint.layer?.backgroundColor = NSColor(calibratedWhite: 0, alpha: 0.55).cgColor
        tint.autoresizingMask = [.width, .height]
        addSubview(tint)

        label.font = .systemFont(ofSize: 28, weight: .medium)
        label.textColor = .white
        label.alignment = .center
        addSubview(label)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not loaded from a nib") }

    override func layout() {
        super.layout()
        blur.frame = bounds
        tint.frame = bounds
        label.sizeToFit()
        label.frame.origin = CGPoint(
            x: bounds.midX - label.frame.width / 2,
            y: bounds.midY - label.frame.height / 2)
    }
}
