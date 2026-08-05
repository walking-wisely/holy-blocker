import AppKit
import Foundation
import MacDaemon

// Line-buffer stdout. When `run` has its output captured to a log file, the default block
// buffering holds every progress line until the process exits — precisely when a supervisor's
// log stops being useful.
setvbuf(stdout, nil, _IOLBF, 0)

// Layer 1 command-line surface. `run` is the supervised daemon; the remaining verbs exist so each
// Layer 1 module can be exercised against the real system independently.

let usage = """
    usage: holy-blocker-macd <command>

      services                          list network services as the daemon sees them
      ca-status   <cert-path> <name>    report System-keychain trust state for the CA
      ca-install  <cert-path> <name>    install the CA as a trusted root (requires root)
      ca-uninstall <cert-path> <name>   remove trust setting and certificate (requires root)
      proxy-status                      show current proxy settings per service
      proxy-apply <host> <port>         point enabled services at the proxy
      proxy-restore                     restore settings from the snapshot
      firefox-status                    report whether Firefox trusts OS roots
      firefox-trust                     enable the ImportEnterpriseRoots policy (requires root)
      firefox-untrust                   remove it again (requires root)
      permissions [user]                report Layer 2 permission and tamper-surface state
      capture                            grab one frame via ScreenCaptureKit and report on it
      ax-text [delay] [no-manual]       read the frontmost window's AX text (default delay: 3s;
                                        no-manual skips Chromium's AXManualAccessibility opt-in)
      overlay [seconds] [passive]       cover every screen with a real overlay window
      image-scan <model.onnx> [size]    classify a synthetic frame with the real ONNX model
      bundle <output-dir> [identity] [ffi-lib-dir]
                                        assemble and sign HolyBlockerDaemon.app (default: ad-hoc)
      bundle-status                     report our own bundle and whether its grants will last
      launchd-plist <daemon|agent>      print the launchd job definition for one half
      run <proxy-binary> <proxy-dir>    supervise the proxy: launch, wait, route, restore on exit
      agent                             Layer 2: watch for permission loss (LaunchAgent entry)

    Proxy changes need no root for an admin user — only the default state directory does.

    Environment:
      HOLY_BLOCKER_STATE_DIR            where the proxy snapshot is stored
                                        (default: /Library/Application Support/HolyBlocker)
      HOLY_BLOCKER_PROXY_HOST           proxy listen host for `run` (default: 127.0.0.1)
      HOLY_BLOCKER_PROXY_PORT           proxy listen port for `run` (default: 8080)
    """

let stateDirectory = URL(
    fileURLWithPath: ProcessInfo.processInfo.environment["HOLY_BLOCKER_STATE_DIR"]
        ?? "/Library/Application Support/HolyBlocker")
let snapshotPath = stateDirectory.appendingPathComponent("proxy-snapshot.json")

let runner = SystemCommandRunner()

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("holy-blocker-macd: \(message)\n".utf8))
    exit(1)
}

func loadServices() throws -> [NetworkService] {
    let result = try runner.run(SystemTool.networksetup, ["-listallnetworkservices"])
    return NetworkServices.parseServiceList(result.standardOutput)
}

func trust(_ arguments: [String]) -> CATrust {
    guard arguments.count == 2 else { fail("expected <cert-path> <common-name>") }
    return CATrust(
        runner: runner,
        certificatePath: URL(fileURLWithPath: arguments[0]),
        commonName: arguments[1])
}

let proxyHost = ProcessInfo.processInfo.environment["HOLY_BLOCKER_PROXY_HOST"] ?? "127.0.0.1"
let proxyPort = Int(ProcessInfo.processInfo.environment["HOLY_BLOCKER_PROXY_PORT"] ?? "") ?? 8080

/// Set from a signal handler, so it must stay a plain flag with no allocation behind it.
nonisolated(unsafe) var stopRequested: sig_atomic_t = 0

