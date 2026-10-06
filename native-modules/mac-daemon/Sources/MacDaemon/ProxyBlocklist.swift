import Foundation

public enum ProxyBlocklistFailure: Equatable, Sendable {
    case missing
    case rejected
}

public enum ProxyBlocklistPolicy: Equatable, Sendable {
    case required
    case optional

    public static let allowMissingVariable = "HOLY_BLOCKER_ALLOW_NO_BLOCKLIST"

    public init(environment: [String: String]) {
        self = environment[Self.allowMissingVariable] == "1" ? .optional : .required
    }

    public func arguments(for source: ProxyBlocklistSource) -> [String]? {
        switch (source, self) {
        case (.configured(let arguments), _): return arguments
        case (.unavailable, .optional): return []
        case (.unavailable, .required): return nil
        }
    }
}

public enum ProxyBlocklistStageError: Error, Equatable {
    case noCurrentSlot(URL)
}

public enum ProxyBlocklistSource: Equatable, Sendable {
    case configured(arguments: [String])
    case unavailable(ProxyBlocklistFailure)
}

/// The signed domain list sealed inside the bundle, and the proxy flags that point at it.
///
/// Layout under `Contents/Resources/blocklist`: `current/` and `previous/` slots as the
/// `domain-blocklist` CLI writes them, and `keys/<key id>.pub` holding raw 32-byte Ed25519 keys.
/// The proxy exits when handed a directory it cannot load, so a list that is absent or has no
/// usable key yields no flags at all; `ProxyBlocklistPolicy` decides whether that may run.
public enum ProxyBlocklist {
    public static let directoryName = "blocklist"
    private static let keyDirectoryName = "keys"
    private static let keySuffix = ".pub"
    private static let ed25519PublicKeyLength = 32
    private static let slots = ["current", "previous"]

    public static func resolve(
        resources: URL?, fileManager: FileManager = .default
    ) -> ProxyBlocklistSource {
        guard let resources else { return .unavailable(.missing) }
        let list = resources.appendingPathComponent(directoryName)
        var isDirectory: ObjCBool = false
        guard
            fileManager.fileExists(
                atPath: list.appendingPathComponent("current").path, isDirectory: &isDirectory),
            isDirectory.boolValue
        else { return .unavailable(.missing) }

        let keys = trustedKeys(in: list.appendingPathComponent(keyDirectoryName), fileManager)
        guard !keys.isEmpty else { return .unavailable(.rejected) }

        var arguments = ["--blocklist-dir", list.path]
        for key in keys {
            arguments += ["--blocklist-key", "\(key.id):\(key.hex)"]
        }
        return .configured(arguments: arguments)
    }

    /// Copies a built artifact directory and its public keys into the layout `resolve` reads.
    public static func stage(
        artifactDirectory: URL, keyDirectory: URL, into resources: URL,
        fileManager: FileManager = .default
    ) throws {
        guard
            fileManager.fileExists(
                atPath: artifactDirectory.appendingPathComponent("current").path)
        else { throw ProxyBlocklistStageError.noCurrentSlot(artifactDirectory) }
        let list = resources.appendingPathComponent(directoryName)
        if fileManager.fileExists(atPath: list.path) {
            try fileManager.removeItem(at: list)
        }
        try fileManager.createDirectory(at: list, withIntermediateDirectories: true)
        for slot in slots {
            let source = artifactDirectory.appendingPathComponent(slot)
            guard fileManager.fileExists(atPath: source.path) else { continue }
            try fileManager.copyItem(at: source, to: list.appendingPathComponent(slot))
        }
        let keys = list.appendingPathComponent(keyDirectoryName)
        try fileManager.createDirectory(at: keys, withIntermediateDirectories: true)
        for name in try fileManager.contentsOfDirectory(atPath: keyDirectory.path)
        where name.hasSuffix(keySuffix) {
            try fileManager.copyItem(
                at: keyDirectory.appendingPathComponent(name),
                to: keys.appendingPathComponent(name))
        }
    }

    private static func trustedKeys(
        in directory: URL, _ fileManager: FileManager
    ) -> [(id: String, hex: String)] {
        let names = (try? fileManager.contentsOfDirectory(atPath: directory.path)) ?? []
        return names.sorted().compactMap { name in
            guard name.hasSuffix(keySuffix) else { return nil }
            let id = String(name.dropLast(keySuffix.count))
            guard !id.isEmpty, !id.contains(":"),
                let bytes = try? Data(contentsOf: directory.appendingPathComponent(name)),
                bytes.count == ed25519PublicKeyLength
            else { return nil }
            return (id, bytes.map { String(format: "%02x", $0) }.joined())
        }
    }
}
