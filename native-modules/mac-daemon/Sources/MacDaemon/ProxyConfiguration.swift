import Foundation

/// The proxy state of one network service, captured before the daemon modified it.
public struct ProxySnapshot: Codable, Equatable, Sendable {
    public let service: String
    public let web: ProxySetting?
    public let secureWeb: ProxySetting?
    public let bypassDomains: [String]
}

public enum ProxyConfigurationError: Error, Equatable {
    case commandFailed(arguments: [String], exitCode: Int32, message: String)
}

/// Points every enabled network service at the local proxy, and puts them back exactly as found.
///
/// macOS proxy settings are per network service, not global, so every operation iterates. The
/// prior state of each service is snapshotted to disk before anything is modified: the daemon can
/// be killed between configure and restore, and a crash must not strand the user behind a dead
/// proxy with no route back.
public struct ProxyConfiguration {
    /// networksetup(8) has no verb for clearing the bypass list; this literal is the sentinel.
    private static let clearBypassSentinel = "Empty"

    private let runner: CommandRunner
    private let snapshotPath: URL

    public init(runner: CommandRunner, snapshotPath: URL) {
        self.runner = runner
        self.snapshotPath = snapshotPath
    }

    // MARK: - Snapshot

    public func snapshot(services: [NetworkService]) throws -> [ProxySnapshot] {
        try services.filter(\.isEnabled).map { service in
            ProxySnapshot(
                service: service.name,
                web: NetworkServices.parseProxySetting(
                    try read("-getwebproxy", service.name)),
                secureWeb: NetworkServices.parseProxySetting(
                    try read("-getsecurewebproxy", service.name)),
                bypassDomains: NetworkServices.parseBypassDomains(
                    try read("-getproxybypassdomains", service.name))
            )
        }
    }

    // MARK: - Apply

    public func apply(
        host: String, port: Int, bypass: [String], to services: [NetworkService]
    ) throws {
        let enabled = services.filter(\.isEnabled)

        // Read and persist before mutating, so an interrupted run is always recoverable.
        let snapshots = try snapshot(services: enabled)
        try persist(snapshots)

        for service in enabled {
            try exec(["-setwebproxy", service.name, host, String(port)])
            try exec(["-setsecurewebproxy", service.name, host, String(port)])
            try applyBypass(bypass, to: service.name)
        }
    }

    // MARK: - Restore

    public func restore() throws {
        guard let data = try? Data(contentsOf: snapshotPath) else { return }
        let snapshots = try JSONDecoder().decode([ProxySnapshot].self, from: data)

        for snapshot in snapshots {
            try restore(snapshot.web, on: snapshot.service, secure: false)
            try restore(snapshot.secureWeb, on: snapshot.service, secure: true)
            try applyBypass(snapshot.bypassDomains, to: snapshot.service)
        }

        // Drop the snapshot so a later start does not replay stale settings over whatever the
        // user has configured since.
        try? FileManager.default.removeItem(at: snapshotPath)
    }

    private func restore(_ setting: ProxySetting?, on service: String, secure: Bool) throws {
        let setVerb = secure ? "-setsecurewebproxy" : "-setwebproxy"
        let stateVerb = secure ? "-setsecurewebproxystate" : "-setwebproxystate"

        guard let setting, setting.isEnabled, !setting.server.isEmpty else {
            // Turning the state off is not enough on its own: networksetup keeps the server and
            // port it was last given, so the service would be left holding our listener address
            // and re-enabling the proxy for any unrelated reason would route traffic at a dead
            // local port. Passing an empty domain and port 0 blanks both fields — accepted by
            // networksetup(8) despite being undocumented, verified on macOS 26.5.
            //
            // Order matters: -setwebproxy also turns the proxy *on*, so the state-off call has to
            // come second or the service is left enabled.
            try exec([setVerb, service, "", "0"])
            try exec([stateVerb, service, "off"])
            return
        }

        try exec([setVerb, service, setting.server, String(setting.port)])
    }

    // MARK: - Helpers

    private func applyBypass(_ domains: [String], to service: String) throws {
        let arguments =
            domains.isEmpty
            ? ["-setproxybypassdomains", service, Self.clearBypassSentinel]
            : ["-setproxybypassdomains", service] + domains
        try exec(arguments)
    }

    private func persist(_ snapshots: [ProxySnapshot]) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try FileManager.default.createDirectory(
            at: snapshotPath.deletingLastPathComponent(),
            withIntermediateDirectories: true)
        try encoder.encode(snapshots).write(to: snapshotPath, options: .atomic)
    }

    private func read(_ verb: String, _ service: String) throws -> String {
        try exec([verb, service]).standardOutput
    }

    @discardableResult
    private func exec(_ arguments: [String]) throws -> CommandResult {
        let result = try runner.run(SystemTool.networksetup, arguments)
        guard result.succeeded else {
            throw ProxyConfigurationError.commandFailed(
                arguments: arguments,
                exitCode: result.exitCode,
                message: result.standardError)
        }
        return result
    }
}

/// The minimum bypass list.
///
/// Loopback and Bonjour must never route through the proxy, and captive-portal detection has to
/// work or the user cannot join a hotel or airport network while protected.
public enum DefaultBypass {
    public static let domains = [
        "localhost",
        "127.0.0.1",
        "::1",
        "*.local",
        // Link-local / APIPA range, RFC 3927 §2.1.
        "169.254/16",
        // Apple's captive-portal probe host.
        "captive.apple.com",
    ]
}
