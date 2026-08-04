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

/// Poll interval for the Layer 2 agent. Permission loss is not urgent to the second, and a tight
/// loop would spend the day waking the machine for four subprocesses.
let permissionPollSeconds: TimeInterval = 30

/// The Layer 2 entry point: stay alive and notice when a capability is taken away.
///
/// This is the macOS shape of the Android `GuardStatusService` lesson — the still-alive process
/// that can record a disable the guard itself cannot report. It watches; it does not yet capture.
func runAgent() throws {
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
    }

    while stopRequested == 0 {
        Thread.sleep(forTimeInterval: permissionPollSeconds)
        for event in try gate.poll() { print("tamper: \(event)") }
    }
    print("agent stopped")
}

/// Grabs one frame through the real `ScreenCaptureKit` edge and reports on it — the live
/// counterpart to `ScreenCaptureTests`, which only exercises the pure logic behind it.
///
/// Run from a shell this is still subject to the responsible-process rule `permissions` warns
/// about: the Screen Recording grant reported (or refused) belongs to the terminal, not to a
/// future signed bundle.
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

        try AppBundle.assemble(
            at: root, identity: identity, executable: executable, libraries: libraries)
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
        try runAgent()

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
