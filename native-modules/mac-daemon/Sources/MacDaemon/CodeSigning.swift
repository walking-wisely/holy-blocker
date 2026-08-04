import Foundation

/// The signature a bundle currently carries, judged by the only question that matters: would a
/// TCC grant made against it survive the next build?
public enum SigningIdentity: Equatable, Sendable {
    case unsigned
    /// Signed with no certificate. The identity is derived from the binary's `cdhash`, so it
    /// changes on every rebuild and takes every grant with it.
    case adhoc
    case signed(authority: String)

    /// Whether a grant made against this signature outlives a recompile.
    public var isStable: Bool {
        if case .signed = self { return true }
        return false
    }
}

public enum CodeSigningError: Error, Equatable {
    case signingFailed(String)
    case inspectionFailed(String)
}

/// `codesign(1)`, narrowed to signing a bundle and reading back what was applied.
public struct CodeSigning {
    /// `codesign` says this, and exits non-zero, for a binary carrying no signature at all.
    private static let unsignedMarker = "code object is not signed at all"
    private static let adhocMarker = "Signature=adhoc"
    private static let authorityPrefix = "Authority="

    private let runner: CommandRunner

    public init(runner: CommandRunner = SystemCommandRunner()) {
        self.runner = runner
    }

    /// Sign the bundle. `identity` is a keychain identity name, or `-` for ad-hoc.
    ///
    /// The **bundle** is signed, never the main executable inside it: signing the inner Mach-O
    /// leaves the bundle's seal stale, and TCC reads the bundle.
    ///
    /// `nestedCode` is the exception — embedded dylibs carry their own signatures and must be
    /// signed **before** the bundle, because signing the bundle seals whatever its contents are at
    /// that moment. Signed in the other order, the seal describes an unsigned dylib while the
    /// dylib is signed, and dyld refuses the mismatch at launch rather than at build time.
    /// `--deep` would do this in one call and is documented by Apple as not to be used: it signs
    /// whatever it happens to find, which is the opposite of knowing what shipped.
    public func sign(bundle: URL, identity: String, nestedCode: [URL] = []) throws {
        for code in nestedCode {
            try sign(path: code.path, identity: identity)
        }
        try sign(path: bundle.path, identity: identity)
    }

    private func sign(path: String, identity: String) throws {
        // --force replaces an existing signature; without it a re-sign keeps the old seal and
        // codesign exits non-zero.
        let result = try runner.run(SystemTool.codesign, ["--force", "--sign", identity, path])
        guard result.succeeded else {
            throw CodeSigningError.signingFailed(result.standardError)
        }
    }

    public func identity(of bundle: URL) throws -> SigningIdentity {
        let result = try runner.run(SystemTool.codesign, ["-dvv", bundle.path])
        // codesign writes its report to **stderr**. Reading stdout returns "unsigned" for every
        // bundle on the machine.
        let output = result.standardError + result.standardOutput

        guard result.succeeded || output.contains(Self.unsignedMarker) else {
            throw CodeSigningError.inspectionFailed(output)
        }
        return Self.parseIdentity(output)
    }

    /// What to hand `codesign` to inspect a running process's own signature.
    ///
    /// `Bundle.main.bundleURL` is the bundle only when there *is* one; for a bare executable it is
    /// the containing directory, which codesign rejects as "bundle format unrecognized".
    public static func codePath(
        bundleIdentifier: String?, bundleURL: URL, executableURL: URL?
    ) -> URL {
        guard bundleIdentifier == nil, let executableURL else { return bundleURL }
        return executableURL
    }

    /// The signature of the currently running code, bundled or not.
    public static var currentCodePath: URL {
        codePath(
            bundleIdentifier: Bundle.main.bundleIdentifier,
            bundleURL: Bundle.main.bundleURL,
            executableURL: Bundle.main.executableURL)
    }

    public static func parseIdentity(_ output: String) -> SigningIdentity {
        if output.contains(unsignedMarker) { return .unsigned }
        if output.contains(adhocMarker) { return .adhoc }

        for line in output.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard trimmed.hasPrefix(authorityPrefix) else { continue }
            // The first Authority line is the leaf; the rest are the chain up to the root.
            return .signed(authority: String(trimmed.dropFirst(authorityPrefix.count)))
        }
        return .unsigned
    }
}
