import Foundation

/// A `launchd` job definition for one half of the daemon.
///
/// **Layer 1 and Layer 2 cannot be the same process.** Layer 1 wants root and no session; Layer 2
/// needs the user's GUI session — ScreenCaptureKit and AppKit have no window server without one —
/// and must not be root, because a TCC grant belongs to the logged-in user and root cannot be
/// given one. That split is the whole reason this type distinguishes two kinds of job.
public struct LaunchdJob: Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        /// Runs as root, at boot, with no login session. `/Library/LaunchDaemons`.
        case daemon
        /// Runs as the logged-in user inside their graphical session. `~/Library/LaunchAgents`.
        case agent(home: URL, uid: uid_t)
    }

    /// `launchctl` calls a graphical login session this. A job without it can be loaded into a
    /// background session, where every window-server call fails in a way that reads as a
    /// permission problem.
    private static let graphicalSessionType = "Aqua"

    public let label: String
    public let executable: URL
    public let arguments: [String]
    public let kind: Kind
    /// Where stdout and stderr go. launchd discards both without it.
    public let logPath: URL?

    public static func daemon(
        label: String, executable: URL, arguments: [String], logPath: URL? = nil
    ) -> LaunchdJob {
        LaunchdJob(
            label: label, executable: executable, arguments: arguments, kind: .daemon,
            logPath: logPath)
    }

    public static func agent(
        label: String, executable: URL, arguments: [String], home: URL, uid: uid_t,
        logPath: URL? = nil
    ) -> LaunchdJob {
        LaunchdJob(
            label: label, executable: executable, arguments: arguments,
            kind: .agent(home: home, uid: uid), logPath: logPath)
    }

    public var installPath: URL {
        let directory: URL
        switch kind {
        case .daemon:
            directory = URL(fileURLWithPath: "/Library/LaunchDaemons")
        case .agent(let home, _):
            directory = home.appendingPathComponent("Library/LaunchAgents")
        }
        return directory.appendingPathComponent("\(label).plist")
    }

    /// The `launchctl bootstrap` / `bootout` domain target.
    public var serviceTarget: String {
        switch kind {
        case .daemon: return "system/\(label)"
        case .agent(_, let uid): return "gui/\(uid)/\(label)"
        }
    }

    /// The domain half of the target, which `bootstrap` takes instead of the full service name.
    public var domainTarget: String {
        switch kind {
        case .daemon: return "system"
        case .agent(_, let uid): return "gui/\(uid)"
        }
    }

    public func plist() throws -> Data {
        var entries: [String: Any] = [
            "Label": label,
            // Absolute, always: launchd does not resolve a job's program through a PATH we
            // control, and a relative one would be resolved against an unrelated directory.
            "ProgramArguments": [executable.path] + arguments,
            "RunAtLoad": true,
            "KeepAlive": true,
        ]

        switch kind {
        case .daemon:
            entries["UserName"] = "root"
        case .agent:
            entries["LimitLoadToSessionType"] = Self.graphicalSessionType
        }

        if let logPath {
            entries["StandardOutPath"] = logPath.path
            entries["StandardErrorPath"] = logPath.path
        }

        return try PropertyListSerialization.data(
            fromPropertyList: entries, format: .xml, options: 0)
    }
}
