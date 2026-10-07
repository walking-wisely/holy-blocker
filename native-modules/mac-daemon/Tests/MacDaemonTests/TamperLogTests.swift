import Foundation
import Testing

@testable import MacDaemon

@Suite("TamperLog")
struct TamperLogTests {
    private func entry(
        _ code: TamperCode, uptime: Int64 = 0, wall: Int64 = 0, detail: String = ""
    ) -> TamperEntry {
        TamperEntry(wallMillis: wall, uptimeMillis: uptime, code: code, detail: detail)
    }

    @Test("codes are unique, lowercase snake case, and carry no content words")
    func vocabularyIsContentFree() {
        let codes = TamperCode.allCases.map(\.rawValue)
        #expect(Set(codes).count == codes.count)
        for code in codes {
            #expect(code.range(of: "^[a-z]+(_[a-z]+)*$", options: .regularExpression) != nil)
            for forbidden in ["app", "bundle", "package", "url", "host", "domain", "text", "score"] {
                #expect(!code.contains(forbidden), "\(code) contains \(forbidden)")
            }
        }
    }

    @Test("format and parse round-trip")
    func roundTrip() {
        let original = entry(.permissionLost, uptime: 42, wall: 1_700_000_000_000, detail: "accessibility")
        #expect(TamperLog.parse(TamperLog.format(original)) == original)
    }

    @Test("a detail cannot smuggle a separator or newline into the file")
    func detailIsSanitized() {
        let line = TamperLog.format(entry(.permissionLost, detail: "a\tb\nc\r"))
        #expect(!line.contains("\n"))
        #expect(line.split(separator: "\t", omittingEmptySubsequences: false).count == 4)
    }

    @Test("a detail is capped")
    func detailIsBounded() {
        let line = TamperLog.format(
            entry(.permissionLost, detail: String(repeating: "x", count: 1_000)))
        #expect(TamperLog.parse(line)?.detail.count == TamperLog.maxDetailCharacters)
    }

    @Test("parse drops a torn line, an unknown code and a non-numeric clock")
    func parseIsTolerant() {
        #expect(TamperLog.parse("123\t45") == nil)
        #expect(TamperLog.parse("123\t45\tfuture_code\t") == nil)
        #expect(TamperLog.parse("abc\t45\tdaemon_on\t") == nil)
        #expect(TamperLog.parse("") == nil)
    }

    @Test("readAll keeps the readable lines in order")
    func readAllSkipsDamage() {
        let lines = [
            TamperLog.format(entry(.daemonStarted, uptime: 1)),
            "garbage",
            TamperLog.format(entry(.daemonStopped, uptime: 2)),
        ]
        #expect(TamperLog.readAll(lines).map(\.code) == [.daemonStarted, .daemonStopped])
    }

    @Test("trim keeps the newest lines and reports what it dropped")
    func trimDropsOldest() {
        let lines = (0..<10).map(String.init)
        let result = TamperLog.trim(lines, maxEntries: 4)
        #expect(result.kept == ["6", "7", "8", "9"])
        #expect(result.dropped == 6)
        #expect(TamperLog.trim(lines, maxEntries: 10).dropped == 0)
    }

    @Test("an empty log is a first run")
    func firstRun() {
        #expect(TamperLog.classifyStart(recent: [], nowUptime: 10) == .firstRun)
    }

    @Test("entries written outside any session are not evidence")
    func noBoundaryIsFirstRun() {
        let recent = [entry(.permissionGranted, uptime: 5)]
        #expect(TamperLog.classifyStart(recent: recent, nowUptime: 10) == .firstRun)
    }

    @Test("a session that ended with a stop record is a clean restart")
    func cleanRestart() {
        let recent = [entry(.daemonStarted, uptime: 1), entry(.daemonStopped, uptime: 5)]
        #expect(TamperLog.classifyStart(recent: recent, nowUptime: 10) == .cleanRestart)
    }

    @Test("a session with no stop record on the same boot is an unclean stop")
    func uncleanStop() {
        let recent = [entry(.daemonStarted, uptime: 1), entry(.permissionLost, uptime: 5)]
        let start = TamperLog.classifyStart(recent: recent, nowUptime: 10)
        #expect(start == .uncleanStop)
        #expect(start.notableCode == .uncleanStop)
    }