/// Supervises the proxy until interrupted, then puts the machine back.
func runSupervisor(binary: URL, workingDirectory: URL) throws {
    let configuration = ProxyConfiguration(runner: runner, snapshotPath: snapshotPath)
    let supervisor = ProxySupervisor(
        host: proxyHost,
        port: proxyPort,
        process: MitmProxyProcess(executable: binary, workingDirectory: workingDirectory),
        probe: TCPListenerProbe(),
        settings: SystemProxySettings(
            configuration: configuration, runner: runner, host: proxyHost, port: proxyPort))

    // Without this the machine is left pointed at a port that dies with us on Ctrl-C.
    for received in [SIGINT, SIGTERM] {
        signal(received) { _ in stopRequested = 1 }
    }

    print("starting proxy \(binary.path) for \(proxyHost):\(proxyPort)")
    try supervisor.start()

    guard supervisor.state == .configured else {
        fail(supervisor.lastFailure ?? "proxy did not become healthy")
    }
    print("routing traffic to \(proxyHost):\(proxyPort) — ^C to restore")

    while stopRequested == 0 && supervisor.state != .stopped {
        try supervisor.checkLiveness()
        Thread.sleep(forTimeInterval: 1)
    }

    try supervisor.stop()
    if let failure = supervisor.lastFailure { fail(failure) }
    print("restored network settings")
}

/// Poll interval for the Layer 2 agent's tamper watch. Permission loss is not urgent to the
/// second, and a tight loop would spend the day waking the machine for four subprocesses.
let permissionPollSeconds: TimeInterval = 30

/// Scan/reconcile cadence for the render loop. An explicitly acknowledged stand-in for module 11
/// (`EventHooks`), not a replacement for it — a real implementation reacts to foreground-change and
/// scroll events instead of polling a fixed timer. 0.25s keeps the interstitial's appearance within
/// the ~1-2s the plan's live-verification step checks for, given `AccessibilityScanner`'s own
/// ~1s rate-limit gate and `ScanLoopConfig`'s ~0.5-2s cadences sit above it.
let agentScanTickSeconds: TimeInterval = 0.25

/// How often the shutdown watch checks `stopRequested`. `NSApplication.run()` owns the thread once
/// the render loop starts, so a signal handler can only set the flag — this timer is what actually
/// asks the app to terminate.
let agentStopPollSeconds: TimeInterval = 0.25

/// The Layer 2 entry point: stay alive, watch for permission loss, and — as of this session — run
/// the real render loop: an accessory `NSApplication` whose scan/reconcile timer scores the
/// frontmost window's on-screen text and keeps a real overlay in sync with the verdict.
///
/// This is the macOS shape of the Android `GuardStatusService` lesson — the still-alive process
/// that can record a disable the guard itself cannot report. It now covers, not just watches.
///
/// Only the two `await`s below live in this `async` function; everything that schedules a Timer
/// closure is pushed into the non-`async` `runRenderLoop` instead. That split is deliberate, not
/// stylistic: SE-0414 region isolation ties a value born inside an `async` function to that call's
/// task region, and capturing such a value from an escaping Timer closure reads to the compiler as
/// a potential data race even though everything here only ever touches the main thread. A plain
/// `@MainActor` function — the same shape `runOverlay` already uses — does not have that region,
/// so passing `gate`/`capture` across the function boundary as parameters clears it.
@MainActor
func runAgent() async throws {
    let gate = PermissionGate(runner: runner)

    let signature = try CodeSigning(runner: runner).identity(of: CodeSigning.currentCodePath)
    if !signature.isStable {
        // Worth saying loudly: every grant this process holds dies with the next build, and the
        // symptom is a capture path that silently returns nothing.
        print("warning: signature is \(signature) — TCC grants will not survive a rebuild")
    }

    for received in [SIGINT, SIGTERM] {
        signal(received) { _ in stopRequested = 1 }
    }

    let initial = try gate.poll()  // Establishes the baseline; reports nothing by design.
    _ = initial
    if let snapshot = gate.lastSnapshot {
        print("watching: \(PermissionGate.assess(snapshot).level.rawValue)")

        // Ask for Accessibility from *this* process rather than leaving it to be added by hand.
        // Adding an app through the Settings pane's + button resolves an entry the daemon's own
        // `AXIsProcessTrusted` may not match; the prompting API registers the running process's
        // real identity, which is how Screen Recording came to be granted correctly. Once per
        // launch, and only when it is not already held — the prompt is a no-op when it is.
        if snapshot.accessibility != .granted {
            print("requesting accessibility — approve the prompt, then this agent restarts itself")
            SystemPermissionProbe().requestAccess(to: .accessibility)
        }
    }

    let capture = SCShareableContentCapture()
    try await capture.start()

    try runRenderLoop(gate: gate, capture: capture)
}

