import Foundation

/// The release and development certificates in the login keychain, compared by fingerprint.
/// Two names over one certificate would let a development build carry the release identity.
public enum SigningIdentities {
    public enum Distinction: Equatable, Sendable {
        case distinct
        case same
        case missing([String])
    }

    /// `security find-identity -v -p codesigning` prints `  N) <SHA-1> "<name>"` per identity.
    public static func parse(_ output: String) -> [String: String] {
        var found: [String: String] = [:]
        for line in output.split(separator: "\n") {
            let parts = line.split(separator: " ", maxSplits: 2, omittingEmptySubsequences: true)
            guard parts.count == 3, parts[0].hasSuffix(")"), parts[1].count == 40,
                parts[1].allSatisfy(\.isHexDigit),
                parts[2].hasPrefix("\""), parts[2].hasSuffix("\""), parts[2].count >= 2
            else { continue }
            found[String(parts[2].dropFirst().dropLast())] = String(parts[1])
        }
        return found
    }

    public static func distinction(in output: String) -> Distinction {
        let found = parse(output)
        let release = BundleIdentity.releaseSigningIdentity
        let development = BundleIdentity.developmentSigningIdentity
        let missing = [release, development].filter { found[$0] == nil }
        guard missing.isEmpty else { return .missing(missing) }
        return found[release] == found[development] ? .same : .distinct
    }

    public static func inspect(runner: CommandRunner) throws -> Distinction {
        let result = try runner.run(
            SystemTool.security, ["find-identity", "-v", "-p", "codesigning"])
        return distinction(in: result.standardOutput)
    }
}
