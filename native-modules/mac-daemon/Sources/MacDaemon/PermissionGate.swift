import ApplicationServices
import CoreGraphics
import Foundation
import IOKit.hid

// MARK: - Vocabulary

/// TCC's view of one capability.
///
/// `undetermined` is the state a capability returns to after `tccutil reset`, and it is not the
/// same as `denied`: only `denied` means the user was asked and said no.
public enum PermissionState: String, Equatable, Sendable {
    case granted
    case denied
    case undetermined
}

/// The three permissions Layer 2 needs, in the order a status report should list them.
public enum Capability: String, CaseIterable, Equatable, Sendable {
    /// ScreenCaptureKit. Without it the render path cannot see anything at all.
    case screenRecording
    /// The Accessibility API — AX text extraction, window observation, fullscreen control.
    case accessibility
    /// Input Monitoring — the scroll signal that says the visible content changed.
    case inputMonitoring
}

/// Everything the daemon can learn about its own protection without holding any permission.
public struct PermissionSnapshot: Equatable, Sendable {
    public var screenRecording: PermissionState
    public var accessibility: PermissionState
    public var inputMonitoring: PermissionState

    /// Whether the person being protected can revoke all of the above themselves.
    public var protectedUserIsAdmin: Bool
    public var sipEnabled: Bool
    public var localUsers: [String]
    public var adminGroupMembers: [String]
    public var bootTime: Date

    public init(
        screenRecording: PermissionState,
        accessibility: PermissionState,
        inputMonitoring: PermissionState,
        protectedUserIsAdmin: Bool,
        sipEnabled: Bool,
        localUsers: [String],
        adminGroupMembers: [String],
        bootTime: Date
    ) {
        self.screenRecording = screenRecording
        self.accessibility = accessibility
        self.inputMonitoring = inputMonitoring
        self.protectedUserIsAdmin = protectedUserIsAdmin
        self.sipEnabled = sipEnabled
        self.localUsers = localUsers
        self.adminGroupMembers = adminGroupMembers
        self.bootTime = bootTime
    }

    public subscript(capability: Capability) -> PermissionState {
        switch capability {
        case .screenRecording: return screenRecording
        case .accessibility: return accessibility
        case .inputMonitoring: return inputMonitoring
        }
    }
}

/// What the daemon can honestly claim right now.
public enum ProtectionLevel: String, Equatable, Sendable {
    /// Capture works and the account model holds.
    case protected
    /// Capture works, but something makes it removable or reduces coverage.
    case weakened
    /// The render path cannot see the screen.
    case blind
}

public enum Weakness: Equatable, Sendable {
    case missingCapability(Capability)
    /// The protected user is in the `admin` group, so both layers are theirs to remove.
    case protectedUserIsAdministrator
    case systemIntegrityProtectionDisabled
}

public struct ProtectionAssessment: Equatable, Sendable {
    public let level: ProtectionLevel
    public let weaknesses: [Weakness]
}

/// A change worth writing to the tamper log.
public enum TamperEvent: Equatable, Sendable {
    case permissionGranted(Capability)
    /// A capability that was held is no longer held — the event this whole module exists for.
    case permissionLost(Capability)
    case systemIntegrityProtectionDisabled
    case systemIntegrityProtectionEnabled
    case localUserAdded(String)
    case localUserRemoved(String)
    case administratorAdded(String)
    case administratorRemoved(String)
    case protectedUserBecameAdministrator
    case rebooted(Date)
}

public enum PermissionGateError: Error, Equatable {
    /// A signal could not be read. Deliberately fatal: a snapshot that substitutes a default
    /// would have the tamper log claim a protection nobody measured.
    case signalUnreadable(signal: String, detail: String)
}

// MARK: - Pure parsers for the environment signals

/// Parsers for the four signals that need no permission at all.
///
/// All four were verified readable on macOS 26.5.2 with nothing granted. As with
/// `NetworkServices`, there is no I/O here — callers run the tool and pass the text in.
public enum SystemEnvironment {
    /// `csrutil(8)` prefixes its one-line report with this.
    private static let sipStatusPrefix = "System Integrity Protection status:"

    /// Apple reserves UIDs below this for system accounts; the first human account is 501.
    /// See `dscl(1)` and Apple's Open Directory user-account conventions.
    private static let firstNonSystemUID = 500

    /// `dscl(1)` reports a group with no members by refusing the key rather than printing it.
    private static let absentKeyMarker = "No such key"

    private static let groupMembershipKey = "GroupMembership:"