/// Owns the render loop's state and gives its three Timer callbacks one thing to capture.
///
/// `ScanLoop` and `PermissionGate` are plain (non-`Sendable`) `MacDaemon` types — correctly so,
/// since neither is otherwise isolated to any particular thread. But `Timer.scheduledTimer`'s block
/// is `@Sendable`, so capturing either directly flags as a potential data race under Swift 6's
/// concurrency checking, even though every access below only ever happens on the main run loop.
/// Marking `AgentRenderLoop` itself `@MainActor` — the same reason `Overlay.swift`'s
/// `OverlayController` is `@MainActor` rather than a plain class — makes it, and only it, the thing
/// the timers need to capture.
@MainActor
final class AgentRenderLoop {
    private let gate: PermissionGate
    private let capture: SCShareableContentCapture
    private let scanLoop: ScanLoop
    private let scanner: AccessibilityScanner
    private let imageScanner: ImageScanner
    /// What the image path is actually doing, for the state line. Resolved once at construction:
    /// "no model on disk" and "a model that scores nothing" look identical from the verdict alone.
    private let imagePathStatus: String
    private let overlay: OverlayController
    private let suppressor = WindowSuppressor()

    /// Diagnostic state — see `reportStateIfChanged`.
    private let diagnosticProbe = SystemAXProbe()
    private var lastReportedState: String?
    private var lastReportedAt: Date?
    private var lastTextProbeAt: Date?
    private var lastTextLength = 0

    init(gate: PermissionGate, capture: SCShareableContentCapture) {
        self.gate = gate
        self.capture = capture
        let scanner = AccessibilityScanner(probe: SystemAXProbe(), policy: RealPolicyEngine())
        self.scanner = scanner

        // The model is sealed inside the bundle. A missing or unloadable one degrades the image
        // path to allow-everything rather than taking the daemon down — the text path is
        // independent of it and must keep running. Which of the two happened is reported, because
        // a silently disabled classifier is indistinguishable from a clean screen.
        let modelURL = Bundle.main.url(
            forResource: AppBundle.classifierModelName, withExtension: nil)
        let classifier: ImageClassifying
        if let modelURL, let loaded = try? RealImageClassifier(modelPath: modelURL.path) {
            classifier = loaded
            self.imagePathStatus = "on (threshold \(loaded.threshold))"
        } else {
            classifier = RealImageClassifier.disabled()
            self.imagePathStatus =
                modelURL == nil
                ? "off (no \(AppBundle.classifierModelName) in bundle)"
                : "off (model failed to load)"
        }
        let imageScanner = ImageScanner(classifier: classifier)
        self.imageScanner = imageScanner

        self.scanLoop = ScanLoop(
            capture: capture, imageScanner: imageScanner, textScanner: scanner)
        self.overlay = OverlayController()
    }

    /// Draws windows and needs no Dock icon/menu bar — must run before the first `OverlayController`
    /// reconcile, since an `NSWindow` created before `NSApplication` exists never appears.
    func start() {
        AppLifecycle.configureAccessoryApp()
        overlay.start()
    }

    /// Ticks the scan loop, then drives the overlay off whatever it decided.
    func scanTick() {
        scanLoop.tick(now: Date())
        let intent = overlayIntent(forVerdict: scanLoop.lastVerdict)
        overlay.apply(intent: intent)

        // The overlay is drawn first and the application hidden second, deliberately: the cover is
        // instant and the hide is a round trip to another process. Covering alone is not enough —
        // unfocusing every window drops the cover while the content stays put, and Mission Control
        // composites live previews above it. See `WindowSuppression`.
        if let verdict = scanLoop.lastVerdict {
            let command = suppressor.apply(
                action: verdict.action, target: scanner.lastVerdictApplication)
            if case .hide(let bundleIdentifier) = command { print("hiding: \(bundleIdentifier)") }
        }

        reportStateIfChanged(intent: intent)
    }

