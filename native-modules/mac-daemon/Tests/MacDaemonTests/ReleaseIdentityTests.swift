#if !HOLY_BLOCKER_DEV_BUILD
import Testing

@testable import MacDaemon

@Suite("release build identity")
struct ReleaseIdentityTests {
    @Test("has no development flavor to bundle")
    func noDevelopmentFlavor() {
        #expect(BundleIdentity.forFlavor("development") == nil)
        #expect(BundleIdentity.forFlavor("release") == .holyBlocker)
    }

    @Test("treats a development bundle identifier as the release identity")
    func developmentIdentifierIsNotRecognised() {
        #expect(BundleIdentity.running(bundleIdentifier: "com.holyblocker.daemon.dev") == .holyBlocker)
    }

    @Test("protects only its own bundle identifier among ours")
    func protectsOnlyRelease() {
        let ours = SuppressionPolicy.defaultProtected.filter { $0.hasPrefix("com.holyblocker") }
        #expect(ours == [BundleIdentity.holyBlocker.identifier])
    }
}
#endif