    /// `true` when SIP is fully on, `false` when it is off *or partially disabled*, `nil` when the
    /// output was not recognised — which must not be read as either.
    public static func parseSIPStatus(_ output: String) -> Bool? {
        for line in output.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard trimmed.hasPrefix(sipStatusPrefix) else { continue }

            let status = trimmed.dropFirst(sipStatusPrefix.count)
                .trimmingCharacters(in: .whitespaces)
                .lowercased()
            if status.hasPrefix("enabled") { return true }
            // "unknown (Custom Configuration)" means some protections are off. It is not enabled,
            // and calling it enabled would hide the state this signal exists to catch.
            if status.hasPrefix("disabled") || status.hasPrefix("unknown") { return false }
            return nil
        }
        return nil
    }

    /// Names of accounts a person could log in as, from `dscl . -list /Users UniqueID`.
    public static func parseUserList(_ output: String) -> [String] {
        output.split(separator: "\n").compactMap { line in
            let fields = line.split(whereSeparator: { $0 == " " || $0 == "\t" })
            guard fields.count >= 2, let uid = Int(fields[fields.count - 1]),
                uid >= firstNonSystemUID
            else { return nil }
            return String(fields[0])
        }
    }

    /// Member names from `dscl . -read /Groups/<name> GroupMembership`.
    public static func parseGroupMembership(_ output: String) -> [String] {
        for line in output.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            guard trimmed.hasPrefix(groupMembershipKey) else { continue }
            return trimmed.dropFirst(groupMembershipKey.count)
                .split(whereSeparator: { $0 == " " || $0 == "\t" })
                .map(String.init)
        }
        return []
    }

    /// The `sec` field of `sysctl kern.boottime`, which is seconds since the Unix epoch.
    ///
    /// Accepts both the keyed form and `sysctl -n` output, which omits the key.
    public static func parseBootTime(_ output: String) -> Date? {
        guard let secondsKey = output.range(of: "sec") else { return nil }

        var index = secondsKey.upperBound
        func skipSpaces() {
            while index < output.endIndex, output[index] == " " {
                index = output.index(after: index)
            }
        }

        skipSpaces()
        guard index < output.endIndex, output[index] == "=" else { return nil }
        index = output.index(after: index)
        skipSpaces()

        var digits = ""
        while index < output.endIndex, output[index].isNumber {
            digits.append(output[index])
            index = output.index(after: index)
        }
        guard let seconds = TimeInterval(digits) else { return nil }
        return Date(timeIntervalSince1970: seconds)
    }

    /// Whether the group read failed for a reason other than the group being empty.
    static func isGenuineFailure(_ result: CommandResult) -> Bool {
        !result.succeeded && !(result.standardOutput + result.standardError)
            .contains(absentKeyMarker)
    }
}

// MARK: - The permission edge

/// The three preflight calls, behind a protocol so everything above them is testable.
public protocol PermissionProbing: Sendable {
    /// Reads state **without prompting**. Never call the requesting variants from a scan path.
    func state(of capability: Capability) -> PermissionState
    /// Shows the system prompt. Onboarding only, once per capability, deliberately.
    func requestAccess(to capability: Capability)
}

/// Real implementation over the supported, non-prompting APIs.
///
/// `TCC.db` is deliberately not read: it is SIP-protected, reading it requires Full Disk Access —
/// a stronger permission than the ones being inspected — and its schema is private.
public struct SystemPermissionProbe: PermissionProbing {
    /// `kAXTrustedCheckOptionPrompt` from `HIServices/AXUIElement.h`, spelled as its literal value
    /// because the imported symbol is a mutable global that Swift 6 refuses to read concurrently.
    /// The string is the constant's documented public name and is API, not an implementation
    /// detail — see Apple's `AXIsProcessTrustedWithOptions` reference.
    private static let trustedCheckOptionPrompt = "AXTrustedCheckOptionPrompt"

    public init() {}

    public func state(of capability: Capability) -> PermissionState {
        switch capability {
        case .screenRecording:
            // CGPreflightScreenCaptureAccess returns a Bool and cannot distinguish "denied" from
            // "never asked". Not-granted is reported as undetermined because the recovery path
            // differs and undetermined is the recoverable one.
            return CGPreflightScreenCaptureAccess() ? .granted : .undetermined

        case .accessibility:
            // AXIsProcessTrusted is the non-prompting form; the options variant can prompt.
            return AXIsProcessTrusted() ? .granted : .undetermined

        case .inputMonitoring:
            // The only one of the three that reports all three states.
            switch IOHIDCheckAccess(kIOHIDRequestTypeListenEvent) {
            case kIOHIDAccessTypeGranted: return .granted
            case kIOHIDAccessTypeDenied: return .denied
            default: return .undetermined
            }
        }
    }

