import Foundation

// MARK: - Backoff

/// Restart pacing for a proxy that keeps dying.
public struct RestartBackoff: Equatable, Sendable {
    public let baseSeconds: Double
    public let capSeconds: Double
    /// Consecutive failed starts tolerated before the supervisor gives up and restores settings.
    public let maxAttempts: Int

    public static let `default` = RestartBackoff(
        baseSeconds: 0.5, capSeconds: 30, maxAttempts: 5)

    public init(baseSeconds: Double, capSeconds: Double, maxAttempts: Int) {
        self.baseSeconds = baseSeconds
        self.capSeconds = capSeconds
        self.maxAttempts = maxAttempts
    }

    public func delay(forAttempt attempt: Int) -> Double {
        let exponent = Double(max(attempt, 1) - 1)
        // pow overflows to +inf for large exponents; min() with the cap absorbs that, but the
        // exponent is clamped anyway so the intermediate stays finite.
        return min(capSeconds, baseSeconds * pow(2, min(exponent, 32)))
    }
}

// MARK: - The pure ordering machine

/// Where the supervisor is in the launch / configure / restore lifecycle.
public enum SupervisorState: Equatable, Sendable {
    case stopped
    /// Child launched, listener not yet answering.
    case starting
    /// Listener answering, system proxy settings not yet applied.
    case healthy
    /// Listener answering and traffic routed to it.
    case configured
    /// Settings are being put back; the child outlives this state deliberately.
    case restoring
}

public enum SupervisorEvent: Equatable, Sendable {
    case start
    case healthy
    case healthCheckFailed
    case settingsApplied
    case proxyExited
    case stop
    case restoreCompleted
}

public enum SupervisorAction: Equatable, Sendable {
    case launchProxy
    case probeHealth
    case applyProxySettings
    case restoreProxySettings
    case terminateProxy
    case waitBeforeRestart(seconds: Double, attempt: Int)
    case giveUp(reason: String)
}

/// The ordering rules, with no process, socket, or clock anywhere near them.
///
/// Two orderings carry the safety of this layer and are the reason this is a separate type:
///
/// 1. **Configure only after healthy.** Pointing the system at a listener that has not bound yet
///    black-holes every request for the length of the gap.
/// 2. **Restore before terminating.** Killing the child first leaves a window in which traffic is
///    still routed at a port with nothing behind it.
///
/// A third rule is less obvious and was the easiest thing to get wrong: settings are applied
/// exactly once per run. `ProxyConfiguration.apply` snapshots the current settings before writing
/// its own, so re-applying after a restart would capture *our* proxy as the machine's prior state
/// and `restore()` would then pin the user to a dead local port permanently.
public struct ProxySupervisorMachine {
    public private(set) var state: SupervisorState = .stopped

    /// Whether this run owns an on-disk snapshot that has to be replayed before stopping.
    private var hasConfiguredSettings = false
    private var restartAttempts = 0
    private let backoff: RestartBackoff

    public init(backoff: RestartBackoff = .default) {
        self.backoff = backoff
    }

    public mutating func handle(_ event: SupervisorEvent) -> [SupervisorAction] {
        switch (state, event) {
        case (.stopped, .start):
            state = .starting
            return [.launchProxy, .probeHealth]

        case (.starting, .healthy):
            restartAttempts = 0
            guard !hasConfiguredSettings else {
                // Came back from a restart; the routing is still in place from the first run.
                state = .configured
                return []
            }
            state = .healthy
            return [.applyProxySettings]

        case (.healthy, .settingsApplied):
            hasConfiguredSettings = true
            state = .configured
            return []

        case (.starting, .healthCheckFailed), (.starting, .proxyExited),
            (.healthy, .healthCheckFailed), (.healthy, .proxyExited),
            (.configured, .healthCheckFailed), (.configured, .proxyExited):
            return retry()

        case (.starting, .stop), (.healthy, .stop), (.configured, .stop):
            return beginShutdown()

        case (.restoring, .restoreCompleted):
            hasConfiguredSettings = false
            restartAttempts = 0
            state = .stopped
            return [.terminateProxy]

        default:
            return []
        }
    }

    private mutating func retry() -> [SupervisorAction] {
        restartAttempts += 1
        guard restartAttempts <= backoff.maxAttempts else {
            let reason =
                "proxy failed to stay alive after \(backoff.maxAttempts) restart attempts"
            // Fail open on the network path: an unfiltered network beats a broken one.
            return [.giveUp(reason: reason)] + beginShutdown()
        }

        state = .starting
        return [
            .terminateProxy,
            .waitBeforeRestart(
                seconds: backoff.delay(forAttempt: restartAttempts), attempt: restartAttempts),
            .launchProxy,
            .probeHealth,
        ]
    }

