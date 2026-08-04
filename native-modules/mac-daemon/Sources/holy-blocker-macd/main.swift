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
      run <proxy-binary> <proxy-dir>    supervise the proxy: launch, wait, route, restore on exit

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