    public func requestAccess(to capability: Capability) {
        switch capability {
        case .screenRecording:
            // Cannot be shown twice: once denied, the only recovery is `settingsURL(for:)`.
            _ = CGRequestScreenCaptureAccess()
        case .accessibility:
            let options = [Self.trustedCheckOptionPrompt as CFString: true] as CFDictionary
            _ = AXIsProcessTrustedWithOptions(options)
        case .inputMonitoring:
            _ = IOHIDRequestAccess(kIOHIDRequestTypeListenEvent)
        }
    }
}

/// Test double for `PermissionProbing`, in the library so integration harnesses can use it too.
public final class FakePermissionProbe: PermissionProbing, @unchecked Sendable {
    private let lock = NSLock()
    private var states: [Capability: PermissionState]
    private var _requestedCapabilities: [Capability] = []

    public init(states: [Capability: PermissionState] = [:]) {
        self.states = states
    }

    public var requestedCapabilities: [Capability] {
        lock.lock()
        defer { lock.unlock() }
        return _requestedCapabilities
    }

    public func set(_ capability: Capability, to state: PermissionState) {
        lock.lock()
        defer { lock.unlock() }
        states[capability] = state
    }

    public func state(of capability: Capability) -> PermissionState {
        lock.lock()
        defer { lock.unlock() }
        return states[capability] ?? .undetermined
    }

    public func requestAccess(to capability: Capability) {
        lock.lock()
        defer { lock.unlock() }
        _requestedCapabilities.append(capability)
        states[capability] = .granted
    }
}

// MARK: - The gate

/// Reads the permission and environment signals, and turns two readings into reportable events.
///
/// The design rule this module follows is **watch outcomes, not routes**: revoking Screen
/// Recording in System Settings and running `tccutil reset` produce the same observable state, so
/// one poll detects both, and every future route to the same outcome for free.
public final class PermissionGate {
    /// A `kern.boottime` move smaller than this is a clock adjustment, not a reboot. The value is
    /// derived from the current time, so NTP corrections shift it by a second or two.
    private static let rebootTolerance: TimeInterval = 5

    /// The group whose membership defeats the whole tamper model.
    private static let adminGroup = "/Groups/admin"

    private let probe: PermissionProbing
    private let runner: CommandRunner
    private let protectedUser: String

    /// The reading the next poll is compared against.
    public private(set) var lastSnapshot: PermissionSnapshot?

    public init(
        probe: PermissionProbing = SystemPermissionProbe(),
        runner: CommandRunner = SystemCommandRunner(),
        protectedUser: String = NSUserName()
    ) {
        self.probe = probe
        self.runner = runner
        self.protectedUser = protectedUser
    }

    // MARK: Reading

    public func snapshot() throws -> PermissionSnapshot {
        let sipResult = try read(SystemTool.csrutil, ["status"], signal: "csrutil status")
        guard let sipEnabled = SystemEnvironment.parseSIPStatus(sipResult) else {
            throw PermissionGateError.signalUnreadable(
                signal: "csrutil status", detail: sipResult)
        }

        let userList = try read(
            SystemTool.dscl, [".", "-list", "/Users", "UniqueID"], signal: "dscl -list /Users")

        let groupResult = try runner.run(
            SystemTool.dscl, [".", "-read", Self.adminGroup, "GroupMembership"])
        // An empty group is reported as a missing key rather than an empty value, and that is not
        // a failure to read.
        if SystemEnvironment.isGenuineFailure(groupResult) {
            throw PermissionGateError.signalUnreadable(
                signal: "dscl -read \(Self.adminGroup)", detail: groupResult.standardError)
        }
        let admins = SystemEnvironment.parseGroupMembership(groupResult.standardOutput)

        let bootResult = try read(SystemTool.sysctl, ["kern.boottime"], signal: "sysctl kern.boottime")
        guard let bootTime = SystemEnvironment.parseBootTime(bootResult) else {
            throw PermissionGateError.signalUnreadable(
                signal: "sysctl kern.boottime", detail: bootResult)
        }

        return PermissionSnapshot(
            screenRecording: probe.state(of: .screenRecording),
            accessibility: probe.state(of: .accessibility),
            inputMonitoring: probe.state(of: .inputMonitoring),
            protectedUserIsAdmin: admins.contains(protectedUser),
            sipEnabled: sipEnabled,
            localUsers: SystemEnvironment.parseUserList(userList),
            adminGroupMembers: admins,
            bootTime: bootTime)
    }

