import Foundation
import Testing

@testable import MacDaemon

// MARK: - Backoff

@Suite("RestartBackoff")
struct RestartBackoffTests {
    @Test("doubles the delay with each attempt")
    func doubles() {
        let backoff = RestartBackoff(baseSeconds: 0.5, capSeconds: 30, maxAttempts: 5)
        #expect(backoff.delay(forAttempt: 1) == 0.5)
        #expect(backoff.delay(forAttempt: 2) == 1.0)
        #expect(backoff.delay(forAttempt: 3) == 2.0)
        #expect(backoff.delay(forAttempt: 4) == 4.0)
    }

    @Test("never exceeds the cap")
    func caps() {
        let backoff = RestartBackoff(baseSeconds: 1, capSeconds: 8, maxAttempts: 20)
        #expect(backoff.delay(forAttempt: 4) == 8)
        // Without a cap this would overflow to infinity long before attempt 2000.
        #expect(backoff.delay(forAttempt: 2000) == 8)
    }

    @Test("treats a non-positive attempt as the first one")
    func clampsLowAttempts() {
        let backoff = RestartBackoff(baseSeconds: 0.5, capSeconds: 30, maxAttempts: 5)
        #expect(backoff.delay(forAttempt: 0) == 0.5)
        #expect(backoff.delay(forAttempt: -3) == 0.5)
    }
}

// MARK: - The pure ordering machine

private let testBackoff = RestartBackoff(baseSeconds: 1, capSeconds: 8, maxAttempts: 3)

private func makeMachine() -> ProxySupervisorMachine {
    ProxySupervisorMachine(backoff: testBackoff)
}

/// Drives a machine to the `configured` state the way a healthy startup would.
private func startAndConfigure(_ machine: inout ProxySupervisorMachine) {
    _ = machine.handle(.start)
    _ = machine.handle(.healthy)
    _ = machine.handle(.settingsApplied)
}

@Suite("ProxySupervisorMachine startup ordering")
struct ProxySupervisorMachineStartupTests {
    @Test("starts stopped")
    func initialState() {
        #expect(makeMachine().state == .stopped)
    }

    @Test("launches the proxy before probing it")
    func launchesThenProbes() {
        var machine = makeMachine()
        let actions = machine.handle(.start)

        #expect(machine.state == .starting)
        #expect(actions == [.launchProxy, .probeHealth])
    }

    @Test("configures system proxy settings only once the listener is healthy")
    func configuresOnlyAfterHealthy() {
        var machine = makeMachine()

        // Configuring first and launching second would black-hole every request during the gap,
        // so nothing may touch system proxy settings until the listener answers.
        let start = machine.handle(.start)
        #expect(!start.contains(.applyProxySettings))

        let healthy = machine.handle(.healthy)
        #expect(machine.state == .healthy)
        #expect(healthy == [.applyProxySettings])

        #expect(machine.handle(.settingsApplied).isEmpty)
        #expect(machine.state == .configured)
    }

    @Test("ignores events that do not apply to the current state")
    func ignoresIrrelevantEvents() {
        var machine = makeMachine()

        #expect(machine.handle(.settingsApplied).isEmpty)
        #expect(machine.handle(.healthy).isEmpty)
        #expect(machine.state == .stopped)
    }
}

