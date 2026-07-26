import Foundation

/// Whether Firefox is configured to trust roots from the operating system store.
public enum FirefoxTrustState: Equatable, Sendable {
    case absent
    case enabled
    /// Explicitly opted out, or the policy engine is switched off so the key is never read.
    case disabled
}

/// The Firefox managed-preferences domain.
///
/// Reads and writes go through `CFPreferences` rather than the plist file, because that is the API
/// Firefox itself reads with. Writing the file directly races `cfprefsd`, which caches the domain
/// and can serve — or write back — a stale copy.
public protocol FirefoxPolicyStore {
    func read() throws -> [String: Any]
    func write(_ policy: [String: Any]) throws
    func clear() throws
}

public enum FirefoxTrustError: Error, Equatable {
    case writeFailed(domain: String)
}

/// Makes Firefox honour the root CA installed in the System keychain.
///
/// Firefox keeps its own NSS trust store and ignores the System keychain by default, so
/// `CATrust` alone leaves every Firefox user seeing certificate errors. The `ImportEnterpriseRoots`
/// policy flips `security.enterprise_roots.enabled`, which makes Firefox read the platform store.
///
/// Mozilla documents two delivery mechanisms on macOS. This uses the managed-preferences domain
/// rather than `Firefox.app/Contents/Resources/distribution/policies.json` for two reasons: writing
/// inside the bundle breaks the notarized app's signature seal, and every Firefox update replaces
/// the bundle and would silently drop the policy. The preferences domain is admin-writable, which
/// is also what the tamper model wants — the protected user cannot revoke it unprivileged.
public struct FirefoxTrust {
    /// Policy keys, per the Firefox enterprise policy reference. See the plan's reference documents.
    private static let policyEngineKey = "EnterprisePoliciesEnabled"
    private static let certificatesKey = "Certificates"
    private static let importEnterpriseRootsKey = "ImportEnterpriseRoots"

    private let store: FirefoxPolicyStore

    public init(store: FirefoxPolicyStore) {
        self.store = store
    }

    public func state() throws -> FirefoxTrustState {
        Self.readState(try store.read())
    }

    public func install() throws {
        let current = try store.read()
        // Rewriting an already-correct domain would churn a file other tooling may be watching.
        guard Self.readState(current) != .enabled else { return }
        try store.write(Self.enablingEnterpriseRoots(in: current))
    }

    public func uninstall() throws {
        let current = try store.read()
        guard current[Self.certificatesKey] != nil else { return }

        let remaining = Self.removingEnterpriseRoots(from: current)
        if remaining.isEmpty {
            try store.clear()
        } else {
            try store.write(remaining)
        }
    }

    // MARK: - Pure policy transforms

    static func readState(_ policy: [String: Any]) -> FirefoxTrustState {
        let certificates = policy[certificatesKey] as? [String: Any]
        guard let imports = certificates?[importEnterpriseRootsKey] as? Bool else { return .absent }
        guard imports else { return .disabled }

        // The key is meaningless while the engine is off; reporting `.enabled` there would claim
        // coverage the user does not have.
        if let engine = policy[policyEngineKey] as? Bool, !engine { return .disabled }
        return .enabled
    }

    static func enablingEnterpriseRoots(in policy: [String: Any]) -> [String: Any] {
        var updated = policy
        var certificates = policy[certificatesKey] as? [String: Any] ?? [:]
        certificates[importEnterpriseRootsKey] = true
        updated[certificatesKey] = certificates
        updated[policyEngineKey] = true
        return updated
    }

    static func removingEnterpriseRoots(from policy: [String: Any]) -> [String: Any] {
        var updated = policy
        guard var certificates = policy[certificatesKey] as? [String: Any] else { return updated }

        certificates.removeValue(forKey: importEnterpriseRootsKey)
        if certificates.isEmpty {
            updated.removeValue(forKey: certificatesKey)
        } else {
            updated[certificatesKey] = certificates
        }

        // The engine flag is only ours to remove if no other policy still depends on it.
        if updated.keys.allSatisfy({ $0 == policyEngineKey }) {
            updated.removeValue(forKey: policyEngineKey)
        }
        return updated
    }
}

/// `CFPreferences`-backed store for `/Library/Preferences/org.mozilla.firefox.plist`.
///
/// `kCFPreferencesAnyUser` + `kCFPreferencesAnyHost` is the pair that maps to that path; using
/// `kCFPreferencesCurrentHost` instead would land in `/Library/Preferences/ByHost/` under a
/// hardware UUID. Writing requires root.
public struct CFPreferencesPolicyStore: FirefoxPolicyStore {
    public static let firefoxApplicationID = "org.mozilla.firefox"

    private let applicationID: CFString

    public init(applicationID: String = firefoxApplicationID) {
        self.applicationID = applicationID as CFString
    }

    public func read() throws -> [String: Any] {
        guard
            let keys = CFPreferencesCopyKeyList(
                applicationID, kCFPreferencesAnyUser, kCFPreferencesAnyHost) as? [String]
        else { return [:] }

        var policy: [String: Any] = [:]
        for key in keys {
            if let value = CFPreferencesCopyValue(
                key as CFString, applicationID, kCFPreferencesAnyUser, kCFPreferencesAnyHost)
            {
                policy[key] = value
            }
        }
        return policy
    }

    public func write(_ policy: [String: Any]) throws {
        // Clear first so keys dropped by the caller do not survive as stale entries.
        try removeAllKeys()
        for (key, value) in policy {
            CFPreferencesSetValue(
                key as CFString, value as CFPropertyList, applicationID,
                kCFPreferencesAnyUser, kCFPreferencesAnyHost)
        }
        try synchronize()
    }

    public func clear() throws {
        try removeAllKeys()
        try synchronize()

        // Removing every key leaves an empty `{}` plist behind. CATrust holds uninstall to a
        // "no residue" standard and this path should match it — an orphaned root-owned file in
        // /Library/Preferences is exactly the kind of thing that outlives an uninstall and
        // confuses the next administrator.
        let path = URL(fileURLWithPath: "/Library/Preferences")
            .appendingPathComponent("\(applicationID as String).plist")
        if let remaining = try? read(), remaining.isEmpty {
            try? FileManager.default.removeItem(at: path)
        }
    }

    private func removeAllKeys() throws {
        let keys =
            CFPreferencesCopyKeyList(applicationID, kCFPreferencesAnyUser, kCFPreferencesAnyHost)
            as? [String] ?? []
        for key in keys {
            CFPreferencesSetValue(
                key as CFString, nil, applicationID, kCFPreferencesAnyUser, kCFPreferencesAnyHost)
        }
    }

    private func synchronize() throws {
        guard
            CFPreferencesSynchronize(applicationID, kCFPreferencesAnyUser, kCFPreferencesAnyHost)
        else {
            // The usual cause is running without root; the domain lives under /Library.
            throw FirefoxTrustError.writeFailed(domain: applicationID as String)
        }
    }
}
