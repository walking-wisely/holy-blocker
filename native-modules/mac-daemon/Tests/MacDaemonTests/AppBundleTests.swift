import Foundation
import Testing

@testable import MacDaemon

private let identity = BundleIdentity(
    identifier: "com.holyblocker.daemon",
    name: "Holy Blocker",
    executableName: "holy-blocker-macd",
    version: "0.1.0",
    build: "1",
    minimumSystemVersion: "14.0")

private func decodePlist(_ data: Data) throws -> [String: Any] {
    let object = try PropertyListSerialization.propertyList(from: data, format: nil)
    return object as? [String: Any] ?? [:]
}

// MARK: - Layout

@Suite("AppBundle.layout")
struct BundleLayoutTests {
    private let layout = AppBundle.layout(
        root: URL(fileURLWithPath: "/tmp/HolyBlockerDaemon.app"), identity: identity)

    @Test("puts the executable where launchd and TCC expect to find it")
    func executablePath() {
        // TCC identifies a bundle by its main executable's signature; the path is fixed by the
        // bundle format, not by us.
        #expect(layout.executable.path == "/tmp/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd")
    }

    @Test("puts Info.plist directly under Contents")
    func infoPlistPath() {
        #expect(layout.infoPlist.path == "/tmp/HolyBlockerDaemon.app/Contents/Info.plist")
    }

    @Test("lists every directory that has to exist before assembly")
    func directories() {
        #expect(
            layout.directories.map(\.path) == [
                "/tmp/HolyBlockerDaemon.app/Contents",
                "/tmp/HolyBlockerDaemon.app/Contents/MacOS",
                "/tmp/HolyBlockerDaemon.app/Contents/Resources",
            ])
    }
}

// MARK: - Info.plist

@Suite("AppBundle.infoPlist")
struct InfoPlistTests {
    @Test("carries the identity TCC will hold the grant against")
    func identityKeys() throws {
        let plist = try decodePlist(try AppBundle.infoPlist(for: identity))

        #expect(plist["CFBundleIdentifier"] as? String == "com.holyblocker.daemon")
        #expect(plist["CFBundleExecutable"] as? String == "holy-blocker-macd")
        #expect(plist["CFBundleName"] as? String == "Holy Blocker")
        #expect(plist["CFBundlePackageType"] as? String == "APPL")
    }

    @Test("names the app the way the permission prompt will")
    func displayName() throws {
        // The Screen Recording prompt has fixed wording and interpolates only the bundle name, so
        // this string is the entire opportunity to say who is asking.
        let plist = try decodePlist(try AppBundle.infoPlist(for: identity))

        #expect(plist["CFBundleDisplayName"] as? String == "Holy Blocker")
    }

    @Test("runs without a Dock icon but stays able to show windows")
    func accessoryApp() throws {
        let plist = try decodePlist(try AppBundle.infoPlist(for: identity))

        // LSUIElement, not LSBackgroundOnly: the overlay in module 10 is a real window, and
        // LSBackgroundOnly would forbid it.
        #expect(plist["LSUIElement"] as? Bool == true)
        #expect(plist["LSBackgroundOnly"] == nil)
    }

    @Test("carries no usage-description strings for the three capabilities")
    func noUsageDescriptions() throws {
        // Verified against the key list tccd actually reads on macOS 26.5.2: there is no
        // NSScreenCaptureUsageDescription, no NSInputMonitoringUsageDescription, and no
        // accessibility key. Shipping invented ones would be cargo cult.
        let plist = try decodePlist(try AppBundle.infoPlist(for: identity))

        for key in plist.keys {
            #expect(!key.hasSuffix("UsageDescription"))
        }
    }

    @Test("is a plist the system parser accepts")
    func wellFormed() throws {
        var format = PropertyListSerialization.PropertyListFormat.binary
        _ = try PropertyListSerialization.propertyList(
            from: try AppBundle.infoPlist(for: identity), options: [], format: &format)

        // XML rather than binary so a reviewer can read the diff.
        #expect(format == .xml)
    }
}

// MARK: - Assembly

