import Foundation
import Testing

@testable import MacDaemon

/// In-memory stand-in for the managed-preferences domain.
private final class FakePolicyStore: FirefoxPolicyStore, @unchecked Sendable {
    var policy: [String: Any]
    private(set) var writeCount = 0
    private(set) var clearCount = 0

    init(policy: [String: Any] = [:]) { self.policy = policy }

    func read() throws -> [String: Any] { policy }

    func write(_ policy: [String: Any]) throws {
        self.policy = policy
        writeCount += 1
    }

    func clear() throws {
        policy = [:]
        clearCount += 1
    }
}

private func equal(_ lhs: [String: Any], _ rhs: [String: Any]) -> Bool {
    NSDictionary(dictionary: lhs) == NSDictionary(dictionary: rhs)
}

@Suite("FirefoxTrust policy merging")
struct FirefoxTrustMergeTests {
    @Test("enables enterprise roots on an empty policy domain")
    func enablesFromEmpty() {
        let result = FirefoxTrust.enablingEnterpriseRoots(in: [:])

        #expect(
            equal(
                result,
                [
                    "EnterprisePoliciesEnabled": true,
                    "Certificates": ["ImportEnterpriseRoots": true],
                ]))
    }

    @Test("leaves policies set by other administrators untouched")
    func preservesForeignPolicies() {
        // An MDM-managed Mac may already carry a Firefox policy payload. Replacing the domain
        // wholesale would silently undo someone else's configuration.
        let existing: [String: Any] = [
            "EnterprisePoliciesEnabled": true,
            "DisableTelemetry": true,
            "Certificates": ["Install": ["corp-root.pem"]],
        ]

        let result = FirefoxTrust.enablingEnterpriseRoots(in: existing)

        #expect(result["DisableTelemetry"] as? Bool == true)
        let certificates = result["Certificates"] as? [String: Any]
        #expect(certificates?["Install"] as? [String] == ["corp-root.pem"])
        #expect(certificates?["ImportEnterpriseRoots"] as? Bool == true)
    }

    @Test("overrides an explicit opt-out")
    func overridesFalse() {
        let existing: [String: Any] = ["Certificates": ["ImportEnterpriseRoots": false]]

        let result = FirefoxTrust.enablingEnterpriseRoots(in: existing)

        let certificates = result["Certificates"] as? [String: Any]
        #expect(certificates?["ImportEnterpriseRoots"] as? Bool == true)
    }

    @Test("turns the policy engine on, since a policy nobody reads does nothing")
    func setsEnterprisePoliciesEnabled() {
        let result = FirefoxTrust.enablingEnterpriseRoots(in: ["EnterprisePoliciesEnabled": false])
        #expect(result["EnterprisePoliciesEnabled"] as? Bool == true)
    }
}

@Suite("FirefoxTrust policy removal")
struct FirefoxTrustRemovalTests {
    @Test("removes only our key and empties the domain when nothing else remains")
    func removesEverythingWeAdded() {
        let installed = FirefoxTrust.enablingEnterpriseRoots(in: [:])

        let result = FirefoxTrust.removingEnterpriseRoots(from: installed)

        #expect(result.isEmpty)
    }

    @Test("keeps a Certificates block that carries other settings")
    func keepsOtherCertificateSettings() {
        let existing: [String: Any] = [
            "EnterprisePoliciesEnabled": true,
            "Certificates": [
                "ImportEnterpriseRoots": true,
                "Install": ["corp-root.pem"],
            ],
        ]

        let result = FirefoxTrust.removingEnterpriseRoots(from: existing)

        let certificates = result["Certificates"] as? [String: Any]
        #expect(certificates?["ImportEnterpriseRoots"] == nil)
        #expect(certificates?["Install"] as? [String] == ["corp-root.pem"])
        // Another policy still needs the engine on.
        #expect(result["EnterprisePoliciesEnabled"] as? Bool == true)
    }

