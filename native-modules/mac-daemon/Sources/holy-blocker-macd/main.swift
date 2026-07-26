import Foundation
import MacDaemon

// Layer 1 command-line surface. The long-running daemon loop lands with ProxySupervisor; these
// verbs exist so each Layer 1 module can be exercised against the real system independently.

let usage = """
    usage: holy-blocker-macd <command>

      services                          list network services as the daemon sees them
      ca-status   <cert-path> <name>    report System-keychain trust state for the CA
      ca-install  <cert-path> <name>    install the CA as a trusted root (requires root)
      ca-uninstall <cert-path> <name>   remove trust setting and certificate (requires root)
      proxy-status                      show current proxy settings per service
      proxy-apply <host> <port>         point enabled services at the proxy
      proxy-restore                     restore settings from the snapshot

    Proxy changes need no root for an admin user — only the default state directory does.

    Environment:
      HOLY_BLOCKER_STATE_DIR            where the proxy snapshot is stored
                                        (default: /Library/Application Support/HolyBlocker)
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

    default:
        print(usage)
        exit(2)
    }
} catch {
    fail("\(command) failed: \(error)")
}
