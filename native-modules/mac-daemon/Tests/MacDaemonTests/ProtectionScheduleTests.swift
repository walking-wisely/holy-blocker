import Foundation
import Testing

@testable import MacDaemon

@Suite("ProtectionSchedule")
struct ProtectionScheduleTests {
    private struct Vector {
        let name: String
        let armed: Bool
        let requestedAt: Int64?
        let disarmedAt: Int64?
        let now: Int64
        let expected: ProtectionState
    }

    private static func vectors() throws -> [Vector] {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let text = try String(
            contentsOf: root.appendingPathComponent("test-vectors/protection-schedule.tsv"),
            encoding: .utf8)
        func optional(_ field: Substring) -> Int64? { field == "-" ? nil : Int64(field) }
        return try text.split(separator: "\n").filter { !$0.hasPrefix("#") }.map { line in
            let f = line.split(separator: "\t", omittingEmptySubsequences: false)
            try #require(f.count == 7, "malformed vector: \(line)")
            return Vector(
                name: String(f[0]), armed: f[1] == "1", requestedAt: optional(f[2]),
                disarmedAt: optional(f[3]), now: Int64(f[4])!,
                expected: ProtectionState(
                    phase: try #require(ProtectionPhase(rawValue: String(f[5]))),
                    remainingMillis: Int64(f[6])!))
        }
    }

    @Test("every shared vector evaluates to its recorded state")
    func sharedVectors() throws {
        let all = try Self.vectors()
        #expect(all.count >= 15)
        for v in all {
            let state = ProtectionSchedule.evaluate(
                armed: v.armed, requestedAt: v.requestedAt, disarmedAt: v.disarmedAt, now: v.now)
            #expect(state == v.expected, "\(v.name)")
        }
    }

    @Test("only armed and the two waiting phases guard")
    func guardActive() {
        #expect(ProtectionState(phase: .off, remainingMillis: 0).guardActive == false)
        #expect(ProtectionState(phase: .disarmed, remainingMillis: 1).guardActive == false)
        for phase in [ProtectionPhase.armed, .disarmPending, .disarmReady] {
            #expect(ProtectionState(phase: phase, remainingMillis: 0).guardActive)
        }
    }

    @Test("the constants are the mechanism")
    func constants() {
        #expect(ProtectionSchedule.cooldownMillis == 15 * 60_000)
        #expect(ProtectionSchedule.readyWindowMillis == 5 * 60_000)
        #expect(ProtectionSchedule.disarmWindowMillis == 10 * 60_000)
    }

    @Test("the clock never runs behind systemUptime, which stops during sleep")
    func clockIsNotBehindUptime() {
        let a = ProtectionClock.nowMillis()
        let b = ProtectionClock.nowMillis()
        #expect(b >= a)
        #expect(a >= Int64(ProcessInfo.processInfo.systemUptime * 1000) - 1000)
    }

    private let boot = "boot-a"

    @Test("disarming needs request, cooldown and confirmation, in that order")
    func transitions() {
        var record = ProtectionRecord()
        #expect(record.requestDisarm(now: 0, bootID: boot) == nil)
        #expect(record.arm() == .protectionArmed)
        #expect(record.arm() == nil)
        #expect(record.confirmDisarm(now: 0, bootID: boot) == nil)
        #expect(record.requestDisarm(now: 1_000, bootID: boot) == .disarmRequested)
        #expect(record.requestDisarm(now: 2_000, bootID: boot) == nil)
        #expect(record.requestedAt == 1_000)
        let early = 1_000 + ProtectionSchedule.cooldownMillis - 1
        #expect(record.confirmDisarm(now: early, bootID: boot) == nil)
        let ready = 1_000 + ProtectionSchedule.cooldownMillis
        #expect(record.confirmDisarm(now: ready, bootID: boot) == .disarmConfirmed)
        #expect(record.state(now: ready, bootID: boot).phase == .disarmed)
        #expect(record.confirmDisarm(now: ready, bootID: boot) == nil)
    }

    @Test("arming clears an open window and cancel only logs a real request")
    func armAndCancel() {
        var record = ProtectionRecord(armed: true, requestedAt: nil, disarmedAt: 1_000, bootID: boot)
        #expect(record.arm() == .protectionArmed)
        #expect(record.disarmedAt == nil)
        #expect(record.cancelDisarm(now: 0, bootID: boot) == nil)
        _ = record.requestDisarm(now: 5, bootID: boot)
        #expect(record.cancelDisarm(now: 6, bootID: boot) == .disarmCancelled)
        #expect(record.state(now: 6, bootID: boot).phase == .armed)
    }

    @Test("a second disarm works after the first window has expired")
    func repeatable() {
        var record = ProtectionRecord(armed: true)
        _ = record.requestDisarm(now: 0, bootID: boot)
        let first = ProtectionSchedule.cooldownMillis
        _ = record.confirmDisarm(now: first, bootID: boot)
        let later = first + ProtectionSchedule.disarmWindowMillis + 1
        #expect(record.requestDisarm(now: later, bootID: boot) == .disarmRequested)
        #expect(record.state(now: later, bootID: boot).phase == .disarmPending)
    }

    @Test("a disarm from an earlier boot cannot reopen once the new clock catches up")
    func rebootReplay() {
        var record = ProtectionRecord(armed: true)
        _ = record.requestDisarm(now: 100_000, bootID: boot)
        let confirmedAt = 100_000 + ProtectionSchedule.cooldownMillis
        _ = record.confirmDisarm(now: confirmedAt, bootID: boot)
        #expect(record.state(now: confirmedAt + 1, bootID: boot).phase == .disarmed)
        #expect(record.state(now: confirmedAt + 1, bootID: "boot-b").phase == .armed)
        #expect(record.state(now: confirmedAt + 1, bootID: nil).phase == .armed)
    }

    @Test("a lapsed request cannot be cancelled")
    func lapsedCancel() {
        var record = ProtectionRecord(armed: true)
        _ = record.requestDisarm(now: 0, bootID: boot)
        let lapsed = ProtectionSchedule.cooldownMillis + ProtectionSchedule.readyWindowMillis
        #expect(record.cancelDisarm(now: lapsed, bootID: boot) == nil)
        #expect(record.cancelDisarm(now: 1, bootID: "boot-b") == nil)
    }

    @Test("a request from an earlier boot cannot be confirmed")
    func rebootVoidsRequest() {
        var record = ProtectionRecord(armed: true)
        _ = record.requestDisarm(now: 0, bootID: boot)
        let ready = ProtectionSchedule.cooldownMillis
        #expect(record.confirmDisarm(now: ready, bootID: "boot-b") == nil)
    }

    @Test("an unknown boot cannot start a request")
    func unknownBoot() {
        var record = ProtectionRecord(armed: true)
        #expect(record.requestDisarm(now: 0, bootID: nil) == nil)
    }

    @Test("the boot identifier is readable and stable within a boot")
    func bootIDReadable() {
        #expect(ProtectionClock.bootID() != nil)
        #expect(ProtectionClock.bootID() == ProtectionClock.bootID())
    }

    @Test("a corrupt timestamp reads as armed instead of trapping")
    func overflowIsArmed() {
        for bad in [Int64.min, Int64.max] {
            #expect(ProtectionSchedule.evaluate(armed: true, requestedAt: nil, disarmedAt: bad, now: 5).phase == .armed)
            #expect(ProtectionSchedule.evaluate(armed: true, requestedAt: bad, disarmedAt: nil, now: 5).phase == .armed)
        }
    }
}