    private mutating func beginShutdown() -> [SupervisorAction] {
        guard hasConfiguredSettings else {
            // Nothing was written this run. Calling restore here would consume a snapshot left
            // behind by an earlier crashed run, which is not ours to spend.
            state = .stopped
            return [.terminateProxy]
        }
        state = .restoring
        return [.restoreProxySettings]
    }
}

// MARK: - Edges

/// The proxy child process.
public protocol ProxyProcessHandle: AnyObject {
    func launch() throws
    func terminate()
    var isRunning: Bool { get }
}

/// A TCP reachability check against the proxy's listener.
public protocol ListenerProbe {
    func isListening(host: String, port: Int) -> Bool
}

/// System proxy settings, narrowed to what the supervisor needs.
public protocol ProxySettingsControlling {
    func apply() throws
    func restore() throws
}

public protocol Sleeper {
    func sleep(seconds: Double)
}

public struct RealSleeper: Sleeper, Sendable {
    public init() {}
    public func sleep(seconds: Double) { Thread.sleep(forTimeInterval: seconds) }
}

/// How hard to poll the listener before declaring one start attempt failed.
public struct HealthCheckPolicy: Equatable, Sendable {
    public let attempts: Int
    public let intervalSeconds: Double

    public static let `default` = HealthCheckPolicy(attempts: 20, intervalSeconds: 0.25)

    public init(attempts: Int, intervalSeconds: Double) {
        self.attempts = attempts
        self.intervalSeconds = intervalSeconds
    }
}

// MARK: - Executor

/// Runs `ProxySupervisorMachine` against the real world.
///
/// Deliberately thin: every decision belongs to the machine, and everything here is the mechanical
/// execution of an action it emitted.
public final class ProxySupervisor {
    private var machine: ProxySupervisorMachine
    private let host: String
    private let port: Int
    private let process: ProxyProcessHandle
    private let probe: ListenerProbe
    private let settings: ProxySettingsControlling
    private let sleeper: Sleeper
    private let healthCheck: HealthCheckPolicy

    /// Set when the machine gave up, so a caller can report *why* the daemon stopped.
    public private(set) var lastFailure: String?

    public var state: SupervisorState { machine.state }

    public init(
        host: String,
        port: Int,
        process: ProxyProcessHandle,
        probe: ListenerProbe,
        settings: ProxySettingsControlling,
        backoff: RestartBackoff = .default,
        sleeper: Sleeper = RealSleeper(),
        healthCheck: HealthCheckPolicy = .default
    ) {
        self.machine = ProxySupervisorMachine(backoff: backoff)
        self.host = host
        self.port = port
        self.process = process
        self.probe = probe
        self.settings = settings
        self.sleeper = sleeper
        self.healthCheck = healthCheck
    }

    /// Launch, wait for the listener, and route traffic to it. Returns once the machine settles,
    /// either at `.configured` or back at `.stopped` after giving up.
    public func start() throws {
        try dispatch(.start)
    }

    /// Put the machine's settings back and stop the child.
    public func stop() throws {
        try dispatch(.stop)
    }

    /// Poll for an unexpected child exit. The caller drives this; the supervisor owns no thread.
    public func checkLiveness() throws {
        guard state != .stopped, !process.isRunning else { return }
        try dispatch(.proxyExited)
    }

    private func dispatch(_ event: SupervisorEvent) throws {
        var followUps: [SupervisorEvent] = []
        for action in machine.handle(event) {
            if let followUp = try execute(action) { followUps.append(followUp) }
        }
        // Recursion is bounded: every restart path increments an attempt counter the machine caps.
        for followUp in followUps { try dispatch(followUp) }
    }

    private func execute(_ action: SupervisorAction) throws -> SupervisorEvent? {
        switch action {
        case .launchProxy:
            try process.launch()
            return nil

        case .probeHealth:
            return pollListener() ? .healthy : .healthCheckFailed

        case .applyProxySettings:
            try settings.apply()
            return .settingsApplied

        case .restoreProxySettings:
            try settings.restore()
            return .restoreCompleted

        case .terminateProxy:
            process.terminate()
            return nil

        case .waitBeforeRestart(let seconds, _):
            sleeper.sleep(seconds: seconds)
            return nil

        case .giveUp(let reason):
            lastFailure = reason
            return nil
        }
    }

    private func pollListener() -> Bool {
        for attempt in 1...max(healthCheck.attempts, 1) {
            if probe.isListening(host: host, port: port) { return true }
            if attempt < healthCheck.attempts { sleeper.sleep(seconds: healthCheck.intervalSeconds) }
        }
        return false
    }
}
