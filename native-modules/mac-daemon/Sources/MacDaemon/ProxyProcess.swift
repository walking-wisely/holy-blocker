import Darwin
import Foundation

/// Launches `mitm-proxy` as a child process.
///
/// The proxy currently takes no command-line arguments: it hardcodes `127.0.0.1:8080` and loads
/// its CA from the relative path `data/ca`, so the working directory is load-bearing and the port
/// is not yet selectable. `arguments` exists so this side needs no change once the Rust binary
/// grows a real CLI — see the note in docs/components/mac-daemon/plan.md.
public final class MitmProxyProcess: ProxyProcessHandle {
    private let executable: URL
    private let arguments: [String]
    private let workingDirectory: URL
    private var process: Process?

    public init(executable: URL, arguments: [String] = [], workingDirectory: URL) {
        self.executable = executable
        self.arguments = arguments
        self.workingDirectory = workingDirectory
    }

    public var isRunning: Bool { process?.isRunning ?? false }

    public func launch() throws {
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw CommandRunnerError.executableNotFound(executable.path)
        }

        // A Process cannot be run twice, so a restart always builds a fresh one.
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        process.currentDirectoryURL = workingDirectory
        try process.run()
        self.process = process
    }

    public func terminate() {
        guard let process, process.isRunning else { return }
        process.terminate()
        process.waitUntilExit()
        self.process = nil
    }
}

/// Checks whether anything is accepting TCP connections at an address.
///
/// A plain blocking `connect(2)` is enough here because the only address ever probed is loopback:
/// the kernel either completes the handshake immediately or returns ECONNREFUSED, with no network
/// round trip that could hang. Probing a remote host would need a non-blocking connect and a
/// timeout.
public struct TCPListenerProbe: ListenerProbe, Sendable {
    public init() {}

    public func isListening(host: String, port: Int) -> Bool {
        let descriptor = socket(AF_INET, SOCK_STREAM, 0)
        guard descriptor >= 0 else { return false }
        defer { close(descriptor) }

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        // Port is network byte order on the wire — RFC 793 §3.1 header layout, as exposed by
        // sockaddr_in in `man 4 inet`.
        address.sin_port = UInt16(truncatingIfNeeded: port).bigEndian
        guard inet_pton(AF_INET, host, &address.sin_addr) == 1 else { return false }

        let connected = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { socketAddress in
                connect(descriptor, socketAddress, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        return connected == 0
    }
}

/// Adapts `ProxyConfiguration` to the narrow interface the supervisor drives.
///
/// The service list is read at apply time rather than construction time: a machine that gains a
/// network service between daemon start and proxy readiness would otherwise leave that service
/// unrouted.
public struct SystemProxySettings: ProxySettingsControlling {
    private let configuration: ProxyConfiguration
    private let runner: CommandRunner
    private let host: String
    private let port: Int
    private let bypass: [String]

    public init(
        configuration: ProxyConfiguration,
        runner: CommandRunner,
        host: String,
        port: Int,
        bypass: [String] = DefaultBypass.domains
    ) {
        self.configuration = configuration
        self.runner = runner
        self.host = host
        self.port = port
        self.bypass = bypass
    }

    public func apply() throws {
        let result = try runner.run(SystemTool.networksetup, ["-listallnetworkservices"])
        let services = NetworkServices.parseServiceList(result.standardOutput)
        try configuration.apply(host: host, port: port, bypass: bypass, to: services)
    }

    public func restore() throws {
        try configuration.restore()
    }
}
