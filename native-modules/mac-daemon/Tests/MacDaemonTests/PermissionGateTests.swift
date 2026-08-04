import Foundation
import Testing

@testable import MacDaemon

// MARK: - Fixtures

/// Verbatim `csrutil status` output, captured on macOS 26.5.2.
private let sipEnabledOutput = "System Integrity Protection status: enabled.\n"
private let sipDisabledOutput = "System Integrity Protection status: disabled.\n"

/// `csrutil status` on a machine with individual protections toggled. The wording is Apple's.
private let sipCustomOutput = """
    System Integrity Protection status: unknown (Custom Configuration).

    Configuration:
    	Apple Internal: disabled
    	Kext Signing: enabled
    	Filesystem Protections: disabled
    	Debugging Restrictions: enabled
    	DTrace Restrictions: enabled
    	NVRAM Protections: enabled
    	BaseSystem Verification: enabled

    This is an unsupported configuration, likely to break in the future and leave your machine in an unknown state.
    """

/// Verbatim `dscl . -list /Users UniqueID` output, truncated to the interesting rows. The real
/// listing on the development machine is 133 entries, all but two of them system accounts.
private let userListOutput = """
    _accessoryupdater        278
    _amavisd                 83
    _analyticsd              263
    daemon                   1
    dutov                    501
    nobody                   -2
    root                     0
    """

/// Verbatim `dscl . -read /Groups/admin GroupMembership` output.
private let adminGroupOutput = "GroupMembership: root dutov _mbsetupuser\n"

/// Verbatim `sysctl kern.boottime` output.
private let bootTimeOutput =
    "kern.boottime: { sec = 1783665813, usec = 796245 } Fri Jul 10 09:43:33 2026\n"

private let bootTime = Date(timeIntervalSince1970: 1_783_665_813)

/// A snapshot of the configuration the tamper model actually asks for: capture granted, the
/// protected user standing as a standard user, SIP on.
private func makeSnapshot(
    screenRecording: PermissionState = .granted,
    accessibility: PermissionState = .granted,
    inputMonitoring: PermissionState = .granted,
    protectedUserIsAdmin: Bool = false,
    sipEnabled: Bool = true,
    localUsers: [String] = ["dutov"],
    adminGroupMembers: [String] = ["root", "partner"],
    bootTime: Date = bootTime
) -> PermissionSnapshot {
    PermissionSnapshot(
        screenRecording: screenRecording,
        accessibility: accessibility,
        inputMonitoring: inputMonitoring,
        protectedUserIsAdmin: protectedUserIsAdmin,
        sipEnabled: sipEnabled,
        localUsers: localUsers,
        adminGroupMembers: adminGroupMembers,
        bootTime: bootTime)
}

// MARK: - Parsers

@Suite("SystemEnvironment.parseSIPStatus")
struct ParseSIPStatusTests {
    @Test("reads the enabled state")
    func enabled() {
        #expect(SystemEnvironment.parseSIPStatus(sipEnabledOutput) == true)
    }

    @Test("reads the disabled state")
    func disabled() {
        #expect(SystemEnvironment.parseSIPStatus(sipDisabledOutput) == false)
    }

    @Test("treats a custom configuration as not protected")
    func customConfiguration() {
        // "unknown (Custom Configuration)" means some protections are off. Reporting that as
        // enabled would hide exactly the state the snapshot exists to notice.
        #expect(SystemEnvironment.parseSIPStatus(sipCustomOutput) == false)
    }

    @Test("returns nil for output it does not recognise")
    func unrecognised() {
        #expect(SystemEnvironment.parseSIPStatus("") == nil)
        #expect(SystemEnvironment.parseSIPStatus("csrutil: command not found") == nil)
    }
}

@Suite("SystemEnvironment.parseUserList")
struct ParseUserListTests {
    @Test("keeps only accounts a person could log in as")
    func filtersSystemAccounts() {
        // Everything below UID 500 is reserved for the system; a bypass account is created above
        // it. Listing all 133 rows would make a new account invisible in the noise.
        #expect(SystemEnvironment.parseUserList(userListOutput) == ["dutov"])
    }

    @Test("reads a second account created after the first")
    func findsAddedAccount() {
        let output = userListOutput + "\nescape                   502"
        #expect(SystemEnvironment.parseUserList(output) == ["dutov", "escape"])
    }

    @Test("skips rows without a numeric UID")
    func skipsMalformedRows() {
        #expect(SystemEnvironment.parseUserList("dutov\nbroken   nan\nother   700") == ["other"])
    }