    /// Session 6 diagnostic. The render loop is otherwise completely silent — the overlay is its
    /// only observable — so a live pass that sees nothing on screen cannot tell which of four
    /// stages failed. This prints one line whenever the pipeline's state *changes*, which is
    /// quiet when nothing is happening and self-explanatory when something is.
    ///
    /// Deliberately reports the frame's dimensions rather than a bare "have one": the
    /// `!frame.isEmpty` gate in `ScanLoop.tick` is the known coupling this pass is most likely to
    /// trip on, and "empty" is the difference between a starved `SCStream` and a scanner that ran
    /// and allowed. Text is reported as a **character count only** — never its content, which is
    /// the screen of the person being protected, not debugging material.
    private func reportStateIfChanged(intent: OverlayIntent) {
        let frame = capture.currentFrame()
        let verdict = scanLoop.lastVerdict
        let tally = capture.diagnostics
        // The delivery tally is excluded from the change key on purpose — it moves every tick, and
        // keying on it would turn this into a 4-line-per-second log. The heartbeat below is what
        // makes it observable.
        let key = [
            "frame: " + (frame.isEmpty ? "empty" : "\(frame.width)x\(frame.height)"),
            "ax grant: \(gate.lastSnapshot.map { "\($0.accessibility)" } ?? "unpolled")",
            "text: \(diagnosticTextLength()) chars",
            // Status only, never the running count: the count moves on every classification, and
            // keying on it would print two lines a second. The tally goes in the suffix below,
            // beside the capture one, for the same reason.
            "image: \(imagePathStatus)",
            "verdict: "
                + (verdict.map { "\($0.action) (score \(String(format: "%.2f", $0.score)))" }
                    ?? "none"),
            "intent: \(intent)",
            "overlay: " + (overlay.isShowing ? "up" : "down"),
        ].joined(separator: "  ")

        let instant = Date()
        let heartbeatDue = lastReportedAt.map { instant.timeIntervalSince($0) >= 10 } ?? true
        guard key != lastReportedState || heartbeatDue else { return }
        lastReportedState = key
        lastReportedAt = instant
        print(
            key
                + "  deliveries: \(tally.deliveries) (\(tally.complete) complete, \(tally.retained) retained)"
                + "  dropped: \(tally.noImageBuffer) no-buffer / \(tally.noBaseAddress) no-base / \(tally.emptyAfterDepad) depad"
                + "  geometry: \(tally.lastGeometry)"
                + "  classified: \(imageScanner.completedClassifications)"
        )
    }

    /// An independent AX read for the diagnostic above, rate-limited to once a second so it costs
    /// about what the scanner's own walk does. This is the one signal that cannot be inferred from
    /// the others: a zero here with a granted Accessibility toggle means the grant is not live for
    /// *this* process, which is a different problem from a policy that scored the text `allow`.
    private func diagnosticTextLength() -> Int {
        let instant = Date()
        if let last = lastTextProbeAt, instant.timeIntervalSince(last) < 1.0 {
            return lastTextLength
        }
        lastTextProbeAt = instant
        lastTextLength = AccessibilityText.extractFocusedText(
            probe: diagnosticProbe, limits: .standard
        ).count
        return lastTextLength
    }

    /// Unchanged cadence and behavior from before this session's render loop existed.
    func tamperTick() {
        guard let events = try? gate.poll() else { return }
        for event in events { print("tamper: \(event)") }
    }

    func shutdown() {
        overlay.stop()
        Task { try? await capture.stop() }
        NSApplication.shared.terminate(nil)
    }
}

