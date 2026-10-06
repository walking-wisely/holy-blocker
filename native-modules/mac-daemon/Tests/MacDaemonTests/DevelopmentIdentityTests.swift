#if HOLY_BLOCKER_DEV_BUILD
import Foundation
import Testing

@testable import MacDaemon

@Suite("BundleIdentity.development")
struct DevelopmentIdentityTests {
    @Test("has its own bundle identifier, so TCC keeps its grants apart from the release build")
    func distinctIdentifier() {
        #expect(BundleIdentity.development.identifier != BundleIdentity.holyBlocker.identifier)
    }

    @Test("has its own bundle file name, so it installs beside the release bundle")
    func distinctFileName() {
        #expect(BundleIdentity.development.bundleFileName != BundleIdentity.holyBlocker.bundleFileName)
        #expect(BundleIdentity.holyBlocker.bundleFileName == "HolyBlockerDaemon.app")
    }

    @Test("names itself differently, since the permission prompt shows only the bundle name")
    func distinctName() {
        #expect(BundleIdentity.development.name != BundleIdentity.holyBlocker.name)
    }

    @Test("is never a launchd label or prefix the release build owns")
    func identifierIsNotAReleaseLabel() {
        let release = [BundleIdentity.holyBlocker.identifier, "com.holyblocker.agent"]
        #expect(!release.contains(BundleIdentity.development.identifier))
    }

    @Test("is protected from suppression alongside the release identity")
    func protectedFromHiding() {
        #expect(
            SuppressionPolicy.defaultProtected.isSuperset(of: [
                BundleIdentity.holyBlocker.identifier, BundleIdentity.development.identifier,
            ]))
    }
}

@Suite("BundleIdentity.forFlavor")
struct BundleFlavorTests {
    @Test("an absent or release flavor is the release identity")
    func releaseDefault() {
        #expect(BundleIdentity.forFlavor(nil) == .holyBlocker)
        #expect(BundleIdentity.forFlavor("release") == .holyBlocker)
    }

    @Test("the development flavor is the development identity")
    func development() {
        #expect(BundleIdentity.forFlavor("development") == .development)
    }

    @Test("an unknown flavor is refused rather than falling back to release")
    func unknown() {
        #expect(BundleIdentity.forFlavor("dev ") == nil)
        #expect(BundleIdentity.forFlavor("") == nil)
    }
}

@Suite("BundleIdentity shared-state names")
struct BundleStateNamesTests {
    @Test("release names are the ones already shipped")
    func releaseNames() {
        let release = BundleIdentity.holyBlocker
        #expect(release.daemonLabel == "com.holyblocker.daemon")
        #expect(release.agentLabel == "com.holyblocker.agent")
        #expect(release.stateDirectoryPath == "/Library/Application Support/HolyBlocker")
        #expect(release.daemonLogName == "holy-blocker-daemon.log")
        #expect(release.agentLogName == "holy-blocker-agent.log")
        #expect(release.installPath == "/Applications/HolyBlockerDaemon.app")
    }

    @Test("no name is shared between the two builds")
    func noSharedNames() {
        let release = BundleIdentity.holyBlocker
        let development = BundleIdentity.development
        let names: (BundleIdentity) -> [String] = {
            [
                $0.daemonLabel, $0.agentLabel, $0.stateDirectoryPath, $0.daemonLogName,
                $0.agentLogName, $0.installPath,
            ]
        }
        #expect(Set(names(release)).isDisjoint(with: names(development)))
    }

    @Test("the running identity is development only for the development bundle id")
    func running() {
        #expect(BundleIdentity.running(bundleIdentifier: nil) == .holyBlocker)
        #expect(BundleIdentity.running(bundleIdentifier: "com.holyblocker.daemon") == .holyBlocker)
        #expect(BundleIdentity.running(bundleIdentifier: "com.holyblocker.daemon.dev") == .development)
        #expect(BundleIdentity.running(bundleIdentifier: "com.example.other") == .holyBlocker)
    }
}

@Suite("BundleIdentity.permitsSigning")
struct BundleSigningTests {
    @Test("development accepts ad-hoc and the development certificate only")
    func development() {
        #expect(BundleIdentity.development.permitsSigning(with: "-"))
        #expect(BundleIdentity.development.permitsSigning(with: "Holy Blocker Dev"))
        #expect(!BundleIdentity.development.permitsSigning(with: "Holy Blocker Release"))
        #expect(!BundleIdentity.development.permitsSigning(with: ""))
    }

    @Test("release signing is not restricted by this guard")
    func release() {
        #expect(BundleIdentity.holyBlocker.permitsSigning(with: "Holy Blocker Dev"))
        #expect(BundleIdentity.holyBlocker.permitsSigning(with: "-"))
    }
}

@Suite("AppBundle.infoPlist for the development identity")
struct DevelopmentInfoPlistTests {
    @Test("carries the development identifier and name")
    func plist() throws {
        let data = try AppBundle.infoPlist(for: .development)
        let entries =
            try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any]
        #expect(entries?["CFBundleIdentifier"] as? String == "com.holyblocker.daemon.dev")
        #expect(entries?["CFBundleName"] as? String == "Holy Blocker Dev")
    }
}
#endif
