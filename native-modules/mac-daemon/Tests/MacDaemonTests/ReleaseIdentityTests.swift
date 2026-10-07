import Foundation
import Testing

@testable import MacDaemon

private let listing = """
      1) 072F00A821D686310784A8790781567CE04ACEBF "Holy Blocker Dev"
      2) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Holy Blocker Release"
      3) BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB "Apple Development: Example (TEAMID0000)"
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

@Suite("BundleIdentity signing check")
struct BundleSigningCheckTests {
    @Test("a bundle signed with its own flavor's certificate matches")
    func matches() {
        #expect(
            BundleIdentity.holyBlocker.signingCheck(of: .signed(authority: "Holy Blocker Release"))
                == .matches)
        #expect(
            BundleIdentity.development.signingCheck(of: .signed(authority: "Holy Blocker Dev"))
                == .matches)
    }

    @Test("a release bundle carrying the development leaf is foreign")
    func foreign() {
        #expect(
            BundleIdentity.holyBlocker.signingCheck(of: .signed(authority: "Holy Blocker Dev"))
                == .foreign("Holy Blocker Dev"))
    }

    @Test("ad-hoc and unsigned are reported as such")
    func unstable() {
        #expect(BundleIdentity.holyBlocker.signingCheck(of: .adhoc) == .adhoc)
        #expect(BundleIdentity.holyBlocker.signingCheck(of: .unsigned) == .unsigned)
    }
}
