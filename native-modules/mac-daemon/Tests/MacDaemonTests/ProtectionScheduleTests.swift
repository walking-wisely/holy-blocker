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
        #expect(ProtectionSchedule.cooldownMillis >= 10 * 60_000)
        #expect(ProtectionSchedule.disarmWindowMillis >= 5 * 60_000)
    }

    @Test("the clock counts through sleep, unlike systemUptime")
    func clockIsNotUptime() {
        let a = ProtectionClock.nowMillis()
        let b = ProtectionClock.nowMillis()
        #expect(b >= a)
        #expect(a >= Int64(ProcessInfo.processInfo.systemUptime * 1000) - 1000)
    }

    @Test("disarming needs request, cooldown and confirmation, in that order")
    func transitions() {
        var record = ProtectionRecord()
        #expect(record.requestDisarm(now: 0) == nil)
        #expect(record.arm() == .protectionArmed)
        #expect(record.confirmDisarm(now: 0) == nil)
        #expect(record.requestDisarm(now: 1_000) == .disarmRequested)
        #expect(record.requestDisarm(now: 2_000) == nil)
        #expect(record.requestedAt == 1_000)
        #expect(record.confirmDisarm(now: 1_000 + ProtectionSchedule.cooldownMillis - 1) == nil)
        let ready = 1_000 + ProtectionSchedule.cooldownMillis
        #expect(record.confirmDisarm(now: ready) == .disarmConfirmed)
        #expect(record.state(now: ready).phase == .disarmed)
        #expect(record.confirmDisarm(now: ready) == nil)
    }

    @Test("arming clears an open window and cancel only logs a real request")
    func armAndCancel() {
        var record = ProtectionRecord(armed: true, requestedAt: nil, disarmedAt: 1_000)
        #expect(record.arm() == .protectionArmed)
        #expect(record.disarmedAt == nil)
        #expect(record.cancelDisarm() == nil)
        _ = record.requestDisarm(now: 5)
        #expect(record.cancelDisarm() == .disarmCancelled)
        #expect(record.state(now: 6).phase == .armed)
    }

    @Test("a second disarm works after the first window has expired")
    func repeatable() {
        var record = ProtectionRecord(armed: true)
        _ = record.requestDisarm(now: 0)
        let first = ProtectionSchedule.cooldownMillis
        _ = record.confirmDisarm(now: first)
        let later = first + ProtectionSchedule.disarmWindowMillis + 1
        #expect(record.requestDisarm(now: later) == .disarmRequested)
        #expect(record.state(now: later).phase == .disarmPending)
    }
}
