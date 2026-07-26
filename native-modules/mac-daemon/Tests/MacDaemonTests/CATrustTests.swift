import Foundation
import Testing

@testable import MacDaemon

private let certificatePath = URL(fileURLWithPath: "/opt/holy blocker/ca/ca.crt")

private func makeTrust(_ runner: FakeCommandRunner) -> CATrust {
    CATrust(runner: runner, certificatePath: certificatePath, commonName: "Holy Blocker Local CA")
}

@Suite("CATrust.state")
struct CATrustStateTests {
    @Test("reports absent when the certificate is not in the keychain")
    func absent() throws {
        let runner = FakeCommandRunner()
        // security(1) exits 44 (errSecItemNotFound) when no matching certificate exists. This is
        // the normal first-run condition and must not surface as an error.
        runner.stub(
            subcommand: "find-certificate",
            result: CommandResult(
                exitCode: 44,
                standardError: "security: SecKeychainSearchCopyNext: The specified item could "
                    + "not be found in the keychain.\n"))

        #expect(try makeTrust(runner).state() == .absent)
    }

    @Test("reports installedAndTrusted for an empty trust-settings array")
    func trusted() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 0))
        // Verbatim `security dump-trust-settings -d` output captured on macOS 26.5 immediately
        // after a successful `add-trusted-cert -d -r trustRoot`. An empty trust-settings array is
        // Apple's encoding of "trusted as a root for all purposes" — every one of the 157
        // built-in system roots is represented exactly this way. Reading it as untrusted makes
        // install() re-run on every daemon start and re-prompt for admin credentials.
        runner.stub(
            subcommand: "dump-trust-settings",
            result: CommandResult(
                exitCode: 0,
                standardOutput: """
                    Number of trusted certs = 1
                    Cert 0: Holy Blocker Local CA
                       Number of trust settings : 0

                    """))

        #expect(try makeTrust(runner).state() == .installedAndTrusted)
    }

    @Test("finds the certificate among many trusted roots")
    func trustedAmongManyRoots() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 0))
        // Shape of the system domain, which holds every shipped root in this same form.
        runner.stub(
            subcommand: "dump-trust-settings",
            result: CommandResult(
                exitCode: 0,
                standardOutput: """
                    Number of trusted certs = 3
                    Cert 0: DigiCert TLS ECC P384 Root G5
                       Number of trust settings : 0
                    Cert 1: Holy Blocker Local CA
                       Number of trust settings : 0
                    Cert 2: XRamp Global Certification Authority
                       Number of trust settings : 0

                    """))

        #expect(try makeTrust(runner).state() == .installedAndTrusted)
    }

    @Test("reports installedUntrusted when present but absent from trust settings")
    func installedButNotTrusted() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 0))
        runner.stub(
            subcommand: "dump-trust-settings",
            result: CommandResult(
                exitCode: 0,
                standardOutput: """
                    Number of trusted certs = 1
                    Cert 0: Some Other Root
                       Number of trust settings : 0

                    """))

        #expect(try makeTrust(runner).state() == .installedUntrusted)
    }

    @Test("treats an explicit deny result as untrusted, not trusted")
    func deniedTrustSetting() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 0))
        // NOTE: unlike the other fixtures here, the exact rendering of a deny result is NOT
        // captured from a real run — reproducing one requires denying a root machine-wide. The
        // parser therefore matches "deny" case-insensitively anywhere in the cert's block, which
        // covers both `Result = deny` and `kSecTrustSettingsResultDeny`. Verify against real
        // output before relying on this branch.
        runner.stub(
            subcommand: "dump-trust-settings",
            result: CommandResult(
                exitCode: 0,
                standardOutput: """
                    Number of trusted certs = 1
                    Cert 0: Holy Blocker Local CA
                       Number of trust settings : 1
                       Trust Setting 0:
                          Result = kSecTrustSettingsResultDeny

                    """))

        // Matching on the name alone would call a explicitly-distrusted CA trusted and skip the
        // install, leaving every proxied HTTPS connection broken with no visible cause.
        #expect(try makeTrust(runner).state() == .installedUntrusted)
    }

    @Test("reports installedUntrusted when the trust-settings store is empty")
    func emptyTrustSettings() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 0))
        // security(1) exits 1 with this message when the admin store has no entries at all.
        runner.stub(
            subcommand: "dump-trust-settings",
            result: CommandResult(
                exitCode: 1, standardError: "SecTrustSettingsCopyCertificates: No Trust Settings "
                    + "were found.\n"))

        #expect(try makeTrust(runner).state() == .installedUntrusted)
    }

    @Test("surfaces an unexpected find-certificate failure as an error")
    func unexpectedFailure() throws {
        let runner = FakeCommandRunner()
        runner.stub(
            subcommand: "find-certificate",
            result: CommandResult(exitCode: 1, standardError: "security: unable to open keychain"))

        #expect(throws: CATrustError.self) { try makeTrust(runner).state() }
    }
}

