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
/// A stored timestamp later than `now` means the clock restarted, and both such cases resolve to
/// `.armed`.
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
        if let disarmedAt {
            let progress = now - disarmedAt
            if progress >= 0 && progress < disarmWindowMillis {
                return ProtectionState(phase: .disarmed, remainingMillis: disarmWindowMillis - progress)
            }
        }

        guard let requestedAt, now >= requestedAt else { return guarded }
        let progress = now - requestedAt
        if progress < cooldownMillis {
            return ProtectionState(phase: .disarmPending, remainingMillis: cooldownMillis - progress)
        }
        if progress < cooldownMillis + readyWindowMillis {
            return ProtectionState(
                phase: .disarmReady, remainingMillis: cooldownMillis + readyWindowMillis - progress)
        }
        return guarded
    }
}

/// Milliseconds on a clock that survives sleep and cannot be set by the user.
/// `ProcessInfo.systemUptime` stops during sleep and so would stretch every cooldown by the time
/// the machine spent asleep; `CLOCK_MONOTONIC_RAW` keeps counting.
public enum ProtectionClock {
    public static func nowMillis() -> Int64 {
        Int64(clock_gettime_nsec_np(CLOCK_MONOTONIC_RAW) / 1_000_000)
    }
}

/// What is stored: a request and a grant are timestamps, not decisions. Losing the record leaves
/// `armed` false only if it was never armed, and dropping a pending request only makes it stricter.
public struct ProtectionRecord: Equatable, Sendable {
    public var armed: Bool
    public var requestedAt: Int64?
    public var disarmedAt: Int64?

    public init(armed: Bool = false, requestedAt: Int64? = nil, disarmedAt: Int64? = nil) {
        self.armed = armed
        self.requestedAt = requestedAt
        self.disarmedAt = disarmedAt
    }

    public func state(now: Int64) -> ProtectionState {
        ProtectionSchedule.evaluate(
            armed: armed, requestedAt: requestedAt, disarmedAt: disarmedAt, now: now)
    }

    /// Each transition returns the tamper code to record, or nil when it changed nothing.
    public mutating func arm() -> TamperCode {
        armed = true
        requestedAt = nil
        disarmedAt = nil
        return .protectionArmed
    }

    public mutating func requestDisarm(now: Int64) -> TamperCode? {
        guard state(now: now).phase == .armed else { return nil }
        requestedAt = now
        return .disarmRequested
    }

    public mutating func confirmDisarm(now: Int64) -> TamperCode? {
        guard state(now: now).phase == .disarmReady else { return nil }
        disarmedAt = now
        requestedAt = nil
        return .disarmConfirmed
    }

    public mutating func cancelDisarm() -> TamperCode? {
        guard requestedAt != nil else { return nil }
        requestedAt = nil
        return .disarmCancelled
    }
}
