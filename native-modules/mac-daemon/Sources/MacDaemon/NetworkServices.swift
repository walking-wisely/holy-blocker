import Foundation

/// A macOS network service (Wi-Fi, Ethernet, a VPN interface, …).
///
/// Proxy settings on macOS are per service, not global, so every configuration operation
/// iterates these. See `networksetup(8)`.
public struct NetworkService: Equatable, Sendable {
    public let name: String
    public let isEnabled: Bool

    public init(name: String, isEnabled: Bool) {
        self.name = name
        self.isEnabled = isEnabled
    }
}

/// The proxy state of one service, as reported by `-getwebproxy` / `-getsecurewebproxy`.
public struct ProxySetting: Equatable, Codable, Sendable {
    public let isEnabled: Bool
    public let server: String
    public let port: Int

    public init(isEnabled: Bool, server: String, port: Int) {
        self.isEnabled = isEnabled
        self.server = server
        self.port = port
    }
}

/// Pure parsers for `networksetup` output.
///
/// No I/O here by design — callers run the tool and pass the text in, which keeps every parsing
/// hazard in this file unit-testable against captured real output.
public enum NetworkServices {
    /// networksetup(8) prefixes this line to `-listallnetworkservices` output.
    private static let listHeaderPrefix = "An asterisk (*) denotes"
    /// A leading `*` marks a disabled service and is not part of the name.
    private static let disabledMarker: Character = "*"
    /// networksetup(8) reports an absent bypass list in prose rather than as empty output.
    private static let noBypassDomainsMarker = "aren't any bypass domains"

    public static func parseServiceList(_ output: String) -> [NetworkService] {
        output.split(separator: "\n", omittingEmptySubsequences: false)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty && !$0.hasPrefix(listHeaderPrefix) }
            .map { line in
                if line.first == disabledMarker {
                    return NetworkService(name: String(line.dropFirst()), isEnabled: false)
                }
                return NetworkService(name: line, isEnabled: true)
            }
    }

    public static func parseProxySetting(_ output: String) -> ProxySetting? {
        let fields = keyedFields(output)
        guard let enabled = fields["Enabled"], let rawPort = fields["Port"] else { return nil }
        // A port we cannot parse must fail the whole read: silently substituting 0 would make a
        // restore write back a bogus setting instead of the one we found.
        guard let port = Int(rawPort) else { return nil }

        return ProxySetting(
            isEnabled: enabled.lowercased() == "yes",
            server: fields["Server"] ?? "",
            port: port
        )
    }

    public static func parseBypassDomains(_ output: String) -> [String] {
        if output.contains(noBypassDomainsMarker) { return [] }
        return output.split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
    }

    /// Splits `Key: value` report lines into a dictionary, ignoring anything without a colon.
    private static func keyedFields(_ output: String) -> [String: String] {
        var fields: [String: String] = [:]
        for line in output.split(separator: "\n") {
            guard let separator = line.firstIndex(of: ":") else { continue }
            let key = line[line.startIndex..<separator].trimmingCharacters(in: .whitespaces)
            let value = line[line.index(after: separator)...]
                .trimmingCharacters(in: .whitespaces)
            fields[key] = value
        }
        return fields
    }
}