    @Test("returns nothing for empty output")
    func empty() {
        #expect(SystemEnvironment.parseUserList("") == [])
    }
}

@Suite("SystemEnvironment.parseGroupMembership")
struct ParseGroupMembershipTests {
    @Test("splits the membership line into names")
    func splitsNames() {
        #expect(
            SystemEnvironment.parseGroupMembership(adminGroupOutput)
                == ["root", "dutov", "_mbsetupuser"])
    }

    @Test("returns nothing when the key is absent")
    func absentKey() {
        // dscl reports an empty group by refusing the key rather than printing an empty value.
        #expect(SystemEnvironment.parseGroupMembership("No such key: GroupMembership\n") == [])
    }
}

@Suite("SystemEnvironment.parseBootTime")
struct ParseBootTimeTests {
    @Test("reads the seconds field")
    func readsSeconds() {
        #expect(SystemEnvironment.parseBootTime(bootTimeOutput) == bootTime)
    }

    @Test("reads output from sysctl -n, which omits the key")
    func readsBareValue() {
        let output = "{ sec = 1783665813, usec = 796245 } Fri Jul 10 09:43:33 2026"
        #expect(SystemEnvironment.parseBootTime(output) == bootTime)
    }

    @Test("returns nil for output it does not recognise")
    func unrecognised() {
        #expect(SystemEnvironment.parseBootTime("kern.boottime: unavailable") == nil)
    }
}

// MARK: - Assessment

@Suite("PermissionGate.assess")
struct AssessmentTests {
    @Test("reports the intended configuration as protected")
    func fullyProtected() {
        let assessment = PermissionGate.assess(makeSnapshot())

        #expect(assessment.level == .protected)
        #expect(assessment.weaknesses.isEmpty)
    }

    @Test("reports blind when screen recording is not held")
    func blindWithoutCapture() {
        // Nothing in Layer 2 works without it, and the failure is silent: an ungranted
        // SCShareableContent returns no displays rather than an error.
        for state in [PermissionState.denied, .undetermined] {
            let assessment = PermissionGate.assess(makeSnapshot(screenRecording: state))

            #expect(assessment.level == .blind)
            #expect(assessment.weaknesses.contains(.missingCapability(.screenRecording)))
        }
    }

    @Test("reports weakened, not blind, when only the text and scroll paths are missing")
    func weakenedWithoutSecondaryCapabilities() {
        let assessment = PermissionGate.assess(
            makeSnapshot(accessibility: .denied, inputMonitoring: .undetermined))

        #expect(assessment.level == .weakened)
        #expect(
            assessment.weaknesses == [
                .missingCapability(.accessibility), .missingCapability(.inputMonitoring),
            ])
    }

    @Test("reports weakened when the protected user is an administrator")
    func adminUserIsWeakened() {
        // Every Layer 2 permission is then revocable in two clicks, and Layer 1's networksetup
        // changes need no root for an admin either — so the tamper model is decorative.
        let assessment = PermissionGate.assess(makeSnapshot(protectedUserIsAdmin: true))

        #expect(assessment.level == .weakened)
        #expect(assessment.weaknesses == [.protectedUserIsAdministrator])
    }

    @Test("never reports an admin user with the same level as a standard user")
    func adminAndStandardAreDistinguishable() {
        // The one outcome the product must not produce: the same status in both configurations.
        #expect(
            PermissionGate.assess(makeSnapshot(protectedUserIsAdmin: true)).level
                != PermissionGate.assess(makeSnapshot()).level)
    }

    @Test("reports weakened when SIP is off")
    func sipOffIsWeakened() {
        // SIP off makes TCC.db directly writable, which defeats every permission below it.
        let assessment = PermissionGate.assess(makeSnapshot(sipEnabled: false))

        #expect(assessment.level == .weakened)
        #expect(assessment.weaknesses == [.systemIntegrityProtectionDisabled])
    }

    @Test("still lists the weaknesses that accompany a blind state")
    func blindStillListsWeaknesses() {
        let assessment = PermissionGate.assess(
            makeSnapshot(screenRecording: .denied, protectedUserIsAdmin: true, sipEnabled: false))

        #expect(assessment.level == .blind)
        #expect(
            assessment.weaknesses == [
                .missingCapability(.screenRecording),
                .protectedUserIsAdministrator,
                .systemIntegrityProtectionDisabled,
            ])
    }
}

// MARK: - Transitions