@Suite("AppBundle.assemble")
struct BundleAssemblyTests {
    /// A stand-in for the built daemon, with the executable bit set the way SwiftPM leaves it.
    private func makeExecutable(in directory: URL) throws -> URL {
        let path = directory.appendingPathComponent("holy-blocker-macd")
        try Data("#!/bin/sh\n".utf8).write(to: path)
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o755], ofItemAtPath: path.path)
        return path
    }

    private func withTemporaryDirectory(_ body: (URL) throws -> Void) throws {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("bundle-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        try body(directory)
    }

    @Test("produces a bundle the system can read as one")
    func assembles() throws {
        try withTemporaryDirectory { directory in
            let root = directory.appendingPathComponent("HolyBlockerDaemon.app")

            try AppBundle.assemble(
                at: root, identity: identity, executable: try makeExecutable(in: directory))

            let layout = AppBundle.layout(root: root, identity: identity)
            #expect(FileManager.default.isExecutableFile(atPath: layout.executable.path))
            let plist = try decodePlist(try Data(contentsOf: layout.infoPlist))
            #expect(plist["CFBundleIdentifier"] as? String == "com.holyblocker.daemon")
            // Bundle only resolves an identifier once the layout is right, which makes this a
            // check on the layout rather than on the plist.
            #expect(Bundle(url: root)?.bundleIdentifier == "com.holyblocker.daemon")
        }
    }

    @Test("replaces a previous bundle rather than merging into it")
    func rebuildIsClean() throws {
        try withTemporaryDirectory { directory in
            let root = directory.appendingPathComponent("HolyBlockerDaemon.app")
            let executable = try makeExecutable(in: directory)
            try AppBundle.assemble(at: root, identity: identity, executable: executable)

            // A stale signature left from the previous build makes codesign refuse the new one.
            let leftover = root.appendingPathComponent("Contents/_CodeSignature")
            try FileManager.default.createDirectory(at: leftover, withIntermediateDirectories: true)

            try AppBundle.assemble(at: root, identity: identity, executable: executable)

            #expect(!FileManager.default.fileExists(atPath: leftover.path))
        }
    }
}

// MARK: - launchd jobs

@Suite("LaunchdJob")
struct LaunchdJobTests {
    private let executable = URL(
        fileURLWithPath: "/Applications/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd")

    @Test("installs the privileged half where only root can write")
    func daemonInstallPath() {
        let job = LaunchdJob.daemon(
            label: "com.holyblocker.daemon", executable: executable, arguments: ["run"])

        #expect(job.installPath.path == "/Library/LaunchDaemons/com.holyblocker.daemon.plist")
        #expect(job.serviceTarget == "system/com.holyblocker.daemon")
    }

    @Test("installs the session half in the user's own library")
    func agentInstallPath() {
        let job = LaunchdJob.agent(
            label: "com.holyblocker.agent", executable: executable, arguments: ["agent"],
            home: URL(fileURLWithPath: "/Users/tester"), uid: 501)

        #expect(job.installPath.path == "/Users/tester/Library/LaunchAgents/com.holyblocker.agent.plist")
        #expect(job.serviceTarget == "gui/501/com.holyblocker.agent")
    }

    @Test("restricts the agent to a graphical login session")
    func agentNeedsAqua() throws {
        // ScreenCaptureKit and AppKit have no window server in a background session, and a job
        // that loads there fails in ways that look like a permission problem.
        let job = LaunchdJob.agent(
            label: "com.holyblocker.agent", executable: executable, arguments: ["agent"],
            home: URL(fileURLWithPath: "/Users/tester"), uid: 501)
        let plist = try decodePlist(try job.plist())

        #expect(plist["LimitLoadToSessionType"] as? String == "Aqua")
    }

    @Test("never asks for a user, so the agent runs as whoever is logged in")
    func agentIsNotRoot() throws {
        // Layer 2 must not be root: TCC grants belong to the user's session, and a root agent
        // would be asking for a permission nobody can give it.
        let job = LaunchdJob.agent(
            label: "com.holyblocker.agent", executable: executable, arguments: ["agent"],
            home: URL(fileURLWithPath: "/Users/tester"), uid: 501)
        let plist = try decodePlist(try job.plist())

        #expect(plist["UserName"] == nil)
    }

