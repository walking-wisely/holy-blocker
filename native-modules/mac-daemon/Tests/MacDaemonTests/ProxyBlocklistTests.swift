import Foundation
import Testing

@testable import MacDaemon

private func makeResources() throws -> URL {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("proxy-blocklist-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    return root
}

private func writeList(
    in resources: URL, slots: [String] = ["current"], keys: [String: Data]
) throws {
    let list = resources.appendingPathComponent(ProxyBlocklist.directoryName)
    for slot in slots {
        try FileManager.default.createDirectory(
            at: list.appendingPathComponent(slot), withIntermediateDirectories: true)
    }
    let keyDirectory = list.appendingPathComponent("keys")
    try FileManager.default.createDirectory(at: keyDirectory, withIntermediateDirectories: true)
    for (name, bytes) in keys {
        try bytes.write(to: keyDirectory.appendingPathComponent(name))
    }
}

@Suite("ProxyBlocklist.resolve")
struct ProxyBlocklistResolveTests {
    @Test("passes the list directory and each key as hex")
    func configured() throws {
        let resources = try makeResources()
        defer { try? FileManager.default.removeItem(at: resources) }
        try writeList(
            in: resources, keys: ["dev-1.pub": Data([0x00, 0xAB] + [UInt8](repeating: 0xFF, count: 30))])

        let source = ProxyBlocklist.resolve(resources: resources)

        let expectedHex = "00ab" + String(repeating: "ff", count: 30)
        #expect(
            source
                == .configured(arguments: [
                    "--blocklist-dir", resources.appendingPathComponent("blocklist").path,
                    "--blocklist-key", "dev-1:\(expectedHex)",
                ]))
    }

    @Test("repeats the key flag per key, in name order")
    func severalKeys() throws {
        let resources = try makeResources()
        defer { try? FileManager.default.removeItem(at: resources) }
        let bytes = Data(repeating: 1, count: 32)
        try writeList(in: resources, keys: ["b.pub": bytes, "a.pub": bytes])

        guard case .configured(let arguments) = ProxyBlocklist.resolve(resources: resources) else {
            Issue.record("expected configured")
            return
        }
        #expect(arguments.filter { $0 == "--blocklist-key" }.count == 2)
        #expect(arguments.firstIndex { $0.hasPrefix("a:") }! < arguments.firstIndex { $0.hasPrefix("b:") }!)
    }

    @Test("is unavailable when not running from a bundle")
    func noResources() {
        #expect(ProxyBlocklist.resolve(resources: nil) == .unavailable(.missing))
    }

    @Test("is unavailable when the bundle carries no list")
    func noList() throws {
        let resources = try makeResources()
        defer { try? FileManager.default.removeItem(at: resources) }
        #expect(ProxyBlocklist.resolve(resources: resources) == .unavailable(.missing))
    }

    @Test("is unavailable when there is no current slot")
    func noCurrentSlot() throws {
        let resources = try makeResources()
        defer { try? FileManager.default.removeItem(at: resources) }
        try writeList(in: resources, slots: ["previous"], keys: ["k.pub": Data(count: 32)])
        #expect(ProxyBlocklist.resolve(resources: resources) == .unavailable(.missing))
    }

    @Test("is rejected when no usable key accompanies the list")
    func noKeys() throws {
        let resources = try makeResources()
        defer { try? FileManager.default.removeItem(at: resources) }
        try writeList(in: resources, keys: [:])
        #expect(ProxyBlocklist.resolve(resources: resources) == .unavailable(.rejected))
    }

    @Test("skips keys that are not 32 bytes or whose id cannot be passed to the proxy")
    func malformedKeys() throws {
        let resources = try makeResources()
        defer { try? FileManager.default.removeItem(at: resources) }
        try writeList(
            in: resources,
            keys: [
                "short.pub": Data(count: 31),
                "has:colon.pub": Data(count: 32),
                "notakey.txt": Data(count: 32),
            ])
        #expect(ProxyBlocklist.resolve(resources: resources) == .unavailable(.rejected))
    }
}