@Suite("ProxySupervisorMachine restart")
struct ProxySupervisorMachineRestartTests {
    @Test("retries with a backoff delay when the listener never answers")
    func retriesOnFailedProbe() {
        var machine = makeMachine()
        _ = machine.handle(.start)

        let actions = machine.handle(.healthCheckFailed)

        #expect(machine.state == .starting)
        #expect(
            actions == [
                .terminateProxy,
                .waitBeforeRestart(seconds: 1, attempt: 1),
                .launchProxy,
                .probeHealth,
            ])
    }

    @Test("lengthens the delay on each successive failure")
    func backoffGrows() {
        var machine = makeMachine()
        _ = machine.handle(.start)

        _ = machine.handle(.healthCheckFailed)
        let second = machine.handle(.healthCheckFailed)

        #expect(second.contains(.waitBeforeRestart(seconds: 2, attempt: 2)))
    }

    @Test("resets the attempt counter once the proxy comes back healthy")
    func resetsAttempts() {
        var machine = makeMachine()
        _ = machine.handle(.start)
        _ = machine.handle(.healthCheckFailed)
        _ = machine.handle(.healthCheckFailed)

        _ = machine.handle(.healthy)
        _ = machine.handle(.settingsApplied)
        let afterExit = machine.handle(.proxyExited)

        // A long-lived daemon that flaps once an hour must not accumulate its way to a give-up.
        #expect(afterExit.contains(.waitBeforeRestart(seconds: 1, attempt: 1)))
    }

    @Test("relaunches when the proxy exits after being configured")
    func relaunchesAfterExit() {
        var machine = makeMachine()
        startAndConfigure(&machine)

        let actions = machine.handle(.proxyExited)

        #expect(machine.state == .starting)
        #expect(actions.contains(.launchProxy))
    }

    @Test("does not re-apply system proxy settings after a restart")
    func doesNotReapplySettings() {
        var machine = makeMachine()
        startAndConfigure(&machine)

        _ = machine.handle(.proxyExited)
        let backHealthy = machine.handle(.healthy)

        // apply() snapshots the current settings before writing its own. Re-applying while our
        // proxy is already configured would capture 127.0.0.1 as the "prior" state and restore
        // would then point the machine at our own dead listener forever.
        #expect(!backHealthy.contains(.applyProxySettings))
        #expect(machine.state == .configured)
    }
}

@Suite("ProxySupervisorMachine give-up")
struct ProxySupervisorMachineGiveUpTests {
    @Test("restores proxy settings when it can no longer keep the proxy alive")
    func restoresOnGiveUp() {
        var machine = makeMachine()
        startAndConfigure(&machine)

        var actions: [SupervisorAction] = []
        for _ in 0..<testBackoff.maxAttempts + 1 {
            actions = machine.handle(.proxyExited)
        }

        // Fail open: leaving the machine pointed at a listener that will never come back is a
        // broken network, which is worse than an unfiltered one.
        #expect(machine.state == .restoring)
        #expect(actions.contains(.restoreProxySettings))
        #expect(actions.contains { if case .giveUp = $0 { return true } else { return false } })
    }

    @Test("stops without restoring when settings were never applied")
    func noRestoreWhenNeverConfigured() {
        var machine = makeMachine()
        _ = machine.handle(.start)

        var actions: [SupervisorAction] = []
        for _ in 0..<testBackoff.maxAttempts + 1 {
            actions = machine.handle(.healthCheckFailed)
        }

        // Nothing was ever written, so there is no snapshot to replay — and calling restore here
        // would consume a snapshot left behind by an earlier crashed run.
        #expect(machine.state == .stopped)
        #expect(!actions.contains(.restoreProxySettings))
        #expect(actions.contains(.terminateProxy))
    }

    @Test("terminates the child only after the restore completes")
    func terminatesAfterRestore() {
        var machine = makeMachine()
        startAndConfigure(&machine)
        for _ in 0..<testBackoff.maxAttempts + 1 { _ = machine.handle(.proxyExited) }

        let actions = machine.handle(.restoreCompleted)

        #expect(machine.state == .stopped)
        #expect(actions == [.terminateProxy])
    }
}

@Suite("ProxySupervisorMachine shutdown")
struct ProxySupervisorMachineShutdownTests {
    @Test("restores before terminating on a clean stop")
    func restoreBeforeTerminate() {
        var machine = makeMachine()
        startAndConfigure(&machine)

        let stopping = machine.handle(.stop)
        #expect(machine.state == .restoring)
        #expect(stopping == [.restoreProxySettings])

        // Terminating first would leave a window where the system still routes at a port with
        // nothing behind it.
        let finished = machine.handle(.restoreCompleted)
        #expect(machine.state == .stopped)
        #expect(finished == [.terminateProxy])
    }