/// Wires the scan/reconcile, tamper-watch and shutdown timers and hands the thread to
/// `NSApplication.run()`. See `runAgent`'s doc comment for why this is a separate, non-`async`
/// function rather than the tail of it.
@MainActor
func runRenderLoop(gate: PermissionGate, capture: SCShareableContentCapture) throws {
    let loop = AgentRenderLoop(gate: gate, capture: capture)
    loop.start()

    // Created before `app.run()` and on the main thread — a timer added off the main run loop
    // silently never fires, the same trap `AXObserver` and `runOverlay`'s teardown timer both hit.
    Timer.scheduledTimer(withTimeInterval: agentScanTickSeconds, repeats: true) { _ in
        MainActor.assumeIsolated { loop.scanTick() }
    }

    Timer.scheduledTimer(withTimeInterval: permissionPollSeconds, repeats: true) { _ in
        MainActor.assumeIsolated { loop.tamperTick() }
    }

    // Shutdown watch: `app.run()` below owns the thread, so SIGINT/SIGTERM can only set
    // `stopRequested`; this is what turns that flag into an actual `terminate(nil)`.
    Timer.scheduledTimer(withTimeInterval: agentStopPollSeconds, repeats: true) { timer in
        guard stopRequested != 0 else { return }
        timer.invalidate()
        MainActor.assumeIsolated { loop.shutdown() }
    }

    print("agent running — scanning every \(agentScanTickSeconds)s")
    NSApplication.shared.run()
    print("agent stopped")
}

/// Grabs one frame through the real `ScreenCaptureKit` edge and reports on it — the live
/// counterpart to `ScreenCaptureTests`, which only exercises the pure logic behind it.
///
/// Run from a shell this is still subject to the responsible-process rule `permissions` warns
/// about: the Screen Recording grant reported (or refused) belongs to the terminal, not to a
/// future signed bundle.
/// Classifies a synthetic frame with the real model — module 18's live check.
///
/// Deliberately *not* driven by `ScreenCaptureKit`: a binary launched from a shell has no Screen
/// Recording grant, so a capture-driven verb would fail before reaching the classifier and prove
/// nothing about it. A frame built in memory exercises the whole path that is actually new here —
/// the dylib loads, the model loads from disk, Swift hands over a BGRA buffer, ONNX runs, a score
/// comes back — with no permission involved at all.
///
/// What it does not check is what the model *says*: a flat colour is not content, and what the
/// classifier makes of one is not a claim this repository should be making. The assertion is that a
/// number comes back in range.
func runImageScan(modelPath: String, size: Int) {
    let classifier: RealImageClassifier
    do {
        classifier = try RealImageClassifier(modelPath: modelPath)
    } catch {
        fail("could not load \(modelPath): \(error)")
    }
    print("model: \(modelPath)")
    print("threshold: \(classifier.threshold)")

    // Mid-grey, opaque, tightly packed BGRA — the layout `PixelBufferCopy.depad` produces.
    let pixels = [UInt8](repeating: 0x80, count: size * size * 4)
    let started = Date()
    let outcome = classifier.classify(pixels: pixels, width: size, height: size)
    let elapsed = Date().timeIntervalSince(started)

    print("frame: \(size)x\(size) (\(pixels.count) bytes)")
    print("outcome: \(outcome)")
    print("elapsed: \(String(format: "%.1f", elapsed * 1000)) ms")

    switch outcome {
    case .allow(score: nil):
        // The one genuinely bad answer: the model was loaded but nothing classified. On this path
        // that means the frame was refused before inference — a geometry or buffer-size mistake.
        print("warning: no score — the frame never reached the model")
    case .allow(let score), .block(let score as Float?):
        if let score, !(0...1).contains(score) {
            print("warning: score outside 0...1 — the output contract has changed")
        }
    }
}

func runCapture() async throws {
    let capture = SCShareableContentCapture()
    try await capture.start()

    // SCStream delivers frames asynchronously off the queue set in `start()`; give it a moment to
    // produce a first `.complete` delivery before reading.
    try await Task.sleep(nanoseconds: 500_000_000)

    let frame = capture.currentFrame()
    try await capture.stop()

    guard !frame.isEmpty else {
        print("no frame captured — check Screen Recording is granted (see `permissions`)")
        return
    }
    print("captured \(frame.width)x\(frame.height) at \(frame.captured)")
    print("all black: \(FrameAnalysis.isAllBlack(frame))")
}

