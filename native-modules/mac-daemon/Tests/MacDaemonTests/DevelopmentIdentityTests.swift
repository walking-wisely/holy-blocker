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
