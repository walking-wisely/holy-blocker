import Foundation

/// Whether the local root CA is installed in the System keychain and trusted.
public enum CATrustState: Equatable, Sendable {
    case absent
    case installedAndTrusted
    /// Present in the keychain but missing from trust settings, or explicitly denied. Leaf
    /// certificates from the proxy will not validate in this state.
    case installedUntrusted
}

public enum CATrustError: Error, Equatable {
    case queryFailed(exitCode: Int32, message: String)
    case installFailed(exitCode: Int32, message: String)
    case uninstallFailed(exitCode: Int32, message: String)
}

/// Installs the proxy's local root CA into the System keychain as a trusted root.
///
/// Writing the System keychain requires administrator authorization, which is deliberate: it is
/// the same admin credential the accountability partner holds, so the protected user cannot
/// quietly remove the CA. See the tamper-resistance section of docs/decisions/content-interception.md.
///
/// Note that Firefox keeps its own NSS trust store and ignores everything done here.
public struct CATrust {
    /// The machine-wide trust domain. `security(1)` calls this the admin domain; without `-d` the
    /// settings land in the per-user store, which the protected user can edit unprivileged.
    private static let adminTrustDomainFlag = "-d"
    /// `security(1)` exits with errSecItemNotFound when no matching certificate exists. This is
    /// the normal first-run condition, not a failure.
    private static let errSecItemNotFound: Int32 = 44
    private static let systemKeychain = "/Library/Keychains/System.keychain"
    /// `security dump-trust-settings` prints this to stderr and exits 1 when the domain has no
    /// entries — also a normal state, not a failure.
    private static let noTrustSettingsMarker = "No Trust Settings were found"
    /// The `-r` result passed to add-trusted-cert. See `security(1)`.
    private static let trustRootResult = "trustRoot"
    /// Each certificate in `dump-trust-settings` output starts a block with this prefix.
    private static let certBlockPrefix = "Cert "
    /// Lowercased substring identifying an explicit deny result within a cert's block.
    private static let denyResultMarker = "deny"

    private let runner: CommandRunner
    private let certificatePath: URL
    private let commonName: String

    public init(runner: CommandRunner, certificatePath: URL, commonName: String) {
        self.runner = runner
        self.certificatePath = certificatePath
        self.commonName = commonName
    }

    public func state() throws -> CATrustState {
        let found = try runner.run(
            SystemTool.security,
            ["find-certificate", "-c", commonName, Self.systemKeychain])

        if found.exitCode == Self.errSecItemNotFound { return .absent }
        guard found.succeeded else {
            throw CATrustError.queryFailed(
                exitCode: found.exitCode, message: found.standardError)
        }

        let dump = try runner.run(
            SystemTool.security, ["dump-trust-settings", Self.adminTrustDomainFlag])

        if !dump.succeeded {
            guard dump.standardError.contains(Self.noTrustSettingsMarker) else {
                throw CATrustError.queryFailed(
                    exitCode: dump.exitCode, message: dump.standardError)
            }
            return .installedUntrusted
        }

        return Self.isTrusted(commonName: commonName, inTrustSettings: dump.standardOutput)
            ? .installedAndTrusted : .installedUntrusted
    }

    public func install() throws {
        // Re-running add-trusted-cert would raise a fresh admin authorization prompt on every
        // daemon start, so only act when the state actually calls for it.
        guard try state() != .installedAndTrusted else { return }

        let result = try runner.run(
            SystemTool.security,
            [
                "add-trusted-cert",
                Self.adminTrustDomainFlag,
                "-r", Self.trustRootResult,
                "-k", Self.systemKeychain,
                certificatePath.path,
            ])

        guard result.succeeded else {
            throw CATrustError.installFailed(
                exitCode: result.exitCode, message: result.standardError)
        }
    }

    public func uninstall() throws {
        guard try state() != .absent else { return }

        let untrust = try runner.run(
            SystemTool.security,
            ["remove-trusted-cert", Self.adminTrustDomainFlag, certificatePath.path])
        guard untrust.succeeded else {
            throw CATrustError.uninstallFailed(
                exitCode: untrust.exitCode, message: untrust.standardError)
        }

        // Dropping the trust setting leaves the certificate itself in the keychain; delete it too
        // so uninstall leaves no residue.
        let delete = try runner.run(
            SystemTool.security,
            ["delete-certificate", "-c", commonName, Self.systemKeychain])
        guard delete.succeeded || delete.exitCode == Self.errSecItemNotFound else {
            throw CATrustError.uninstallFailed(
                exitCode: delete.exitCode, message: delete.standardError)
        }
    }

    /// Decides whether `dump-trust-settings` output shows the named certificate as trusted.
    ///
    /// Appearing in the domain's list *is* the trust signal. An empty trust-settings array is
    /// Apple's encoding of "trusted as a root for all purposes" — the 157 built-in roots in the
    /// system domain are all rendered exactly that way:
    ///
    ///     Cert 0: DigiCert TLS ECC P384 Root G5
    ///        Number of trust settings : 0
    ///
    /// An earlier version of this function required an explicit `Result = trustRoot` line, which
    /// no real output ever contains. That made `state()` report `installedUntrusted` immediately
    /// after a successful install, so `install()` re-ran `add-trusted-cert` on every daemon start
    /// and raised a fresh admin authorization prompt each time.
    ///
    /// A cert can still be listed with an explicit deny, which must not read as trusted — matched
    /// case-insensitively so both `Result = deny` and `kSecTrustSettingsResultDeny` are caught.
    /// That branch is not yet confirmed against real output; see the note in CATrustTests.
    static func isTrusted(commonName: String, inTrustSettings output: String) -> Bool {
        var inMatchingCert = false
        var found = false

        for line in output.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if trimmed.hasPrefix(certBlockPrefix) {
                // A new cert block ends the previous one.
                inMatchingCert = trimmed.hasSuffix(": \(commonName)")
                found = found || inMatchingCert
                continue
            }
            if inMatchingCert && trimmed.lowercased().contains(denyResultMarker) {
                return false
            }
        }

        return found
    }
}