    @Test("an uptime that went backwards is a reboot, not a kill")
    func afterReboot() {
        let recent = [entry(.daemonStarted, uptime: 5_000), entry(.permissionLost, uptime: 9_000)]
        let start = TamperLog.classifyStart(recent: recent, nowUptime: 100)
        #expect(start == .afterReboot)
        #expect(start.notableCode == nil)
    }

    @Test("a reset hidden behind a later low-uptime line is still seen")
    func rebootHiddenBehindLaterEntry() {
        let recent = [
            entry(.daemonStarted, uptime: 5_000),
            entry(.rebooted, uptime: 20),
            entry(.permissionGranted, uptime: 30),
        ]
        #expect(TamperLog.classifyStart(recent: recent, nowUptime: 60) == .afterReboot)
    }

    @Test("an older boot before the last session does not excuse it")
    func olderBootIgnored() {
        let recent = [
            entry(.daemonStarted, uptime: 9_000),
            entry(.daemonStopped, uptime: 9_500),
            entry(.daemonStarted, uptime: 10),
            entry(.permissionGranted, uptime: 20),
        ]
        #expect(TamperLog.classifyStart(recent: recent, nowUptime: 60) == .uncleanStop)
    }

    @Test("every permission-gate event maps to a code, and no user name reaches the log")
    func gateEventsMap() {
        let mapped: [(TamperEvent, TamperCode, String)] = [
            (.permissionGranted(.screenRecording), .permissionGranted, "screenRecording"),
            (.permissionLost(.accessibility), .permissionLost, "accessibility"),
            (.systemIntegrityProtectionDisabled, .sipDisabled, ""),
            (.systemIntegrityProtectionEnabled, .sipEnabled, ""),
            (.localUserAdded("alice"), .userAdded, ""),
            (.localUserRemoved("alice"), .userRemoved, ""),
            (.administratorAdded("alice"), .adminAdded, ""),
            (.administratorRemoved("alice"), .adminRemoved, ""),
            (.protectedUserBecameAdministrator, .becameAdministrator, ""),
            (.rebooted(Date(timeIntervalSince1970: 0)), .rebooted, ""),
        ]
        for (event, code, detail) in mapped {
            let result = TamperLog.record(for: event)
            #expect(result.code == code)
            #expect(result.detail.text == detail)
        }
    }
}

@Suite("TamperLogStore")
struct TamperLogStoreTests {
    private func makeStore(maxEntries: Int = TamperLog.maxEntries, slack: Int = 5)
        throws -> (TamperLogStore, URL)
    {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("tamper-log-\(UUID().uuidString)")
        let store = TamperLogStore(directory: directory, maxEntries: maxEntries, trimSlack: slack)
        return (store, directory)
    }

    @Test("records append and read back in order")
    func appendAndRead() throws {
        let (store, _) = try makeStore()
        #expect(store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1))
        #expect(store.record(.permissionLost, detail: .capability(.accessibility), wallMillis: 2, uptimeMillis: 2))
        #expect(store.entries().map(\.code) == [.daemonStarted, .permissionLost])
        #expect(store.sessionEntries().map(\.code) == [.daemonStarted, .permissionLost])
    }