    @Test("runs the privileged half as root and outside any session")
    func daemonIsRoot() throws {
        let job = LaunchdJob.daemon(
            label: "com.holyblocker.daemon", executable: executable, arguments: ["run"])
        let plist = try decodePlist(try job.plist())

        #expect(plist["UserName"] as? String == "root")
        #expect(plist["LimitLoadToSessionType"] == nil)
    }

    @Test("passes the executable by absolute path as the first argument")
    func programArguments() throws {
        let job = LaunchdJob.daemon(
            label: "com.holyblocker.daemon", executable: executable, arguments: ["run", "/tmp/p"])
        let plist = try decodePlist(try job.plist())

        #expect(
            plist["ProgramArguments"] as? [String] == [
                executable.path, "run", "/tmp/p",
            ])
    }

    @Test("sends both streams to the log file, or launchd discards them")
    func logPaths() throws {
        let withLog = LaunchdJob.agent(
            label: "com.holyblocker.agent", executable: executable, arguments: ["agent"],
            home: URL(fileURLWithPath: "/Users/tester"), uid: 501,
            logPath: URL(fileURLWithPath: "/tmp/agent.log"))
        let plist = try decodePlist(try withLog.plist())

        #expect(plist["StandardOutPath"] as? String == "/tmp/agent.log")
        #expect(plist["StandardErrorPath"] as? String == "/tmp/agent.log")

        // Without a path the keys must be absent rather than empty, which launchd rejects.
        let withoutLog = try decodePlist(
            try LaunchdJob.daemon(
                label: "com.holyblocker.daemon", executable: executable, arguments: ["run"]
            ).plist())
        #expect(withoutLog["StandardOutPath"] == nil)
    }

    @Test("comes back after a crash and after a login")
    func staysRunning() throws {
        let job = LaunchdJob.agent(
            label: "com.holyblocker.agent", executable: executable, arguments: ["agent"],
            home: URL(fileURLWithPath: "/Users/tester"), uid: 501)
        let plist = try decodePlist(try job.plist())

        #expect(plist["RunAtLoad"] as? Bool == true)
        #expect(plist["KeepAlive"] as? Bool == true)
        #expect(plist["Label"] as? String == "com.holyblocker.agent")
    }
}

// MARK: - Code signing

@Suite("CodeSigning.parseIdentity")
struct ParseSigningIdentityTests {
    /// Verbatim `codesign -dvv` output for an ad-hoc signed bundle. The tool writes to stderr.
    private let adhocOutput = """
        Executable=/tmp/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd
        Identifier=com.holyblocker.daemon
        Format=app bundle with Mach-O thin (arm64)
        CodeDirectory v=20400 size=1234 flags=0x2(adhoc) hashes=32+7 location=embedded
        Signature=adhoc
        Info.plist entries=9
        TeamIdentifier=not set
        Sealed Resources version=2 rules=13 files=0
        """

    private let signedOutput = """
        Executable=/Applications/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd
        Identifier=com.holyblocker.daemon
        Format=app bundle with Mach-O thin (arm64)
        CodeDirectory v=20400 size=1234 flags=0x0(none) hashes=32+7 location=embedded
        Signature size=4785
        Authority=Holy Blocker Development
        Signed Time=4 Aug 2026 at 10:00:00
        Info.plist entries=9
        TeamIdentifier=not set
        """

    @Test("recognises an ad-hoc signature, which is the grant-losing case")
    func adhoc() {
        // An ad-hoc identity is derived from the binary's cdhash, so it changes on every build and
        // every TCC grant made against it dies with the next compile.
        #expect(CodeSigning.parseIdentity(adhocOutput) == .adhoc)
    }

    @Test("reads the authority of a real signature")
    func signed() {
        #expect(CodeSigning.parseIdentity(signedOutput) == .signed(authority: "Holy Blocker Development"))
    }