@Suite("PermissionGate.transitions")
struct TransitionTests {
    @Test("reports nothing when nothing changed")
    func steadyState() {
        #expect(PermissionGate.transitions(from: makeSnapshot(), to: makeSnapshot()).isEmpty)
    }

    @Test("reports a held permission being taken away")
    func permissionLost() {
        // The module's most important job. A revoked capture grant is indistinguishable from
        // "nothing bad is on screen" unless it is detected explicitly.
        let events = PermissionGate.transitions(
            from: makeSnapshot(), to: makeSnapshot(screenRecording: .denied))

        #expect(events == [.permissionLost(.screenRecording)])
    }

    @Test("treats a grant that became undetermined as a loss")
    func resetCountsAsLoss() {
        // `tccutil reset` returns a capability to undetermined rather than denied. Watching the
        // outcome rather than the route is what makes one poll cover both.
        let events = PermissionGate.transitions(
            from: makeSnapshot(), to: makeSnapshot(accessibility: .undetermined))

        #expect(events == [.permissionLost(.accessibility)])
    }

    @Test("does not report a capability that was never held")
    func neverHeldIsNotALoss() {
        let before = makeSnapshot(inputMonitoring: .undetermined)
        let after = makeSnapshot(inputMonitoring: .denied)

        #expect(PermissionGate.transitions(from: before, to: after).isEmpty)
    }

    @Test("reports a capability being granted")
    func permissionGranted() {
        let events = PermissionGate.transitions(
            from: makeSnapshot(screenRecording: .undetermined), to: makeSnapshot())

        #expect(events == [.permissionGranted(.screenRecording)])
    }

    @Test("reports SIP being turned off and back on")
    func sipChanges() {
        let on = makeSnapshot()
        let off = makeSnapshot(sipEnabled: false)

        #expect(
            PermissionGate.transitions(from: on, to: off)
                == [.systemIntegrityProtectionDisabled])
        #expect(
            PermissionGate.transitions(from: off, to: on)
                == [.systemIntegrityProtectionEnabled])
    }

    @Test("reports accounts appearing and disappearing")
    func userChanges() {
        let before = makeSnapshot(localUsers: ["dutov"])
        let after = makeSnapshot(localUsers: ["dutov", "escape"])

        #expect(PermissionGate.transitions(from: before, to: after) == [.localUserAdded("escape")])
        #expect(
            PermissionGate.transitions(from: after, to: before) == [.localUserRemoved("escape")])
    }

    @Test("reports admin group membership changing")
    func adminChanges() {
        let before = makeSnapshot(adminGroupMembers: ["root", "partner"])
        let after = makeSnapshot(adminGroupMembers: ["root", "partner", "dutov"])

        #expect(
            PermissionGate.transitions(from: before, to: after) == [.administratorAdded("dutov")])
        #expect(
            PermissionGate.transitions(from: after, to: before)
                == [.administratorRemoved("dutov")])
    }

    @Test("calls out the protected user self-granting admin")
    func selfGrantedAdmin() {
        // This one transition invalidates both layers at once, so it is reported separately from
        // the membership change that carries it.
        let events = PermissionGate.transitions(
            from: makeSnapshot(),
            to: makeSnapshot(protectedUserIsAdmin: true, adminGroupMembers: ["root", "dutov"]))

        #expect(events.contains(.protectedUserBecameAdministrator))
    }

    @Test("reports a reboot")
    func reboot() {
        let after = makeSnapshot(bootTime: bootTime.addingTimeInterval(3600))

        #expect(
            PermissionGate.transitions(from: makeSnapshot(), to: after)
                == [.rebooted(after.bootTime)])
    }

    @Test("does not call a clock adjustment a reboot")
    func clockDriftIsNotAReboot() {
        // kern.boottime is derived from the current clock, so an NTP correction moves it by a
        // second or two without the machine having gone anywhere.
        let after = makeSnapshot(bootTime: bootTime.addingTimeInterval(2))

        #expect(PermissionGate.transitions(from: makeSnapshot(), to: after).isEmpty)
    }

    @Test("puts the reboot first so later events can be read against it")
    func rebootIsReportedFirst() {
        let after = makeSnapshot(
            screenRecording: .undetermined, bootTime: bootTime.addingTimeInterval(3600))

        #expect(
            PermissionGate.transitions(from: makeSnapshot(), to: after)
                == [.rebooted(after.bootTime), .permissionLost(.screenRecording)])
    }
}

// MARK: - Recovery deep links