    @Test("a second store over the same directory sees the first one's entries")
    func survivesRestart() throws {
        let (store, directory) = try makeStore()
        store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1)
        let reopened = TamperLogStore(directory: directory)
        #expect(reopened.entries().count == 1)
    }

    @Test("the file is private to its owner")
    func filePermissions() throws {
        let (store, directory) = try makeStore()
        store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1)
        let attributes = try FileManager.default.attributesOfItem(
            atPath: directory.appendingPathComponent(TamperLogStore.fileName).path)
        #expect((attributes[.posixPermissions] as? Int) == 0o600)
    }

    @Test("a torn final line costs only that entry")
    func tornTail() throws {
        let (store, directory) = try makeStore()
        store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1)
        let handle = try FileHandle(
            forWritingTo: directory.appendingPathComponent(TamperLogStore.fileName))
        try handle.seekToEnd()
        try handle.write(contentsOf: Data("12\t3\tdaem".utf8))
        try handle.close()
        #expect(TamperLogStore(directory: directory).entries().map(\.code) == [.daemonStarted])
    }

    @Test("past the cap the oldest entries go and a trim marker is left")
    func boundedWithMarker() throws {
        let (store, _) = try makeStore(maxEntries: 10, slack: 3)
        for index in 0..<40 {
            store.record(.permissionGranted, detail: .dropped(index), wallMillis: 0, uptimeMillis: Int64(index))
        }
        let entries = store.entries()
        #expect(entries.count <= 10 + 3 + 1)
        #expect(entries.contains { $0.code == .logTrimmed })
        #expect(entries.last?.detail == "dropped=39" || entries.last?.code == .logTrimmed)
        #expect(!entries.contains { $0.detail == "dropped=0" })
    }

    @Test("an unwritable directory never throws and reports no write")
    func unwritableIsQuiet() {
        let store = TamperLogStore(directory: URL(fileURLWithPath: "/dev/null/nope"))
        #expect(!store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1))
        #expect(store.entries().isEmpty)
    }

    @Test("an append after a torn tail starts on its own line")
    func appendAfterTear() throws {
        let (store, directory) = try makeStore()
        store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1)
        let handle = try FileHandle(
            forWritingTo: directory.appendingPathComponent(TamperLogStore.fileName))
        try handle.seekToEnd()
        try handle.write(contentsOf: Data("5\t200\tperm".utf8))
        try handle.close()
        let reopened = TamperLogStore(directory: directory)
        reopened.record(.daemonStarted, wallMillis: 9, uptimeMillis: 300)
        #expect(reopened.entries().map(\.code) == [.daemonStarted, .daemonStarted])
    }

    @Test("the session window reaches back past any number of later entries")
    func sessionWindowIsNotFixed() throws {
        let (store, _) = try makeStore()
        store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1)
        for index in 0..<200 {
            store.record(
                index.isMultiple(of: 2) ? .permissionLost : .permissionGranted,
                detail: .capability(.accessibility), wallMillis: 2, uptimeMillis: Int64(2 + index))
        }
        let session = store.sessionEntries()
        #expect(session.first?.code == .daemonStarted)
        #expect(TamperLog.classifyStart(recent: session, nowUptime: 5_000) == .uncleanStop)
    }

    @Test("a symlinked log file is refused, not followed")
    func refusesSymlink() throws {
        let (store, directory) = try makeStore()
        store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1)
        let target = directory.appendingPathComponent("victim")
        try Data("keep\n".utf8).write(to: target)
        let log = directory.appendingPathComponent(TamperLogStore.fileName)
        try FileManager.default.removeItem(at: log)
        try FileManager.default.createSymbolicLink(at: log, withDestinationURL: target)
        let fresh = TamperLogStore(directory: directory)
        #expect(!fresh.record(.daemonStopped, wallMillis: 2, uptimeMillis: 2))
        #expect(try String(contentsOf: target, encoding: .utf8) == "keep\n")
    }

    @Test("a state directory others can write to is refused")
    func refusesLooseDirectory() throws {
        let (store, directory) = try makeStore()
        store.record(.daemonStarted, wallMillis: 1, uptimeMillis: 1)
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o777], ofItemAtPath: directory.path)
        let fresh = TamperLogStore(directory: directory)
        #expect(!fresh.record(.daemonStopped, wallMillis: 2, uptimeMillis: 2))
    }

    @Test("the log keeps its mode after a trim")
    func modeSurvivesTrim() throws {
        let (store, directory) = try makeStore(maxEntries: 5, slack: 2)
        for index in 0..<20 {
            store.record(.permissionGranted, detail: .dropped(index), wallMillis: 0, uptimeMillis: Int64(index))
        }
        let attributes = try FileManager.default.attributesOfItem(
            atPath: directory.appendingPathComponent(TamperLogStore.fileName).path)
        #expect((attributes[.posixPermissions] as? Int) == 0o600)
    }
}