    @Test("recognises an unsigned binary")
    func unsigned() {
        #expect(
            CodeSigning.parseIdentity("/tmp/thing: code object is not signed at all") == .unsigned)
        #expect(CodeSigning.parseIdentity("") == .unsigned)
    }

    @Test("reports whether a grant made against this signature would survive a rebuild")
    func stability() {
        // The single question module 0 exists to answer.
        #expect(SigningIdentity.signed(authority: "Holy Blocker Development").isStable)
        #expect(!SigningIdentity.adhoc.isStable)
        #expect(!SigningIdentity.unsigned.isStable)
    }
}

@Suite("CodeSigning.codePath")
struct CodePathTests {
    private let bundle = URL(fileURLWithPath: "/Applications/HolyBlockerDaemon.app")
    private let executable = URL(
        fileURLWithPath: "/Applications/HolyBlockerDaemon.app/Contents/MacOS/holy-blocker-macd")

    @Test("inspects the bundle when running from one")
    func bundled() {
        #expect(
            CodeSigning.codePath(
                bundleIdentifier: "com.holyblocker.daemon", bundleURL: bundle,
                executableURL: executable) == bundle)
    }

    @Test("inspects the executable when there is no bundle")
    func bare() {
        // Bundle.main.bundleURL for a bare binary is its *containing directory*, and codesign
        // rejects a plain directory with "bundle format unrecognized".
        let directory = URL(fileURLWithPath: "/tmp/.build/debug")
        let binary = directory.appendingPathComponent("holy-blocker-macd")

        #expect(
            CodeSigning.codePath(
                bundleIdentifier: nil, bundleURL: directory, executableURL: binary) == binary)
    }

    @Test("falls back to the bundle URL when the executable is unknown")
    func noExecutable() {
        let directory = URL(fileURLWithPath: "/tmp/.build/debug")

        #expect(
            CodeSigning.codePath(
                bundleIdentifier: nil, bundleURL: directory, executableURL: nil) == directory)
    }
}

@Suite("CodeSigning invocation")
struct CodeSigningInvocationTests {
    @Test("signs the bundle, not the executable inside it")
    func signsTheBundle() throws {
        // Signing the inner Mach-O leaves the bundle's seal stale, and TCC reads the bundle.
        let runner = FakeCommandRunner()
        let bundle = URL(fileURLWithPath: "/tmp/HolyBlockerDaemon.app")

        try CodeSigning(runner: runner).sign(bundle: bundle, identity: "Holy Blocker Development")

        #expect(runner.invocations.count == 1)
        #expect(runner.invocations[0].executable == SystemTool.codesign)
        #expect(runner.invocations[0].arguments.contains(bundle.path))
        #expect(!runner.invocations[0].arguments.contains { $0.hasSuffix("holy-blocker-macd") })
    }

    @Test("replaces any existing signature")
    func forcesReplacement() throws {
        // Without --force a re-signed bundle keeps the old seal and codesign exits non-zero.
        let runner = FakeCommandRunner()

        try CodeSigning(runner: runner).sign(
            bundle: URL(fileURLWithPath: "/tmp/HolyBlockerDaemon.app"), identity: "-")

        #expect(runner.invocations[0].arguments.contains("--force"))
    }

    @Test("surfaces a signing failure instead of leaving an unsigned bundle behind")
    func propagatesFailure() {
        let runner = FakeCommandRunner(
            defaultResult: CommandResult(exitCode: 1, standardError: "no identity found"))

        #expect(throws: CodeSigningError.self) {
            try CodeSigning(runner: runner).sign(
                bundle: URL(fileURLWithPath: "/tmp/HolyBlockerDaemon.app"), identity: "missing")
        }
    }

    @Test("reads back the identity actually applied")
    func readsBackIdentity() throws {
        let runner = FakeCommandRunner(
            defaultResult: CommandResult(exitCode: 0, standardError: "Signature=adhoc"))

        let identity = try CodeSigning(runner: runner).identity(
            of: URL(fileURLWithPath: "/tmp/HolyBlockerDaemon.app"))

        // codesign writes its report to stderr, which is easy to get wrong and yields "unsigned"
        // for every bundle if you read stdout.
        #expect(identity == .adhoc)
    }
}
