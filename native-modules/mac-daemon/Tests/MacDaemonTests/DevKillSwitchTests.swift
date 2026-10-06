#if HOLY_BLOCKER_DEV_BUILD
import Foundation
import Testing

@testable import MacDaemon

@Suite("DevKillSwitch")
struct DevKillSwitchTests {
    @Test("targets only the development build's own agent and daemon labels")
    func targetsOwnLabels() throws {
        let targets = try DevKillSwitch.targets(for: .development, uid: 501)
        #expect(
            targets == [
                "gui/501/com.holyblocker.daemon.dev.agent",
                "system/com.holyblocker.daemon.dev",
            ])
    }

    @Test("never names a launchd label the release build owns")
    func neverTargetsRelease() throws {
        let release = [BundleIdentity.holyBlocker.daemonLabel, BundleIdentity.holyBlocker.agentLabel]
        for target in try DevKillSwitch.targets(for: .development, uid: 501) {
            #expect(!release.contains { target.hasSuffix("/\($0)") })
        }
    }

    @Test("refuses the release identity")
    func refusesRelease() {
        #expect(throws: DevKillSwitch.Refusal.notDevelopmentBuild) {
            try DevKillSwitch.targets(for: .holyBlocker, uid: 501)
        }
    }

    @Test("boots out each target with launchctl by absolute path")
    func engageRunsBootout() throws {
        let runner = FakeCommandRunner()
        _ = try DevKillSwitch.engage(identity: .development, uid: 501, runner: runner)
        #expect(
            runner.invocations == [
                .init(
                    executable: "/bin/launchctl",
                    arguments: ["bootout", "gui/501/com.holyblocker.daemon.dev.agent"]),
                .init(
                    executable: "/bin/launchctl",
                    arguments: ["bootout", "system/com.holyblocker.daemon.dev"]),
            ])
    }

    @Test("runs nothing for the release identity")
    func engageRefusesRelease() {
        let runner = FakeCommandRunner()
        #expect(throws: DevKillSwitch.Refusal.notDevelopmentBuild) {
            try DevKillSwitch.engage(identity: .holyBlocker, uid: 501, runner: runner)
        }
        #expect(runner.invocations.isEmpty)
    }

    @Test("reports a failed bootout for one target and still attempts the other")
    func continuesAfterFailure() throws {
        let runner = FakeCommandRunner()
        runner.stub(
            where: { _, args in args.last?.hasPrefix("gui/") == true },
            result: CommandResult(exitCode: 5))
        let outcomes = try DevKillSwitch.engage(identity: .development, uid: 501, runner: runner)
        #expect(outcomes.map(\.exitCode) == [5, 0])
        #expect(runner.invocations.count == 2)
    }
}
#endif
