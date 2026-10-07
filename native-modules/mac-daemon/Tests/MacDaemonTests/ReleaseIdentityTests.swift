import Foundation
import Testing

@testable import MacDaemon

private let listing = """
      1) 072F00A821D686310784A8790781567CE04ACEBF "Holy Blocker Dev"
      2) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Holy Blocker Release"
      3) F1150B33206FB73238B5185EBCBC9BE3790A9856 "Apple Development: Someone (QVXF5GVX5V)"
         3 valid identities found
    """

@Suite("SigningIdentities")
struct SigningIdentitiesTests {
    @Test("parses the fingerprint of a named identity")
    func parses() {
        let found = SigningIdentities.parse(listing)
        #expect(found["Holy Blocker Dev"] == "072F00A821D686310784A8790781567CE04ACEBF")
        #expect(found["Holy Blocker Release"] == "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
    }

    @Test("ignores the summary line")
    func ignoresSummary() {
        #expect(SigningIdentities.parse(listing).count == 3)
    }

    @Test("distinct certificates are reported distinct")
    func distinct() {
        #expect(SigningIdentities.distinction(in: listing) == .distinct)
    }

    @Test("the same fingerprint under both names is reported the same")
    func same() {
        let shared = """
              1) 072F00A821D686310784A8790781567CE04ACEBF "Holy Blocker Dev"
              2) 072F00A821D686310784A8790781567CE04ACEBF "Holy Blocker Release"
            """
        #expect(SigningIdentities.distinction(in: shared) == .same)
    }

    @Test("a missing identity is named rather than reported distinct")
    func missing() {
        let devOnly = "  1) 072F00A821D686310784A8790781567CE04ACEBF \"Holy Blocker Dev\""
        #expect(SigningIdentities.distinction(in: devOnly) == .missing(["Holy Blocker Release"]))
        #expect(SigningIdentities.distinction(in: "") == .missing(["Holy Blocker Release", "Holy Blocker Dev"]))
    }

    @Test("inspect asks the keychain for code-signing identities only")
    func inspect() throws {
        let runner = FakeCommandRunner(
            defaultResult: CommandResult(exitCode: 0, standardOutput: listing))
        #expect(try SigningIdentities.inspect(runner: runner) == .distinct)
        #expect(runner.invocations.first?.arguments == ["find-identity", "-v", "-p", "codesigning"])
    }
}

@Suite("BundleIdentity release signing")
struct ReleaseSigningTests {
    @Test("the release bundle refuses the development certificate")
    func refusesDevelopment() {
        #expect(!BundleIdentity.holyBlocker.permitsSigning(with: "Holy Blocker Dev"))
    }

    @Test("the release bundle accepts the release certificate and ad-hoc")
    func accepts() {
        #expect(BundleIdentity.holyBlocker.permitsSigning(with: "Holy Blocker Release"))
        #expect(BundleIdentity.holyBlocker.permitsSigning(with: "-"))
    }

    @Test("the release and development certificate names differ")
    func namesDiffer() {
        #expect(BundleIdentity.releaseSigningIdentity != BundleIdentity.developmentSigningIdentity)
    }
}