@Suite("ProxyBlocklist.stage")
struct ProxyBlocklistStageTests {
    @Test("lays the artifact slots and keys out as resolve expects")
    func stagesResolvableLayout() throws {
        let work = try makeResources()
        defer { try? FileManager.default.removeItem(at: work) }
        let artifacts = work.appendingPathComponent("artifacts")
        for slot in ["current", "previous"] {
            let dir = artifacts.appendingPathComponent(slot)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try Data([1]).write(to: dir.appendingPathComponent("manifest.bin"))
        }
        let keys = work.appendingPathComponent("keys")
        try FileManager.default.createDirectory(at: keys, withIntermediateDirectories: true)
        try Data(repeating: 7, count: 32).write(to: keys.appendingPathComponent("dev-1.pub"))
        try Data([9]).write(to: keys.appendingPathComponent("ignored.txt"))
        let resources = work.appendingPathComponent("resources")
        try FileManager.default.createDirectory(at: resources, withIntermediateDirectories: true)

        try ProxyBlocklist.stage(artifactDirectory: artifacts, keyDirectory: keys, into: resources)

        guard case .configured = ProxyBlocklist.resolve(resources: resources) else {
            Issue.record("staged layout did not resolve")
            return
        }
        let list = resources.appendingPathComponent("blocklist")
        #expect(FileManager.default.fileExists(atPath: list.appendingPathComponent("previous/manifest.bin").path))
        #expect(!FileManager.default.fileExists(atPath: list.appendingPathComponent("keys/ignored.txt").path))
    }

    @Test("refuses an artifact directory with no current slot")
    func refusesMissingCurrent() throws {
        let work = try makeResources()
        defer { try? FileManager.default.removeItem(at: work) }
        let artifacts = work.appendingPathComponent("artifacts")
        try FileManager.default.createDirectory(at: artifacts, withIntermediateDirectories: true)

        #expect(throws: ProxyBlocklistStageError.noCurrentSlot(artifacts)) {
            try ProxyBlocklist.stage(
                artifactDirectory: artifacts, keyDirectory: work, into: work)
        }
    }
}

@Suite("ProxyBlocklistPolicy")
struct ProxyBlocklistPolicyTests {
    private let configured = ProxyBlocklistSource.configured(arguments: ["--blocklist-dir", "/x"])

    @Test("requires a list by default")
    func requiredByDefault() {
        let policy = ProxyBlocklistPolicy(environment: [:])
        #expect(policy == .required)
        #expect(policy.arguments(for: .unavailable(.missing)) == nil)
        #expect(policy.arguments(for: .unavailable(.rejected)) == nil)
    }

    @Test("passes the flags through whenever the list resolved")
    func configuredPasses() {
        #expect(ProxyBlocklistPolicy.required.arguments(for: configured) == ["--blocklist-dir", "/x"])
        #expect(ProxyBlocklistPolicy.optional.arguments(for: configured) == ["--blocklist-dir", "/x"])
    }

#if HOLY_BLOCKER_DEV_BUILD
    @Test("runs unfiltered only on the explicit opt-in")
    func optIn() {
        let policy = ProxyBlocklistPolicy(
            environment: [ProxyBlocklistPolicy.allowMissingVariable: "1"])
        #expect(policy.arguments(for: .unavailable(.missing)) == [])
    }

    @Test("treats any other value as not opted in")
    func otherValues() {
        let policy = ProxyBlocklistPolicy(
            environment: [ProxyBlocklistPolicy.allowMissingVariable: "true"])
        #expect(policy == .required)
    }
#else
    @Test("ignores the development override in a release build")
    func overrideIgnored() {
        let policy = ProxyBlocklistPolicy(
            environment: ["HOLY_BLOCKER_ALLOW_NO_BLOCKLIST": "1"])
        #expect(policy == .required)
        #expect(policy.arguments(for: .unavailable(.missing)) == nil)
    }
#endif
}
