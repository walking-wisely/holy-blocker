#if HOLY_BLOCKER_DEV_BUILD
import Foundation

public enum DevKillSwitch {
    public enum Refusal: Error, Equatable {
        case notDevelopmentBuild
    }

    private static let launchctl = "/bin/launchctl"

    public static func targets(for identity: BundleIdentity, uid: uid_t) throws -> [String] {
        guard identity == .development else { throw Refusal.notDevelopmentBuild }
        return [
            LaunchdJob.agent(
                label: identity.agentLabel, executable: URL(fileURLWithPath: "/"), arguments: [],
                home: URL(fileURLWithPath: "/"), uid: uid
            ).serviceTarget,
            LaunchdJob.daemon(
                label: identity.daemonLabel, executable: URL(fileURLWithPath: "/"), arguments: []
            ).serviceTarget,
        ]
    }

    public static func engage(
        identity: BundleIdentity, uid: uid_t, runner: CommandRunner
    ) throws -> [CommandResult] {
        try targets(for: identity, uid: uid).map { try runner.run(launchctl, ["bootout", $0]) }
    }
}
#endif