@Suite("PermissionGate.settingsURL")
struct SettingsURLTests {
    @Test("points at the pane that can restore each capability")
    func panePerCapability() {
        // Screen Recording's prompt cannot be shown a second time once denied, so this link is
        // the only recovery path there is.
        #expect(
            PermissionGate.settingsURL(for: .screenRecording).absoluteString
                == "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        #expect(
            PermissionGate.settingsURL(for: .accessibility).absoluteString
                == "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        #expect(
            PermissionGate.settingsURL(for: .inputMonitoring).absoluteString
                == "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
    }
}

// MARK: - Composition over the edges

@Suite("PermissionGate.snapshot")
struct SnapshotCompositionTests {
    /// `FakeCommandRunner` answers with the first stub that matches, so a caller that wants a
    /// different `csrutil` result has to pass it in rather than register a second one.
    private func makeRunner(
        csrutil: CommandResult = CommandResult(exitCode: 0, standardOutput: sipEnabledOutput)
    ) -> FakeCommandRunner {
        let runner = FakeCommandRunner()
        runner.stub(
            where: { executable, _ in executable == SystemTool.csrutil }, result: csrutil)
        runner.stub(
            where: { _, args in args.contains("/Users") },
            result: CommandResult(exitCode: 0, standardOutput: userListOutput))
        runner.stub(
            where: { _, args in args.contains("/Groups/admin") },
            result: CommandResult(exitCode: 0, standardOutput: adminGroupOutput))
        runner.stub(
            where: { executable, _ in executable == SystemTool.sysctl },
            result: CommandResult(exitCode: 0, standardOutput: bootTimeOutput))
        return runner
    }

    @Test("reads every signal without asking for a permission")
    func readsAllSignals() throws {
        let probe = FakePermissionProbe(
            states: [.screenRecording: .granted, .accessibility: .denied])
        let gate = PermissionGate(probe: probe, runner: makeRunner(), protectedUser: "dutov")

        let snapshot = try gate.snapshot()

        #expect(snapshot.screenRecording == .granted)
        #expect(snapshot.accessibility == .denied)
        #expect(snapshot.inputMonitoring == .undetermined)
        #expect(snapshot.sipEnabled)
        #expect(snapshot.localUsers == ["dutov"])
        #expect(snapshot.adminGroupMembers == ["root", "dutov", "_mbsetupuser"])
        #expect(snapshot.bootTime == bootTime)
        // The whole point of these four signals: none of them needed a grant.
        #expect(probe.requestedCapabilities.isEmpty)
    }

    @Test("derives admin status from the group the protected user is actually in")
    func derivesAdminStatus() throws {
        let inGroup = PermissionGate(probe: FakePermissionProbe(), runner: makeRunner(),
            protectedUser: "dutov")
        let notInGroup = PermissionGate(probe: FakePermissionProbe(), runner: makeRunner(),
            protectedUser: "partner")

        #expect(try inGroup.snapshot().protectedUserIsAdmin)
        #expect(try notInGroup.snapshot().protectedUserIsAdmin == false)
    }

    @Test("never reads TCC.db")
    func doesNotReadTCCDatabase() throws {
        // Reading it needs Full Disk Access — a stronger permission than the ones being
        // inspected — and its schema is private and has changed across releases.
        let runner = makeRunner()
        _ = try PermissionGate(probe: FakePermissionProbe(), runner: runner,
            protectedUser: "dutov").snapshot()

        for invocation in runner.invocations {
            #expect(!invocation.arguments.joined().contains("TCC.db"))
            #expect(!invocation.executable.contains("sqlite"))
        }
    }

    @Test("fails rather than guessing when a signal cannot be read")
    func failsOnUnreadableSignal() {
        // A snapshot that quietly reports SIP enabled because csrutil was missing would make the
        // tamper log claim a protection that was never measured.
        let runner = makeRunner(csrutil: CommandResult(exitCode: 1, standardError: "not found"))

        #expect(throws: PermissionGateError.self) {
            try PermissionGate(probe: FakePermissionProbe(), runner: runner,
                protectedUser: "dutov").snapshot()
        }
    }

    @Test("reports transitions against the previous poll")
    func pollReportsTransitions() throws {
        let probe = FakePermissionProbe(states: [.screenRecording: .granted])
        let gate = PermissionGate(probe: probe, runner: makeRunner(), protectedUser: "dutov")

        // The first poll has nothing to compare against and must not invent a grant event.
        #expect(try gate.poll().isEmpty)

        probe.set(.screenRecording, to: .undetermined)

        #expect(try gate.poll() == [.permissionLost(.screenRecording)])
        #expect(try gate.poll().isEmpty)
    }
}
