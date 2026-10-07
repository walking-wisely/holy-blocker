import Foundation

public enum TamperCode: String, CaseIterable, Equatable, Sendable {
    case daemonStarted = "daemon_on"
    case daemonStopped = "daemon_off"
    case uncleanStop = "unclean_stop"
    case permissionGranted = "permission_granted"
    case permissionLost = "permission_lost"
    case sipDisabled = "sip_off"
    case sipEnabled = "sip_on"
    case userAdded = "user_added"
    case userRemoved = "user_removed"
    case adminAdded = "admin_added"
    case adminRemoved = "admin_removed"
    case becameAdministrator = "became_administrator"
    case rebooted = "rebooted"
    case logTrimmed = "log_trimmed"

    var isSessionBoundary: Bool { self == .daemonStarted || self == .daemonStopped }
}

public struct TamperEntry: Equatable, Sendable {
    public let wallMillis: Int64
    public let uptimeMillis: Int64
    public let code: TamperCode
    public let detail: String

    public init(wallMillis: Int64, uptimeMillis: Int64, code: TamperCode, detail: String = "") {
        self.wallMillis = wallMillis
        self.uptimeMillis = uptimeMillis
        self.code = code
        self.detail = detail
    }
}

public enum SessionStart: Equatable, Sendable {
    case firstRun
    case cleanRestart
    case afterReboot
    case uncleanStop

    public var notableCode: TamperCode? { self == .uncleanStop ? .uncleanStop : nil }
}

/// Records what happened to the guard, never what was on the screen. Codes are the on-disk format;
/// details are closed identifiers the maintainer already has, never user, app, host or bundle names.
public enum TamperLog {
    private static let separator: Character = "\t"

    public static let maxDetailCharacters = 120
    public static let maxEntries = 2_000
    public static let classifyTail = 64

    public static func format(_ entry: TamperEntry) -> String {
        [
            String(entry.wallMillis), String(entry.uptimeMillis), entry.code.rawValue,
            sanitize(entry.detail),
        ].joined(separator: String(separator))
    }

    /// Tolerant on purpose: a process killed mid-append leaves a torn line, and that must cost
    /// that one entry only.
    public static func parse(_ line: String) -> TamperEntry? {
        let fields = line.split(separator: separator, maxSplits: 3, omittingEmptySubsequences: false)
        guard fields.count == 4,
            let wall = Int64(fields[0]),
            let uptime = Int64(fields[1]),
            let code = TamperCode(rawValue: String(fields[2]))
        else { return nil }
        return TamperEntry(
            wallMillis: wall, uptimeMillis: uptime, code: code, detail: String(fields[3]))
    }

    public static func readAll(_ lines: [String]) -> [TamperEntry] {
        lines.compactMap(parse)
    }

    public struct TrimResult: Equatable {
        public let kept: [String]
        public let dropped: Int
    }

    public static func trim(_ lines: [String], maxEntries: Int = maxEntries) -> TrimResult {
        guard lines.count > maxEntries else { return TrimResult(kept: lines, dropped: 0) }
        return TrimResult(kept: Array(lines.suffix(maxEntries)), dropped: lines.count - maxEntries)
    }

    /// Classifies the gap before this start from the last session boundary onward. Uptime is the
    /// only evidence of a reboot: it resets then and at no other time, so a machine that restarted
    /// and a daemon that was killed stay distinct.
    public static func classifyStart(recent: [TamperEntry], nowUptime: Int64) -> SessionStart {
        guard let boundary = recent.lastIndex(where: { $0.code.isSessionBoundary }) else {
            return .firstRun
        }
        let clock = recent[boundary...].map(\.uptimeMillis) + [nowUptime]
        if zip(clock, clock.dropFirst()).contains(where: { $1 < $0 }) { return .afterReboot }
        return recent[boundary].code == .daemonStopped ? .cleanRestart : .uncleanStop
    }

    /// User names are dropped: the log needs to show that an account appeared, not who it is.
    public static func record(for event: TamperEvent) -> (code: TamperCode, detail: String) {
        switch event {
        case .permissionGranted(let capability): return (.permissionGranted, capability.rawValue)
        case .permissionLost(let capability): return (.permissionLost, capability.rawValue)
        case .systemIntegrityProtectionDisabled: return (.sipDisabled, "")
        case .systemIntegrityProtectionEnabled: return (.sipEnabled, "")
        case .localUserAdded: return (.userAdded, "")
        case .localUserRemoved: return (.userRemoved, "")
        case .administratorAdded: return (.adminAdded, "")
        case .administratorRemoved: return (.adminRemoved, "")
        case .protectedUserBecameAdministrator: return (.becameAdministrator, "")
        case .rebooted: return (.rebooted, "")
        }
    }

    private static func sanitize(_ detail: String) -> String {
        String(
            detail.replacingOccurrences(
                of: "[\\t\\r\\n]+", with: " ", options: .regularExpression
            )
            .trimmingCharacters(in: .whitespaces)
            .prefix(maxDetailCharacters))
    }
}