    @Test("leaves the policy engine enabled when unrelated policies remain")
    func keepsEngineForForeignPolicies() {
        let existing: [String: Any] = [
            "EnterprisePoliciesEnabled": true,
            "DisableTelemetry": true,
            "Certificates": ["ImportEnterpriseRoots": true],
        ]

        let result = FirefoxTrust.removingEnterpriseRoots(from: existing)

        #expect(result["EnterprisePoliciesEnabled"] as? Bool == true)
        #expect(result["DisableTelemetry"] as? Bool == true)
        #expect(result["Certificates"] == nil)
    }

    @Test("is a no-op on a domain that never had the policy")
    func noopWhenAbsent() {
        let existing: [String: Any] = ["DisableTelemetry": true]

        let result = FirefoxTrust.removingEnterpriseRoots(from: existing)

        #expect(equal(result, existing))
    }
}

@Suite("FirefoxTrust state")
struct FirefoxTrustStateTests {
    @Test("reports absent for an empty domain")
    func absent() {
        #expect(FirefoxTrust.readState([:]) == .absent)
        #expect(FirefoxTrust.readState(["DisableTelemetry": true]) == .absent)
    }

    @Test("reports enabled once the policy is in place")
    func enabled() {
        let policy = FirefoxTrust.enablingEnterpriseRoots(in: [:])
        #expect(FirefoxTrust.readState(policy) == .enabled)
    }

    @Test("reports disabled on an explicit opt-out")
    func disabled() {
        let policy: [String: Any] = [
            "EnterprisePoliciesEnabled": true,
            "Certificates": ["ImportEnterpriseRoots": false],
        ]
        #expect(FirefoxTrust.readState(policy) == .disabled)
    }

    @Test("reports disabled when the policy engine itself is switched off")
    func engineOff() {
        // The key is set but Firefox will not read it, so the CA is not actually trusted — saying
        // "enabled" here would report protection the user does not have.
        let policy: [String: Any] = [
            "EnterprisePoliciesEnabled": false,
            "Certificates": ["ImportEnterpriseRoots": true],
        ]
        #expect(FirefoxTrust.readState(policy) == .disabled)
    }
}

@Suite("FirefoxTrust install and uninstall")
struct FirefoxTrustLifecycleTests {
    @Test("writes the policy on install")
    func install() throws {
        let store = FakePolicyStore()
        let trust = FirefoxTrust(store: store)

        try trust.install()

        #expect(try trust.state() == .enabled)
        #expect(store.writeCount == 1)
    }

    @Test("does not rewrite the domain when already enabled")
    func installIsIdempotent() throws {
        let store = FakePolicyStore()
        let trust = FirefoxTrust(store: store)
        try trust.install()

        try trust.install()

        #expect(store.writeCount == 1)
    }

    @Test("re-enables an explicit opt-out")
    func installOverridesOptOut() throws {
        let store = FakePolicyStore(policy: [
            "Certificates": ["ImportEnterpriseRoots": false]
        ])
        let trust = FirefoxTrust(store: store)

        try trust.install()

        #expect(try trust.state() == .enabled)
    }

    @Test("clears the domain on uninstall when we were its only occupant")
    func uninstallClears() throws {
        let store = FakePolicyStore()
        let trust = FirefoxTrust(store: store)
        try trust.install()

        try trust.uninstall()

        #expect(try trust.state() == .absent)
        // Leaving an empty plist behind counts as residue; the CA uninstall path holds the same
        // standard.
        #expect(store.clearCount == 1)
    }

    @Test("keeps another administrator's policies on uninstall")
    func uninstallPreservesForeignPolicies() throws {
        let store = FakePolicyStore(policy: ["DisableTelemetry": true])
        let trust = FirefoxTrust(store: store)
        try trust.install()

        try trust.uninstall()

        #expect(try trust.state() == .absent)
        #expect(store.policy["DisableTelemetry"] as? Bool == true)
        #expect(store.clearCount == 0)
    }

    @Test("is a no-op when the policy was never installed")
    func uninstallWhenAbsent() throws {
        let store = FakePolicyStore()
        let trust = FirefoxTrust(store: store)

        try trust.uninstall()

        #expect(store.writeCount == 0)
        #expect(store.clearCount == 0)
    }
}