    @Test("just terminates when stopped before settings were applied")
    func stopBeforeConfigured() {
        var machine = makeMachine()
        _ = machine.handle(.start)

        let actions = machine.handle(.stop)

        #expect(machine.state == .stopped)
        #expect(actions == [.terminateProxy])
    }

    @Test("a second stop is a no-op")
    func idempotentStop() {
        var machine = makeMachine()
        startAndConfigure(&machine)
        _ = machine.handle(.stop)
        _ = machine.handle(.restoreCompleted)

        #expect(machine.handle(.stop).isEmpty)
        #expect(machine.state == .stopped)
    }

    @Test("a restart after a full stop applies settings again")
    func restartAfterStopReconfigures() {
        var machine = makeMachine()
        startAndConfigure(&machine)
        _ = machine.handle(.stop)
        _ = machine.handle(.restoreCompleted)

        _ = machine.handle(.start)
        // The snapshot was consumed by the restore, so the next run has to take a fresh one.
        #expect(machine.handle(.healthy) == [.applyProxySettings])
    }
}

// MARK: - Executor fakes

/// Shared ordering log, so assertions can span the process, probe, and settings edges.
private final class Recorder: @unchecked Sendable {
    private let lock = NSLock()
    private var _entries: [String] = []

    var entries: [String] {
        lock.lock()
        defer { lock.unlock() }
        return _entries
    }

    func record(_ entry: String) {
        lock.lock()
        defer { lock.unlock() }
        _entries.append(entry)
    }
}

private final class FakeProxyProcess: ProxyProcessHandle, @unchecked Sendable {
    private let recorder: Recorder
    var isRunning = false
    var launchError: Error?

    init(recorder: Recorder) { self.recorder = recorder }

    func launch() throws {
        recorder.record("launch")
        if let launchError { throw launchError }
        isRunning = true
    }

    func terminate() {
        recorder.record("terminate")
        isRunning = false
    }
}

private final class FakeListenerProbe: ListenerProbe, @unchecked Sendable {
    private let recorder: Recorder
    /// Answers popped in order; the last one repeats once exhausted.
    var answers: [Bool]

    init(recorder: Recorder, answers: [Bool] = [true]) {
        self.recorder = recorder
        self.answers = answers
    }

    func isListening(host: String, port: Int) -> Bool {
        recorder.record("probe")
        return answers.count > 1 ? answers.removeFirst() : (answers.first ?? false)
    }
}

private final class FakeSettings: ProxySettingsControlling, @unchecked Sendable {
    private let recorder: Recorder
    var applyError: Error?

    init(recorder: Recorder) { self.recorder = recorder }

    func apply() throws {
        recorder.record("apply")
        if let applyError { throw applyError }
    }

    func restore() throws { recorder.record("restore") }
}

private final class FakeSleeper: Sleeper, @unchecked Sendable {
    private let lock = NSLock()
    private var _slept: [Double] = []

    var slept: [Double] {
        lock.lock()
        defer { lock.unlock() }
        return _slept
    }

    func sleep(seconds: Double) {
        lock.lock()
        defer { lock.unlock() }
        _slept.append(seconds)
    }
}

private struct Harness {
    let recorder = Recorder()
    let process: FakeProxyProcess
    let probe: FakeListenerProbe
    let settings: FakeSettings
    let sleeper = FakeSleeper()
    let supervisor: ProxySupervisor

    init(probeAnswers: [Bool] = [true], backoff: RestartBackoff = testBackoff) {
        let recorder = self.recorder
        process = FakeProxyProcess(recorder: recorder)
        probe = FakeListenerProbe(recorder: recorder, answers: probeAnswers)
        settings = FakeSettings(recorder: recorder)
        supervisor = ProxySupervisor(
            host: "127.0.0.1",
            port: 8080,
            process: process,
            probe: probe,
            settings: settings,
            backoff: backoff,
            sleeper: sleeper,
            healthCheck: HealthCheckPolicy(attempts: 2, intervalSeconds: 0.1))
    }
}