@Suite("CATrust.install")
struct CATrustInstallTests {
    @Test("builds the add-trusted-cert argument vector for the System keychain")
    func argumentVector() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 44))

        try makeTrust(runner).install()

        let install = try #require(
            runner.invocations.first { $0.arguments.contains("add-trusted-cert") })
        #expect(install.executable == "/usr/bin/security")
        #expect(
            install.arguments == [
                "add-trusted-cert",
                "-d",  // admin (machine-wide) trust domain, not the per-user store
                "-r", "trustRoot",
                "-k", "/Library/Keychains/System.keychain",
                "/opt/holy blocker/ca/ca.crt",
            ])
    }

    @Test("passes a path containing spaces as one argument, unquoted")
    func pathWithSpaces() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 44))

        try makeTrust(runner).install()

        let install = try #require(
            runner.invocations.first { $0.arguments.contains("add-trusted-cert") })
        // Process takes an argv array, so shell quoting would become part of the literal path.
        #expect(install.arguments.last == "/opt/holy blocker/ca/ca.crt")
        #expect(install.arguments.last?.contains("\"") == false)
        #expect(install.arguments.last?.contains("\\") == false)
    }

    @Test("is a no-op when the certificate is already installed and trusted")
    func idempotent() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 0))
        runner.stub(
            subcommand: "dump-trust-settings",
            result: CommandResult(
                exitCode: 0,
                standardOutput: "Cert 0: Holy Blocker Local CA\n   Number of trust settings : 0\n"))

        try makeTrust(runner).install()

        // Re-running add-trusted-cert would pop a second admin authorization prompt on every
        // daemon start.
        #expect(!runner.invocations.contains { $0.arguments.contains("add-trusted-cert") })
    }

    @Test("reinstalls when the certificate is present but not trusted")
    func repairsUntrusted() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 0))
        runner.stub(
            subcommand: "dump-trust-settings",
            result: CommandResult(exitCode: 0, standardOutput: "Cert 0: Some Other Root\n"))

        try makeTrust(runner).install()

        #expect(runner.invocations.contains { $0.arguments.contains("add-trusted-cert") })
    }

    @Test("throws when the user cancels the admin authorization prompt")
    func authorizationCancelled() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 44))
        runner.stub(
            subcommand: "add-trusted-cert",
            result: CommandResult(
                exitCode: 1,
                standardError: "SecTrustSettingsSetTrustSettings: The authorization was "
                    + "cancelled by the user.\n"))

        #expect(throws: CATrustError.self) { try makeTrust(runner).install() }
    }
}

@Suite("CATrust.uninstall")
struct CATrustUninstallTests {
    @Test("removes the trust setting and deletes the certificate from the System keychain")
    func removesBoth() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 0))
        runner.stub(
            subcommand: "dump-trust-settings",
            result: CommandResult(
                exitCode: 0,
                standardOutput: "Cert 0: Holy Blocker Local CA\n   Number of trust settings : 0\n"))

        try makeTrust(runner).uninstall()

        let remove = try #require(
            runner.invocations.first { $0.arguments.contains("remove-trusted-cert") })
        #expect(remove.arguments == ["remove-trusted-cert", "-d", "/opt/holy blocker/ca/ca.crt"])

        // Removing the trust setting alone leaves the certificate sitting in the System keychain.
        let delete = try #require(
            runner.invocations.first { $0.arguments.contains("delete-certificate") })
        #expect(
            delete.arguments == [
                "delete-certificate", "-c", "Holy Blocker Local CA",
                "/Library/Keychains/System.keychain",
            ])
    }

    @Test("is a no-op when nothing is installed")
    func nothingInstalled() throws {
        let runner = FakeCommandRunner()
        runner.stub(subcommand: "find-certificate", result: CommandResult(exitCode: 44))

        try makeTrust(runner).uninstall()

        #expect(!runner.invocations.contains { $0.arguments.contains("remove-trusted-cert") })
        #expect(!runner.invocations.contains { $0.arguments.contains("delete-certificate") })
    }
}
