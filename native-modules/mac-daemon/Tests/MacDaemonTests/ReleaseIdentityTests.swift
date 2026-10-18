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

@Suite("BundleIdentity accepts")
struct BundleAcceptsTests {
    @Test("a release bundle must carry its own certificate")
    func releaseNeedsCertificate() {
        #expect(BundleIdentity.holyBlocker.accepts(.matches))
        #expect(!BundleIdentity.holyBlocker.accepts(.adhoc))
        #expect(!BundleIdentity.holyBlocker.accepts(.unsigned))
        #expect(!BundleIdentity.holyBlocker.accepts(.foreign("Holy Blocker Dev")))
    }

    @Test("a development bundle may be ad-hoc but never unsigned or foreign")
    func developmentMayBeAdhoc() {
        #expect(BundleIdentity.development.accepts(.matches))
        #expect(BundleIdentity.development.accepts(.adhoc))
        #expect(!BundleIdentity.development.accepts(.unsigned))
        #expect(!BundleIdentity.development.accepts(.foreign("Holy Blocker Release")))
    }
}

@Suite("Leaf fingerprint")
struct LeafFingerprintTests {
    @Test("hashes DER bytes to the upper-case SHA-1 that find-identity prints")
    func hash() {
        #expect(
            SigningIdentities.fingerprint(ofCertificate: Data("abc".utf8))
                == "A9993E364706816ABA3E25717850C26C9CD0D89D")
    }

    @Test("a leaf with the right name but another certificate does not match")
    func sameNameOtherCertificate() {
        let leaf = SigningIdentities.fingerprint(ofCertificate: Data("impostor".utf8))
        #expect(!SigningIdentities.leafMatches(.holyBlocker, fingerprint: leaf, listing: listing))
    }

    @Test("a leaf equal to the keychain's certificate for the flavor matches")
    func genuineLeaf() {
        #expect(
            SigningIdentities.leafMatches(
                .development, fingerprint: "072F00A821D686310784A8790781567CE04ACEBF",
                listing: listing))
    }

    @Test("a leaf equal to any of several trusted certificates under the name matches")
    func duplicateNames() {
        let twice = """
              1) AAAA1111AAAA1111AAAA1111AAAA1111AAAA1111 "Holy Blocker Release"
              2) BBBB2222BBBB2222BBBB2222BBBB2222BBBB2222 "Holy Blocker Release"
            """
        for fingerprint in ["AAAA1111AAAA1111AAAA1111AAAA1111AAAA1111", "BBBB2222BBBB2222BBBB2222BBBB2222BBBB2222"] {
            #expect(SigningIdentities.leafMatches(.holyBlocker, fingerprint: fingerprint, listing: twice))
        }
    }

    @Test("a flavor whose certificate is absent from the keychain never matches")
    func absent() {
        let devOnly = "  1) 072F00A821D686310784A8790781567CE04ACEBF \"Holy Blocker Dev\""
        #expect(
            !SigningIdentities.leafMatches(
                .holyBlocker, fingerprint: "072F00A821D686310784A8790781567CE04ACEBF",
                listing: devOnly))
    }

    @Test("the leaf certificate is extracted into a scratch directory and read back")
    func extraction() throws {
        let runner = FakeCommandRunner(defaultResult: CommandResult(exitCode: 0))
        _ = try? CodeSigning(runner: runner).leafFingerprint(of: URL(fileURLWithPath: "/x/App.app"))
        let arguments = runner.invocations.first?.arguments ?? []
        #expect(arguments.dropFirst().first?.hasPrefix("--extract-certificates=") == true)
        #expect(arguments.last == "/x/App.app")
    }
}