/// Puts a real overlay on every screen for a few seconds — the live counterpart to
/// `OverlayPlanTests`/`OverlayReconciliationTests`, which only exercise the pure planning behind
/// it. The scan-driven version of this loop is a later step; this verb exists so the window level,
/// the Spaces behaviour and the multi-display reconcile can be watched by a human now.
///
/// Needs no TCC grant at all: drawing a window over the screen is not capturing it.
@MainActor
func runOverlay(seconds: Double, passive: Bool) {
    AppLifecycle.configureAccessoryApp()

    let controller = OverlayController()
    controller.start()
    controller.apply(intent: passive ? .passive : .interstitial(swallowsMouseEvents: true))
    print("overlay up on \(ScreenConfiguration.current().count) screen(s) for \(seconds)s")
    print(passive ? "passive: clicks pass through" : "interstitial: clicks are swallowed")

    // The timer has to be scheduled before `run()` takes the thread, and on the main run loop —
    // a timer created off it is never added to it and silently never fires.
    Timer.scheduledTimer(withTimeInterval: seconds, repeats: false) { _ in
        MainActor.assumeIsolated {
            controller.stop()
            NSApplication.shared.terminate(nil)
        }
    }
    NSApplication.shared.run()
}

/// How long to hold the AX client open between the two walks below. Measured against Chrome 151:
/// its web tree appears about three seconds after a client starts reading, so a shorter pause
/// reports a browser as having no page content when it is merely still building one.
let settleSeconds: TimeInterval = 4

/// Reads the frontmost window's accessibility text through the real AX edge — the live counterpart
/// to `AccessibilityTextTests`, which only exercises the walk behind it.
///
/// The delay exists because the frontmost application at the moment this is typed is the terminal.
/// Bring the window under test forward during it. Chromium-based applications need a second run:
/// `AXManualAccessibility` is set on the first walk and the tree is built asynchronously after it,
/// so the first read against Chrome or an Electron app is legitimately thin or empty.
func runAXText(delay: TimeInterval, manualAccessibility: Bool) {
    if delay > 0 {
        print("bring the window to test frontmost — reading in \(Int(delay))s")
        Thread.sleep(forTimeInterval: delay)
    }

    let frontmost = NSWorkspace.shared.frontmostApplication
    print("frontmost: \(frontmost?.bundleIdentifier ?? frontmost?.localizedName ?? "none")")

    let probe = SystemAXProbe(setsManualAccessibility: manualAccessibility)

    // Walked twice from one process, because a one-shot walk cannot tell three different failures
    // apart: a Chromium tree still being built, an opt-in that does nothing from an external
    // client, and a client that exits before either could matter. The daemon walks repeatedly
    // against a probe that stays alive, so the second walk is the one that describes it.
    for attempt in 1...2 {
        if attempt == 2 { Thread.sleep(forTimeInterval: settleSeconds) }

        guard let root = probe.focusedRoot() else {
            print("no focused root — nothing frontmost, or Accessibility is not granted")
            return
        }

        let started = Date()
        let walk = AccessibilityText.extract(from: root, probe: probe)
        let elapsed = Date().timeIntervalSince(started)

        print(
            "walk \(attempt): \(walk.nodesVisited) nodes, \(walk.text.count) characters, "
                + "\(String(format: "%.3f", elapsed))s "
                + "(depth limit: \(walk.hitDepthLimit), node limit: \(walk.hitNodeLimit))")

        guard attempt == 2 else { continue }
        if walk.text.isEmpty {
            // Not an error, and not "there is no text on screen" — see AccessibilityText's header.
            print("no text — a canvas-drawn UI, or Accessibility is not granted")
        } else {
            print("---")
            print(walk.text)
        }
    }
}

let arguments = Array(CommandLine.arguments.dropFirst())
guard let command = arguments.first else { print(usage); exit(2) }
let rest = Array(arguments.dropFirst())

