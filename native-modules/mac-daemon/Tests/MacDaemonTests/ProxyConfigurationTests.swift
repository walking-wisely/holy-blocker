import Foundation
import Testing

@testable import MacDaemon

private func makeSnapshotPath() -> URL {
    FileManager.default.temporaryDirectory
        .appendingPathComponent("holy-blocker-proxy-\(UUID().uuidString).json")
}

/// Stubs a runner so every service reports no proxy configured and no bypass domains.
private func stubCleanMachine(_ runner: FakeCommandRunner) {
    runner.stub(
        subcommand: "-getwebproxy",
        result: CommandResult(
            exitCode: 0, standardOutput: "Enabled: No\nServer: \nPort: 0\n"))
    runner.stub(
        subcommand: "-getsecurewebproxy",
        result: CommandResult(
            exitCode: 0, standardOutput: "Enabled: No\nServer: \nPort: 0\n"))
    runner.stub(
        subcommand: "-getproxybypassdomains",
        result: CommandResult(
            exitCode: 0,
            standardOutput: "There aren't any bypass domains set on this network service."))
}

private let wifi = NetworkService(name: "Wi-Fi", isEnabled: true)
private let usbLan = NetworkService(name: "USB 10/100/1000 LAN", isEnabled: true)
private let disabledBridge = NetworkService(name: "Thunderbolt Bridge", isEnabled: false)

@Suite("ProxyConfiguration.apply")
struct ProxyConfigurationApplyTests {
    @Test("sets both the web and secure web proxy for an enabled service")
    func setsBothProxies() throws {
        let runner = FakeCommandRunner()
        stubCleanMachine(runner)
        let config = ProxyConfiguration(runner: runner, snapshotPath: makeSnapshotPath())

        try config.apply(host: "127.0.0.1", port: 8080, bypass: ["*.local"], to: [wifi])

        let web = try #require(runner.invocations.first { $0.arguments.first == "-setwebproxy" })
        #expect(web.executable == "/usr/sbin/networksetup")
        #expect(web.arguments == ["-setwebproxy", "Wi-Fi", "127.0.0.1", "8080"])

        let secure = try #require(
            runner.invocations.first { $0.arguments.first == "-setsecurewebproxy" })
        // HTTPS is the whole point — configuring only the plain web proxy would leave every
        // CONNECT request going direct.
        #expect(secure.arguments == ["-setsecurewebproxy", "Wi-Fi", "127.0.0.1", "8080"])
    }

    @Test("passes a service name containing spaces as a single argument")
    func serviceNameWithSpaces() throws {
        let runner = FakeCommandRunner()
        stubCleanMachine(runner)
        let config = ProxyConfiguration(runner: runner, snapshotPath: makeSnapshotPath())

        try config.apply(host: "127.0.0.1", port: 8080, bypass: [], to: [usbLan])

        let web = try #require(runner.invocations.first { $0.arguments.first == "-setwebproxy" })
        #expect(web.arguments[1] == "USB 10/100/1000 LAN")
    }

    @Test("skips disabled services")
    func skipsDisabled() throws {
        let runner = FakeCommandRunner()
        stubCleanMachine(runner)
        let config = ProxyConfiguration(runner: runner, snapshotPath: makeSnapshotPath())

        try config.apply(
            host: "127.0.0.1", port: 8080, bypass: [], to: [disabledBridge, wifi])

        let configured = runner.invocations
            .filter { $0.arguments.first == "-setwebproxy" }
            .map { $0.arguments[1] }
        #expect(configured == ["Wi-Fi"])
    }

    @Test("applies the bypass list, and clears it with the Empty sentinel when there is none")
    func bypassList() throws {
        let runner = FakeCommandRunner()
        stubCleanMachine(runner)
        let config = ProxyConfiguration(runner: runner, snapshotPath: makeSnapshotPath())

        try config.apply(
            host: "127.0.0.1", port: 8080, bypass: ["*.local", "169.254/16"], to: [wifi])
        let set = try #require(
            runner.invocations.first { $0.arguments.first == "-setproxybypassdomains" })
        #expect(
            set.arguments == ["-setproxybypassdomains", "Wi-Fi", "*.local", "169.254/16"])

        runner.reset()
        stubCleanMachine(runner)
        try config.apply(host: "127.0.0.1", port: 8080, bypass: [], to: [wifi])
        let cleared = try #require(
            runner.invocations.first { $0.arguments.first == "-setproxybypassdomains" })
        // networksetup(8) has no "remove all" verb; the literal string Empty is the sentinel.
        #expect(cleared.arguments == ["-setproxybypassdomains", "Wi-Fi", "Empty"])
    }

    @Test("writes a snapshot before mutating anything")
    func snapshotWrittenFirst() throws {
        let runner = FakeCommandRunner()
        stubCleanMachine(runner)
        let path = makeSnapshotPath()
        defer { try? FileManager.default.removeItem(at: path) }
        let config = ProxyConfiguration(runner: runner, snapshotPath: path)

        try config.apply(host: "127.0.0.1", port: 8080, bypass: [], to: [wifi])

        // A crash between mutating and persisting would strand the user behind a dead proxy with
        // no record of how to get back.
        #expect(FileManager.default.fileExists(atPath: path.path))
        let firstMutation = try #require(
            runner.invocations.firstIndex { $0.arguments.first == "-setwebproxy" })
        let lastRead = try #require(
            runner.invocations.lastIndex { $0.arguments.first == "-getwebproxy" })
        #expect(lastRead < firstMutation)
    }
}