@Suite("ProxySupervisor execution")
struct ProxySupervisorExecutionTests {
    @Test("launches, probes, then configures — in that order")
    func happyPath() throws {
        let harness = Harness()

        try harness.supervisor.start()

        #expect(harness.supervisor.state == .configured)
        #expect(harness.recorder.entries == ["launch", "probe", "apply"])
    }

    @Test("keeps probing while the listener is still coming up")
    func waitsForListener() throws {
        // The proxy needs a moment to bind; one refused connection is not a failure.
        let harness = Harness(probeAnswers: [false, true])

        try harness.supervisor.start()

        #expect(harness.supervisor.state == .configured)
        #expect(harness.recorder.entries == ["launch", "probe", "probe", "apply"])
        #expect(harness.sleeper.slept == [0.1])
    }

    @Test("never touches proxy settings when the listener never comes up")
    func neverConfiguresWhenUnhealthy() throws {
        let harness = Harness(probeAnswers: [false])

        try harness.supervisor.start()

        #expect(harness.supervisor.state == .stopped)
        #expect(!harness.recorder.entries.contains("apply"))
        #expect(!harness.recorder.entries.contains("restore"))
    }

    @Test("sleeps for the backoff delay between restart attempts")
    func honoursBackoff() throws {
        let harness = Harness(probeAnswers: [false])

        try harness.supervisor.start()

        // Two health-check probes per attempt, then the growing inter-attempt delays.
        #expect(harness.sleeper.slept.contains(1))
        #expect(harness.sleeper.slept.contains(2))
    }

    @Test("restores settings before terminating on stop")
    func stopRestoresFirst() throws {
        let harness = Harness()
        try harness.supervisor.start()

        try harness.supervisor.stop()

        #expect(harness.supervisor.state == .stopped)
        let entries = harness.recorder.entries
        let restore = try #require(entries.firstIndex(of: "restore"))
        let terminate = try #require(entries.lastIndex(of: "terminate"))
        #expect(restore < terminate)
        #expect(!harness.process.isRunning)
    }

    @Test("relaunches when a liveness check finds the child gone")
    func livenessRestart() throws {
        let harness = Harness()
        try harness.supervisor.start()
        harness.process.isRunning = false

        try harness.supervisor.checkLiveness()

        #expect(harness.supervisor.state == .configured)
        #expect(harness.recorder.entries.filter { $0 == "launch" }.count == 2)
        // Still exactly one apply — the restart must not re-snapshot.
        #expect(harness.recorder.entries.filter { $0 == "apply" }.count == 1)
    }

    @Test("a liveness check on a healthy child does nothing")
    func livenessNoop() throws {
        let harness = Harness()
        try harness.supervisor.start()
        let before = harness.recorder.entries

        try harness.supervisor.checkLiveness()

        #expect(harness.recorder.entries == before)
    }

    @Test("restores settings when the proxy dies for good after being configured")
    func restoresAfterRepeatedDeath() throws {
        let harness = Harness()
        try harness.supervisor.start()
        // Every subsequent probe fails, so the restarts exhaust the attempt budget.
        harness.probe.answers = [false]
        harness.process.isRunning = false

        try harness.supervisor.checkLiveness()

        #expect(harness.supervisor.state == .stopped)
        #expect(harness.recorder.entries.contains("restore"))
    }

    @Test("surfaces a launch failure rather than reporting success")
    func launchFailurePropagates() {
        let harness = Harness()
        harness.process.launchError = CommandRunnerError.executableNotFound("/nope")

        #expect(throws: CommandRunnerError.self) { try harness.supervisor.start() }
    }
}