do {
    switch command {
    case "services":
        let services = try loadServices()
        print("network services (\(services.count)):")
        for service in services {
            print("  \(service.name)\(service.isEnabled ? "" : " [disabled]")")
        }

    case "ca-status":
        print(try trust(rest).state())

    case "ca-install":
        let ca = trust(rest)
        try ca.install()
        print("state: \(try ca.state())")

    case "ca-uninstall":
        let ca = trust(rest)
        try ca.uninstall()
        print("state: \(try ca.state())")

    case "proxy-status":
        let config = ProxyConfiguration(runner: runner, snapshotPath: snapshotPath)
        for snapshot in try config.snapshot(services: try loadServices()) {
            print("\(snapshot.service):")
            print("  web:        \(snapshot.web.map(String.init(describing:)) ?? "unreadable")")
            print("  secureWeb:  \(snapshot.secureWeb.map(String.init(describing:)) ?? "unreadable")")
            print("  bypass:     \(snapshot.bypassDomains)")
        }

    case "proxy-apply":
        guard rest.count == 2, let port = Int(rest[1]) else { fail("expected <host> <port>") }
        let config = ProxyConfiguration(runner: runner, snapshotPath: snapshotPath)
        try config.apply(
            host: rest[0], port: port, bypass: DefaultBypass.domains, to: try loadServices())
        print("applied \(rest[0]):\(port); snapshot at \(snapshotPath.path)")

    case "proxy-restore":
        let config = ProxyConfiguration(runner: runner, snapshotPath: snapshotPath)
        try config.restore()
        print("restored from snapshot")

    case "firefox-status":
        print(try FirefoxTrust(store: CFPreferencesPolicyStore()).state())

    case "firefox-trust":
        let firefox = FirefoxTrust(store: CFPreferencesPolicyStore())
        try firefox.install()
        print("state: \(try firefox.state())")

    case "firefox-untrust":
        let firefox = FirefoxTrust(store: CFPreferencesPolicyStore())
        try firefox.uninstall()
        print("state: \(try firefox.state())")

    case "permissions":
        let gate = PermissionGate(
            probe: SystemPermissionProbe(), runner: runner,
            protectedUser: rest.first ?? NSUserName())
        let snapshot = try gate.snapshot()
        let assessment = PermissionGate.assess(snapshot)

        print("protection: \(assessment.level.rawValue)")
        for capability in Capability.allCases {
            print("  \(capability.rawValue): \(snapshot[capability].rawValue)")
        }
        print("  protected user is admin: \(snapshot.protectedUserIsAdmin)")
        print("  SIP: \(snapshot.sipEnabled ? "enabled" : "not fully enabled")")
        print("  local users: \(snapshot.localUsers.joined(separator: ", "))")
        print("  admin group: \(snapshot.adminGroupMembers.joined(separator: ", "))")
        print("  booted: \(snapshot.bootTime)")
        for weakness in assessment.weaknesses { print("  weakness: \(weakness)") }

        // TCC attributes a request to the *responsible* process, which for a binary started from
        // a shell is the terminal — so the three capability lines above describe the terminal's
        // grants, not the daemon's. Only the signals below them mean anything when run this way.
        print("\nnote: run from a shell, the capability states are the terminal's, not ours.")

    case "capture":
        try await runCapture()

    case "image-scan":
        guard let modelPath = rest.first else { fail("expected <model.onnx> [size]") }
        runImageScan(modelPath: modelPath, size: rest.count > 1 ? Int(rest[1]) ?? 512 : 512)

    case "ax-text":
        runAXText(
            delay: rest.first.flatMap(TimeInterval.init) ?? 3,
            manualAccessibility: !rest.contains("no-manual"))

    case "bundle":
        guard let outputDirectory = rest.first else { fail("expected <output-dir> [identity]") }
        guard let executable = Bundle.main.executableURL else { fail("cannot locate own binary") }
        let identity = BundleIdentity.holyBlocker
        let root = URL(fileURLWithPath: outputDirectory)
            .appendingPathComponent("HolyBlockerDaemon.app")

        // The UniFFI dylib the daemon links. `rest[2]` is where scripts/build-ffi.sh staged it;
        // a missing one is reported rather than skipped silently, since the bundle would assemble
        // and sign perfectly and then die at the first policy call.
        let libraryDirectory = rest.count > 2 ? URL(fileURLWithPath: rest[2]) : nil
        var libraries: [URL] = []
        for name in AppBundle.embeddedLibraryNames {
            guard let candidate = libraryDirectory?.appendingPathComponent(name),
                FileManager.default.fileExists(atPath: candidate.path)
            else {
                print("warning: \(name) not found — run scripts/build-ffi.sh before bundling")
                continue
            }
            libraries.append(candidate)
        }

        // The classifier model. `rest[3]` overrides where it is taken from; the default is the
        // repository's gitignored artifact directory, so a developer who has exported one gets it
        // in the bundle without a flag. Absent, the daemon runs its text path and reports the
        // image path as disabled — see `RealImageClassifier.disabled()`.
        let modelSource =
            rest.count > 3
            ? URL(fileURLWithPath: rest[3])
            : URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
                .appendingPathComponent("../../data/models/\(AppBundle.classifierModelName)")
                .standardizedFileURL
        var resources: [URL] = []
        if FileManager.default.fileExists(atPath: modelSource.path) {
            resources.append(modelSource)
        } else {
            print("warning: no classifier model at \(modelSource.path) — image scanning disabled")
        }

        try AppBundle.assemble(
            at: root, identity: identity, executable: executable, libraries: libraries,
            resources: resources)
        // Ad-hoc by default so the bundle is runnable with no certificate — but ad-hoc is exactly
        // the identity that does not survive a rebuild, so say so rather than leave it implied.
        let signingIdentity = rest.count > 1 ? rest[1] : "-"
        let layout = AppBundle.layout(root: root, identity: identity)
        try CodeSigning(runner: runner).sign(
            bundle: root, identity: signingIdentity,
            nestedCode: try AppBundle.nestedCode(in: layout))

        let applied = try CodeSigning(runner: runner).identity(of: root)
        print("assembled \(root.path)")
        print("signature: \(applied)")
        if !applied.isStable {
            print("warning: TCC grants made against this bundle die on the next build")
        }

    case "bundle-status":
        // Run as a bare binary this reports no identifier at all, which is the point: that is the
        // state in which every grant is keyed to a cdhash that changes on each build.
        print("bundle: \(Bundle.main.bundleURL.path)")
        print("identifier: \(Bundle.main.bundleIdentifier ?? "none — not running from a bundle")")
        let signature = try CodeSigning(runner: runner).identity(of: CodeSigning.currentCodePath)
        print("signature: \(signature)")
        print("grants survive a rebuild: \(signature.isStable)")

    case "launchd-plist":
        guard let half = rest.first else { fail("expected <daemon|agent>") }
        let executable = URL(fileURLWithPath: "/Applications/HolyBlockerDaemon.app")
            .appendingPathComponent("Contents/MacOS/\(BundleIdentity.holyBlocker.executableName)")
        let job: LaunchdJob
        switch half {
        case "daemon":
            job = LaunchdJob.daemon(
                label: "com.holyblocker.daemon", executable: executable,
                arguments: ["run", "/usr/local/bin/mitm-proxy", "/Library/Application Support/HolyBlocker"],
                logPath: URL(fileURLWithPath: "/var/log/holy-blocker-daemon.log"))
        case "agent":
            job = LaunchdJob.agent(
                label: "com.holyblocker.agent", executable: executable, arguments: ["agent"],
                home: FileManager.default.homeDirectoryForCurrentUser, uid: getuid(),
                logPath: FileManager.default.homeDirectoryForCurrentUser
                    .appendingPathComponent("Library/Logs/holy-blocker-agent.log"))
        default:
            fail("expected <daemon|agent>")
        }
        print("# install at: \(job.installPath.path)")
        print("# load with:  launchctl bootstrap \(job.domainTarget) \(job.installPath.path)")
        print("# unload:     launchctl bootout \(job.serviceTarget)")
        print(String(decoding: try job.plist(), as: UTF8.self))

    case "agent":
        try await runAgent()

    case "overlay":
        runOverlay(
            seconds: rest.first.flatMap(Double.init) ?? 5,
            passive: rest.contains("passive"))

    case "run":
        guard rest.count == 2 else { fail("expected <proxy-binary> <proxy-working-dir>") }
        try runSupervisor(
            binary: URL(fileURLWithPath: rest[0]),
            workingDirectory: URL(fileURLWithPath: rest[1]))

    default:
        print(usage)
        exit(2)
    }
} catch {
    fail("\(command) failed: \(error)")
}