@Suite("ProxyConfiguration.restore")
struct ProxyConfigurationRestoreTests {
    @Test("turns the proxy off for a service that had none before")
    func restoresToOff() throws {
        let runner = FakeCommandRunner()
        stubCleanMachine(runner)
        let path = makeSnapshotPath()
        defer { try? FileManager.default.removeItem(at: path) }
        let config = ProxyConfiguration(runner: runner, snapshotPath: path)

        try config.apply(host: "127.0.0.1", port: 8080, bypass: ["*.local"], to: [wifi])
        runner.reset()
        try config.restore()

        // Turning the state off is not sufficient: networksetup keeps the server and port, so the
        // machine is left holding 127.0.0.1:8080 and re-enabling the proxy for any unrelated
        // reason silently routes traffic at a dead local port. The host must be cleared too.
        // `-setwebproxy <service> "" 0` is accepted and blanks it (verified on macOS 26.5), but it
        // also turns the proxy on, so the state-off call has to come after.
        let clearIndex = try #require(
            runner.invocations.firstIndex { $0.arguments == ["-setwebproxy", "Wi-Fi", "", "0"] })
        let offIndex = try #require(
            runner.invocations.firstIndex {
                $0.arguments == ["-setwebproxystate", "Wi-Fi", "off"]
            })
        #expect(clearIndex < offIndex)

        let secureClearIndex = try #require(
            runner.invocations.firstIndex {
                $0.arguments == ["-setsecurewebproxy", "Wi-Fi", "", "0"]
            })
        let secureOffIndex = try #require(
            runner.invocations.firstIndex {
                $0.arguments == ["-setsecurewebproxystate", "Wi-Fi", "off"]
            })
        #expect(secureClearIndex < secureOffIndex)
    }

    @Test("does not clear the host when a prior proxy is being restored")
    func doesNotClearWhenRestoringPriorProxy() throws {
        let runner = FakeCommandRunner()
        runner.stub(
            subcommand: "-getwebproxy",
            result: CommandResult(
                exitCode: 0,
                standardOutput: "Enabled: Yes\nServer: corp.example.com\nPort: 3128\n"))
        runner.stub(
            subcommand: "-getsecurewebproxy",
            result: CommandResult(exitCode: 0, standardOutput: "Enabled: No\nServer: \nPort: 0\n"))
        runner.stub(
            subcommand: "-getproxybypassdomains",
            result: CommandResult(exitCode: 0, standardOutput: "\n"))

        let path = makeSnapshotPath()
        defer { try? FileManager.default.removeItem(at: path) }
        let config = ProxyConfiguration(runner: runner, snapshotPath: path)

        try config.apply(host: "127.0.0.1", port: 8080, bypass: [], to: [wifi])
        runner.reset()
        try config.restore()

        #expect(!runner.invocations.contains { $0.arguments == ["-setwebproxy", "Wi-Fi", "", "0"] })
        // The secure proxy had none, so that one still gets cleared.
        #expect(
            runner.invocations.contains {
                $0.arguments == ["-setsecurewebproxy", "Wi-Fi", "", "0"]
            })
    }

    @Test("restores a pre-existing proxy exactly as it was found")
    func restoresPriorProxy() throws {
        let runner = FakeCommandRunner()
        runner.stub(
            subcommand: "-getwebproxy",
            result: CommandResult(
                exitCode: 0,
                standardOutput: "Enabled: Yes\nServer: corp.example.com\nPort: 3128\n"))
        runner.stub(
            subcommand: "-getsecurewebproxy",
            result: CommandResult(
                exitCode: 0,
                standardOutput: "Enabled: Yes\nServer: corp.example.com\nPort: 3129\n"))
        runner.stub(
            subcommand: "-getproxybypassdomains",
            result: CommandResult(exitCode: 0, standardOutput: "internal.example.com\n"))

        let path = makeSnapshotPath()
        defer { try? FileManager.default.removeItem(at: path) }
        let config = ProxyConfiguration(runner: runner, snapshotPath: path)

        try config.apply(host: "127.0.0.1", port: 8080, bypass: ["*.local"], to: [wifi])
        runner.reset()
        try config.restore()

        #expect(
            runner.invocations.contains {
                $0.arguments == ["-setwebproxy", "Wi-Fi", "corp.example.com", "3128"]
            })
        #expect(
            runner.invocations.contains {
                $0.arguments == ["-setsecurewebproxy", "Wi-Fi", "corp.example.com", "3129"]
            })
        #expect(
            runner.invocations.contains {
                $0.arguments == ["-setproxybypassdomains", "Wi-Fi", "internal.example.com"]
            })
    }

    @Test("honours a snapshot left on disk by a previously crashed run")
    func restoresAcrossProcessRestart() throws {
        let path = makeSnapshotPath()
        defer { try? FileManager.default.removeItem(at: path) }

        let firstRunner = FakeCommandRunner()
        stubCleanMachine(firstRunner)
        try ProxyConfiguration(runner: firstRunner, snapshotPath: path)
            .apply(host: "127.0.0.1", port: 8080, bypass: [], to: [wifi])

        // A fresh instance with no in-memory state, standing in for the next daemon start.
        let secondRunner = FakeCommandRunner()
        try ProxyConfiguration(runner: secondRunner, snapshotPath: path).restore()

        #expect(
            secondRunner.invocations.contains {
                $0.arguments == ["-setwebproxystate", "Wi-Fi", "off"]
            })
    }

    @Test("deletes the snapshot once restored so it is not replayed")
    func clearsSnapshot() throws {
        let runner = FakeCommandRunner()
        stubCleanMachine(runner)
        let path = makeSnapshotPath()
        defer { try? FileManager.default.removeItem(at: path) }
        let config = ProxyConfiguration(runner: runner, snapshotPath: path)

        try config.apply(host: "127.0.0.1", port: 8080, bypass: [], to: [wifi])
        try config.restore()

        #expect(!FileManager.default.fileExists(atPath: path.path))
    }

    @Test("is a harmless no-op when there is no snapshot")
    func noSnapshot() throws {
        let runner = FakeCommandRunner()
        let config = ProxyConfiguration(runner: runner, snapshotPath: makeSnapshotPath())

        try config.restore()

        #expect(runner.invocations.isEmpty)
    }
}