    /// Take a reading and report what changed since the last one.
    ///
    /// The first poll reports nothing: there is no prior state, and inventing grant events for
    /// permissions that were already held would fill the tamper log with fiction on every start.
    @discardableResult
    public func poll() throws -> [TamperEvent] {
        let current = try snapshot()
        defer { lastSnapshot = current }
        guard let previous = lastSnapshot else { return [] }
        return Self.transitions(from: previous, to: current)
    }

    private func read(_ executable: String, _ arguments: [String], signal: String) throws -> String
    {
        let result = try runner.run(executable, arguments)
        guard result.succeeded else {
            throw PermissionGateError.signalUnreadable(
                signal: signal, detail: result.standardError)
        }
        return result.standardOutput
    }

    // MARK: Pure decisions

    public static func assess(_ snapshot: PermissionSnapshot) -> ProtectionAssessment {
        var weaknesses: [Weakness] = Capability.allCases
            .filter { snapshot[$0] != .granted }
            .map { Weakness.missingCapability($0) }

        if snapshot.protectedUserIsAdmin { weaknesses.append(.protectedUserIsAdministrator) }
        // SIP off makes TCC.db directly writable, which defeats every permission above it.
        if !snapshot.sipEnabled { weaknesses.append(.systemIntegrityProtectionDisabled) }

        let level: ProtectionLevel
        if snapshot.screenRecording != .granted {
            level = .blind
        } else {
            level = weaknesses.isEmpty ? .protected : .weakened
        }
        return ProtectionAssessment(level: level, weaknesses: weaknesses)
    }

    public static func transitions(
        from previous: PermissionSnapshot, to current: PermissionSnapshot
    ) -> [TamperEvent] {
        var events: [TamperEvent] = []

        // First, so everything below it can be read against the machine having restarted. The
        // Android lesson: a daemon that died and a machine that rebooted are different events.
        if abs(current.bootTime.timeIntervalSince(previous.bootTime)) > rebootTolerance {
            events.append(.rebooted(current.bootTime))
        }

        for capability in Capability.allCases {
            switch (previous[capability], current[capability]) {
            case (.granted, .granted):
                continue
            case (.granted, _):
                // Covers System Settings and `tccutil reset` alike: one is denied, the other
                // undetermined, and both mean the capability is gone.
                events.append(.permissionLost(capability))
            case (_, .granted):
                events.append(.permissionGranted(capability))
            default:
                continue
            }
        }

        if previous.sipEnabled != current.sipEnabled {
            events.append(
                current.sipEnabled
                    ? .systemIntegrityProtectionEnabled : .systemIntegrityProtectionDisabled)
        }

        if !previous.protectedUserIsAdmin && current.protectedUserIsAdmin {
            // Reported separately from the membership change that carries it, because this single
            // transition invalidates Layer 1 and Layer 2 at once.
            events.append(.protectedUserBecameAdministrator)
        }

        events += difference(previous.adminGroupMembers, current.adminGroupMembers)
            .map { $0.added ? .administratorAdded($0.name) : .administratorRemoved($0.name) }
        events += difference(previous.localUsers, current.localUsers)
            .map { $0.added ? .localUserAdded($0.name) : .localUserRemoved($0.name) }

        return events
    }

    /// Names that appeared and disappeared between two listings, in a stable order.
    private static func difference(_ previous: [String], _ current: [String])
        -> [(name: String, added: Bool)]
    {
        let before = Set(previous)
        let after = Set(current)
        return current.filter { !before.contains($0) }.map { ($0, true) }
            + previous.filter { !after.contains($0) }.map { ($0, false) }
    }

    /// The System Settings pane that can restore a capability.
    ///
    /// Screen Recording's prompt cannot be shown again once denied, so for that capability this
    /// link is the only recovery path there is.
    public static func settingsURL(for capability: Capability) -> URL {
        let anchor: String
        switch capability {
        case .screenRecording: anchor = "Privacy_ScreenCapture"
        case .accessibility: anchor = "Privacy_Accessibility"
        case .inputMonitoring: anchor = "Privacy_ListenEvent"
        }
        return URL(
            string: "x-apple.systempreferences:com.apple.preference.security?\(anchor)")!
    }
}
