import Foundation

public enum ProtectionPhase: String, Equatable, Sendable {
    case off
    case armed
    case disarmPending = "disarm_pending"
    case disarmReady = "disarm_ready"
    case disarmed
}

public struct ProtectionState: Equatable, Sendable {
    public let phase: ProtectionPhase
    public let remainingMillis: Int64

    public init(phase: ProtectionPhase, remainingMillis: Int64) {
        self.phase = phase
        self.remainingMillis = remainingMillis
    }

    public var guardActive: Bool {
        phase == .armed || phase == .disarmPending || phase == .disarmReady
    }
}

/// Turning protection down is requested, waits out a cooldown, and is then confirmed; arming is
/// immediate. Port of apps/mobile's `ProtectionSchedule`; both are held to
/// test-vectors/protection-schedule.tsv.
///
/// Inputs are start timestamps on `ProtectionClock`, never the wall clock, which the user can set.
/// A timestamp later than `now`, or one whose difference overflows, resolves to `.armed`.
public enum ProtectionSchedule {
    public static let cooldownMillis: Int64 = 15 * 60_000
    public static let readyWindowMillis: Int64 = 5 * 60_000
    public static let disarmWindowMillis: Int64 = 10 * 60_000

    public static func evaluate(
        armed: Bool, requestedAt: Int64?, disarmedAt: Int64?, now: Int64
    ) -> ProtectionState {
        if !armed { return ProtectionState(phase: .off, remainingMillis: 0) }
        let guarded = ProtectionState(phase: .armed, remainingMillis: 0)

        // The disarm timestamp outlives its window, so only a live one outranks a request.
        if let disarmedAt, let progress = elapsed(from: disarmedAt, to: now) {
            if progress < disarmWindowMillis {
                return ProtectionState(phase: .disarmed, remainingMillis: disarmWindowMillis - progress)
            }
        }

        guard let requestedAt, let progress = elapsed(from: requestedAt, to: now) else {
            return guarded
        }
        if progress < cooldownMillis {
            return ProtectionState(phase: .disarmPending, remainingMillis: cooldownMillis - progress)
        }
        if progress < cooldownMillis + readyWindowMillis {
            return ProtectionState(
                phase: .disarmReady, remainingMillis: cooldownMillis + readyWindowMillis - progress)
        }
        return guarded
    }

    private static func elapsed(from start: Int64, to now: Int64) -> Int64? {
        let (progress, overflow) = now.subtractingReportingOverflow(start)
        return overflow || progress < 0 ? nil : progress
    }
}

/// Milliseconds on a clock that survives sleep and cannot be set by the user.
/// `ProcessInfo.systemUptime` stops during sleep and so would stretch every cooldown by the time
/// the machine spent asleep; `CLOCK_MONOTONIC_RAW` keeps counting.
public enum ProtectionClock {
    public static func nowMillis() -> Int64 {
        Int64(clock_gettime_nsec_np(CLOCK_MONOTONIC_RAW) / 1_000_000)
    }

    /// Changes on every boot. The clock restarts at zero, so a timestamp from an earlier boot can
    /// land inside a live window once the new boot's clock catches up to it.
    public static func bootID() -> String? {
        var size = 0
        guard sysctlbyname("kern.bootsessionuuid", nil, &size, nil, 0) == 0, size > 1 else {
            return nil
        }
        var buffer = [CChar](repeating: 0, count: size)
        guard sysctlbyname("kern.bootsessionuuid", &buffer, &size, nil, 0) == 0 else { return nil }
        return String(cString: buffer)
    }
}

/// What is stored: a request and a grant are timestamps, not decisions, tied to the boot that
/// wrote them. A record that is missing evaluates to off, so the store behind it has to tell
/// "never armed" from "unreadable" and fail closed on the latter.
public struct ProtectionRecord: Equatable, Sendable {
    public var armed: Bool
    public var requestedAt: Int64?
    public var disarmedAt: Int64?
    public var bootID: String?

    public init(
        armed: Bool = false, requestedAt: Int64? = nil, disarmedAt: Int64? = nil,
        bootID: String? = nil
    ) {
        self.armed = armed
        self.requestedAt = requestedAt
        self.disarmedAt = disarmedAt
        self.bootID = bootID
    }

    /// Timestamps from another boot, or from an unknown one, are void.
    public func state(now: Int64, bootID current: String?) -> ProtectionState {
        let sameBoot = bootID != nil && bootID == current
        return ProtectionSchedule.evaluate(
            armed: armed, requestedAt: sameBoot ? requestedAt : nil,
            disarmedAt: sameBoot ? disarmedAt : nil, now: now)
    }

    /// Each transition returns the tamper code to record, or nil when it changed nothing.
    public mutating func arm() -> TamperCode? {
        let changed = !armed || requestedAt != nil || disarmedAt != nil
        armed = true
        requestedAt = nil
        disarmedAt = nil
        return changed ? .protectionArmed : nil
    }

    public mutating func requestDisarm(now: Int64, bootID current: String?) -> TamperCode? {
        guard state(now: now, bootID: current).phase == .armed, let current else { return nil }
        requestedAt = now
        disarmedAt = nil
        bootID = current
        return .disarmRequested
    }

    public mutating func confirmDisarm(now: Int64, bootID current: String?) -> TamperCode? {
        guard state(now: now, bootID: current).phase == .disarmReady else { return nil }
        disarmedAt = now
        requestedAt = nil
        return .disarmConfirmed
    }

    public mutating func cancelDisarm(now: Int64, bootID current: String?) -> TamperCode? {
        let phase = state(now: now, bootID: current).phase
        guard phase == .disarmPending || phase == .disarmReady else { return nil }
        requestedAt = nil
        return .disarmCancelled
    }
}
