package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SettingsGuardTest {
    private val self = "com.holyblocker.mobile"
    private val label = "Holy Blocker"

    private companion object {
        /** DeviceAdminAdd's own view ids, dumped from an android-36 emulator. */
        val DEVICE_ADMIN_IDS = setOf(
            "com.android.settings:id/admin_name",
            "com.android.settings:id/add_msg",
            "com.android.settings:id/admin_warning",
        )
    }

    private fun guard(
        profile: SettingsProfile? = SettingsProfiles.AOSP,
        // Active by default so the existing cases read as "steady state": the
        // admin is on and the screen that would remove it is guarded.
        deviceAdminActive: Boolean = true,
        // Likewise armed by default. The guard itself starts unarmed — see
        // `starts unarmed` — but every matching case below is about what it does
        // once the user has turned protection on.
        armed: Boolean = true,
    ) = SettingsGuard(
        profile = profile,
        selfPackage = self,
        selfLabel = label,
        isDeviceAdminActive = { deviceAdminActive },
    ).apply { setGuardActive(armed) }

    // Every identifier below was dumped from a running android-36 emulator, not
    // inferred. An earlier draft of this file used plausible-looking names and
    // almost none of them existed.

    /** The accessibility list — a dedicated activity. */
    private fun accessibilityScreen() = ScreenIdentity(
        packageName = "com.android.settings",
        className = "com.android.settings.Settings\$AccessibilitySettingsActivity",
        resourceIds = setOf(
            // Generic Settings chrome; present on every sub-page, so these must
            // not be what makes the match.
            "com.android.settings:id/recycler_view",
            "com.android.settings:id/content_frame",
        ),
    )

    /** The per-service page carrying our on/off switch — a generic host. */
    private fun serviceTogglePage() = ScreenIdentity(
        packageName = "com.android.settings",
        className = "com.android.settings.SubSettings",
        texts = listOf(label, "Use $label"),
    )

    private fun appInfoScreen() = ScreenIdentity(
        packageName = "com.android.settings",
        className = "com.android.settings.spa.SpaActivity",
        texts = listOf(label),
    )

    private fun deviceAdminScreen() = ScreenIdentity(
        packageName = "com.android.settings",
        className =
            "com.android.settings.applications.specialaccess.deviceadmin.DeviceAdminAdd",
        resourceIds = DEVICE_ADMIN_IDS,
        texts = listOf(label),
    )

    /**
     * The same screen as it actually arrives most of the time.
     *
     * Dumped from an android-36 emulator: opening the activation prompt emits
     * events carrying `android.widget.FrameLayout`, not the activity class. The
     * resource ids are present throughout, which is why they carry the match.
     */
    private fun deviceAdminScreenWithoutClass() = ScreenIdentity(
        packageName = "com.android.settings",
        className = "android.widget.FrameLayout",
        resourceIds = DEVICE_ADMIN_IDS,
        texts = listOf(label),
    )

    /**
     * The device-admin prompt for somebody else's app.
     *
     * Same activity and same view ids — "Find Hub" ships an admin on the stock
     * android-36 image, so this is the ordinary case, not a contrived one.
     */
    private fun otherAppDeviceAdminScreen() = ScreenIdentity(
        packageName = "com.android.settings",
        className =
            "com.android.settings.applications.specialaccess.deviceadmin.DeviceAdminAdd",
        resourceIds = DEVICE_ADMIN_IDS,
        texts = listOf("Find Hub", "Allow Find Hub to lock or erase a lost device"),
    )

    /**
     * The device-admin *list*, which is a different screen from the prompt.
     *
     * Its own alias class, as reported by the window-state event on an
     * android-36 emulator — not the `.Settings` target that alias resolves to.
     * The rows are `accessibilityDataSensitive`, so this screen only became
     * matchable at all once `isAccessibilityTool` was declared.
     */
    private fun deviceAdminListScreen() = ScreenIdentity(
        packageName = "com.android.settings",
        className = "com.android.settings.Settings\$DeviceAdminSettingsActivity",
        resourceIds = setOf(
            "com.android.settings:id/content_parent",
            "com.android.settings:id/recycler_view",
        ),
        texts = listOf("Find Hub", label),
    )

    private fun unrelatedScreen() = ScreenIdentity(
        packageName = "com.other.app",
        className = "com.other.app.MainActivity",
    )

    // --- matching ---------------------------------------------------------

    @Test
    fun `backs out of the accessibility settings screen`() {
        val decision = guard().evaluate(accessibilityScreen(), nowMillis = 0)

        assertEquals(GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS), decision)
    }

    @Test
    fun `backs out of our own app info screen`() {
        // Force-stop, clear data and uninstall all live here, and none of them
        // can be blocked by policy at plain Device Admin level.
        val decision = guard().evaluate(appInfoScreen(), nowMillis = 0)

        assertEquals(GuardDecision.BackOut(GuardedSurface.APP_INFO_SELF), decision)
    }

    @Test
    fun `backs out of the device admin list once the admin is active`() {
        // The list is where deactivation starts, and deactivation is what gates
        // uninstall. It names this app, so before isAccessibilityTool was
        // declared it should already have been caught by the self-mention
        // catch-all — it was not, because the rows carrying the name were
        // withheld from the service entirely.
        val decision = guard(deviceAdminActive = true)
            .evaluate(deviceAdminListScreen(), nowMillis = 0)

        assertEquals(GuardDecision.BackOut(GuardedSurface.DEVICE_ADMIN_SETTINGS), decision)
    }

    @Test
    fun `leaves the device admin list alone while the admin is inactive`() {
        // Same trap as the prompt, one screen earlier: the list is the route to
        // the activation prompt, so guarding it while the admin is off blocks
        // the only settings path that can turn the feature on. There is also
        // nothing to protect — with no admin active, nothing on this screen can
        // remove anything.
        //
        // Observed on an android-36 emulator once the rows became visible: the
        // list was ejected as SELF_IN_SETTINGS with the admin inactive.
        assertEquals(
            GuardDecision.Ignore,
            guard(deviceAdminActive = false).evaluate(deviceAdminListScreen(), nowMillis = 0),
        )
    }

    @Test
    fun `guards the device admin list for another app's admin only when it names us`() {
        // The list shows every admin on the device. It is guarded because *our*
        // row is on it, so a list that does not mention us is somebody else's
        // business.
        val theirs = deviceAdminListScreen().copy(texts = listOf("Find Hub"))

        assertEquals(
            GuardDecision.Ignore,
            guard(deviceAdminActive = true).evaluate(theirs, nowMillis = 0),
        )
    }

    @Test
    fun `backs out of the device admin screen once the admin is active`() {
        val decision = guard(deviceAdminActive = true)
            .evaluate(deviceAdminScreen(), nowMillis = 0)

        assertEquals(GuardDecision.BackOut(GuardedSurface.DEVICE_ADMIN_SETTINGS), decision)
    }

    @Test
    fun `allows the device admin screen while the admin is inactive`() {
        // DeviceAdminAdd is the same activity for activating and deactivating.
        // Guarding it unconditionally would eject the user from the only screen
        // that can turn the admin on — the feature could never be enabled.
        val decision = guard(deviceAdminActive = false)
            .evaluate(deviceAdminScreen(), nowMillis = 0)

        assertEquals(GuardDecision.Ignore, decision)
    }

    @Test
    fun `allows the activation dialog when the activity class is not delivered`() {
        // Regression, reproduced on an android-36 emulator before it was fixed:
        // the activation prompt arrives as android.widget.FrameLayout, so a
        // class-keyed exemption never fires. The screen then fell through to the
        // self-mention catch-all and the guard backed the user out of the only
        // screen that can turn the admin on — the feature was unreachable.
        val decision = guard(deviceAdminActive = false)
            .evaluate(deviceAdminScreenWithoutClass(), nowMillis = 0)

        assertEquals(GuardDecision.Ignore, decision)
    }

    @Test
    fun `guards the deactivation dialog when the activity class is not delivered`() {
        // The other half: the same event shape must still be caught once the
        // admin is on, or the exemption would simply unguard the screen.
        val decision = guard(deviceAdminActive = true)
            .evaluate(deviceAdminScreenWithoutClass(), nowMillis = 0)

        assertEquals(GuardDecision.BackOut(GuardedSurface.DEVICE_ADMIN_SETTINGS), decision)
    }

    @Test
    fun `the activation dialog is not recaptured by the self-mention catch-all`() {
        // The trap in the case above: the activation dialog names this app, so
        // exempting it by class is not enough — falling through to the
        // SELF_IN_SETTINGS catch-all would back out of it anyway and the
        // exemption would be silently dead.
        val screen = deviceAdminScreen()
        assertTrue("fixture must name the app for this test to mean anything", label in screen.texts)

        val decision = guard(deviceAdminActive = false).evaluate(screen, nowMillis = 0)

        assertEquals(GuardDecision.Ignore, decision)
    }

    @Test
    fun `admin activation state is read live rather than captured at construction`() {
        // The guard outlives the activation: it is built in onServiceConnected,
        // and the admin is typically enabled later from onboarding.
        var active = false
        val g = SettingsGuard(
            profile = SettingsProfiles.AOSP,
            selfPackage = self,
            selfLabel = label,
            isDeviceAdminActive = { active },
        ).apply { setGuardActive(true) }

        assertEquals(GuardDecision.Ignore, g.evaluate(deviceAdminScreen(), nowMillis = 0))

        active = true
        assertEquals(
            GuardDecision.BackOut(GuardedSurface.DEVICE_ADMIN_SETTINGS),
            g.evaluate(deviceAdminScreen(), nowMillis = 10_000),
        )
    }

    @Test
    fun `ignores the device admin prompt for another app`() {
        // DeviceAdminAdd is shared by every device admin on the phone, exactly
        // like the uninstall dialog. Matching it by ids or class alone would
        // eject the user from managing somebody else's admin app — well outside
        // what this tool may do, and the same overreach the uninstall path is
        // already careful to avoid.
        assertEquals(
            GuardDecision.Ignore,
            guard(deviceAdminActive = true).evaluate(otherAppDeviceAdminScreen(), nowMillis = 0),
        )
        assertEquals(
            GuardDecision.Ignore,
            guard(deviceAdminActive = false).evaluate(otherAppDeviceAdminScreen(), nowMillis = 0),
        )
    }

    @Test
    fun `still guards our own device admin prompt among others`() {
        // The narrowing above must not cost the real case.
        assertEquals(
            GuardDecision.BackOut(GuardedSurface.DEVICE_ADMIN_SETTINGS),
            guard(deviceAdminActive = true).evaluate(deviceAdminScreen(), nowMillis = 0),
        )
    }

    @Test
    fun `the admin exemption does not leak to other guarded screens`() {
        // Only the device-admin screen is conditional. Accessibility and App
        // Info must stay guarded regardless of admin state, or turning the admin
        // off would unguard everything else with it.
        val g = guard(deviceAdminActive = false)

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            g.evaluate(accessibilityScreen(), nowMillis = 0),
        )
        assertEquals(
            GuardDecision.BackOut(GuardedSurface.APP_INFO_SELF),
            g.evaluate(appInfoScreen(), nowMillis = 10_000),
        )
    }

    @Test
    fun `guards a settings screen naming us even with no usable class`() {
        // Verified on device: opening the accessibility list can deliver only
        // content-changed events carrying android.widget.FrameLayout, with the
        // activity class never arriving. Without this rule the screen that
        // disables us sits open and unguarded.
        val noClass = ScreenIdentity(
            packageName = "com.android.settings",
            className = "android.widget.FrameLayout",
            resourceIds = setOf("com.android.settings:id/recycler_view"),
            texts = listOf("Downloaded apps", label),
        )

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.SELF_IN_SETTINGS),
            guard().evaluate(noClass, nowMillis = 0),
        )
    }

    @Test
    fun `does not guard a settings screen that never names us`() {
        val noClass = ScreenIdentity(
            packageName = "com.android.settings",
            className = "android.widget.FrameLayout",
            resourceIds = setOf("com.android.settings:id/recycler_view"),
            texts = listOf("Wi-Fi", "Add network"),
        )

        assertEquals(GuardDecision.Ignore, guard().evaluate(noClass, nowMillis = 0))
    }

    @Test
    fun `never guards the main settings activity`() {
        // Settings$DeviceAdminSettingsActivity is an alias that resolves to this
        // class, so an entry for the alias would match every Settings screen and
        // lock the user out of their own device. Kept as a regression test
        // because the alias name looks like a perfectly good identifier.
        //
        // No self-mention here: this asserts the class is not guarded on its own
        // account, which is separate from the naming rule above.
        val root = ScreenIdentity(
            packageName = "com.android.settings",
            className = "com.android.settings.Settings",
            texts = listOf("Network & internet", "Connected devices"),
        )

        assertEquals(GuardDecision.Ignore, guard().evaluate(root, nowMillis = 0))
    }

    @Test
    fun `ignores ordinary app screens`() {
        assertEquals(GuardDecision.Ignore, guard().evaluate(unrelatedScreen(), nowMillis = 0))
    }

    @Test
    fun `backs out of the page holding our on-off switch`() {
        // The switch that actually disables us lives here, not on the list.
        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard().evaluate(serviceTogglePage(), nowMillis = 0),
        )
    }

    @Test
    fun `ignores unrelated settings screens`() {
        // SubSettings hosts most of Settings, so the class alone must never be
        // enough — otherwise the guard ejects the user from Wi-Fi and Bluetooth
        // too, and they lose access to their own device.
        val wifi = ScreenIdentity(
            packageName = "com.android.settings",
            className = "com.android.settings.SubSettings",
            texts = listOf("Wi-Fi", "Add network"),
        )

        assertEquals(GuardDecision.Ignore, guard().evaluate(wifi, nowMillis = 0))
    }

    @Test
    fun `ignores app info for a different app`() {
        val other = ScreenIdentity(
            packageName = "com.android.settings",
            className = "com.android.settings.spa.SpaActivity",
            texts = listOf("Some Other App", "Force stop"),
        )

        assertEquals(GuardDecision.Ignore, guard().evaluate(other, nowMillis = 0))
    }

    @Test
    fun `matches a dedicated screen whatever language it is in`() {
        // Settings copy is localised; the activity class is not. A screen whose
        // every visible string is Turkish must still be recognised, because the
        // alternative fails silently on any device not set to English.
        val localised = ScreenIdentity(
            packageName = "com.android.settings",
            className = "com.android.settings.Settings\$AccessibilitySettingsActivity",
            texts = listOf("Erişilebilirlik", "Yüklü hizmetler"),
        )

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard().evaluate(localised, nowMillis = 0),
        )
    }

    @Test
    fun `matches a self-scoped screen whatever language it is in`() {
        // Our label is a brand string, so it survives localisation where the
        // surrounding Settings copy does not.
        val localised = ScreenIdentity(
            packageName = "com.android.settings",
            className = "com.android.settings.SubSettings",
            texts = listOf("$label kullan", "Kısayol"),
        )

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard().evaluate(localised, nowMillis = 0),
        )
    }

    @Test
    fun `prefers a resource id when the vendor has a distinctive one`() {
        // AOSP has none, but the mechanism must work for a vendor that does.
        val profile = SettingsProfiles.AOSP.copy(
            resourceIdSurfaces = mapOf(
                "com.example.settings:id/a11y_toggle" to GuardedSurface.ACCESSIBILITY_SETTINGS,
            ),
        )
        val screen = ScreenIdentity(
            packageName = "com.android.settings",
            className = "com.example.settings.SomeUnknownHost",
            resourceIds = setOf("com.example.settings:id/a11y_toggle"),
        )

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard(profile).evaluate(screen, nowMillis = 0),
        )
    }

    @Test
    fun `only watches the settings app and the installers`() {
        // Collecting resource ids means a second pass over the node tree on the
        // UI-event path, so the service asks this before harvesting anything.
        val guard = guard()

        assertTrue(guard.watchesPackage("com.android.settings"))
        assertTrue(guard.watchesPackage("com.google.android.packageinstaller"))
        assertTrue(!guard.watchesPackage("com.other.app"))
        assertTrue(!guard.watchesPackage(self))
    }

    // --- uninstall --------------------------------------------------------

    @Test
    fun `backs out of our own uninstall dialog`() {
        // Reproduced on device: long-pressing the launcher icon and tapping
        // Uninstall opens com.google.android.packageinstaller, which never
        // touches Settings — so every identifier in the settings profile is
        // irrelevant to it. Three taps, no research, and it was completely
        // unguarded.
        val dialog = ScreenIdentity(
            packageName = "com.google.android.packageinstaller",
            className = "com.android.packageinstaller.UninstallerActivity",
            texts = listOf("Do you want to uninstall this app?", label),
        )

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.UNINSTALL_SELF),
            guard().evaluate(dialog, nowMillis = 0),
        )
    }

    @Test
    fun `never interferes with uninstalling a different app`() {
        // The installer is shared by every uninstall on the device. Matching it
        // on anything but our own name would stop the user managing their own
        // phone, which is well outside what this tool may do.
        val dialog = ScreenIdentity(
            packageName = "com.google.android.packageinstaller",
            className = "com.android.packageinstaller.UninstallerActivity",
            texts = listOf("Do you want to uninstall this app?", "Some Other App"),
        )

        assertEquals(GuardDecision.Ignore, guard().evaluate(dialog, nowMillis = 0))
    }

    @Test
    fun `matches our uninstall dialog by package name too`() {
        // Some installers show the package rather than the label.
        val dialog = ScreenIdentity(
            packageName = "com.android.packageinstaller",
            className = "com.android.packageinstaller.UninstallerActivity",
            texts = listOf(self),
        )

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.UNINSTALL_SELF),
            guard().evaluate(dialog, nowMillis = 0),
        )
    }

    @Test
    fun `watches nothing on an unsupported device`() {
        assertTrue(!guard(profile = null).watchesPackage("com.android.settings"))
    }

    // --- unwatched events ---------------------------------------------------

    @Test
    fun `an overlay package event is not a departure while settings stays in front`() {
        // Measured on an android-36 emulator: opening the device-admin list is
        // followed ~200 ms later by a content-changed event from
        // com.android.systemui, for the status bar drawn *over* the settings
        // screen. Treating that as "the user left" cancelled the deferred
        // re-look and left the list unguarded for as long as it stayed open.
        assertEquals(
            UnwatchedEvent.STILL_WATCHED,
            guard().classifyUnwatchedEvent(foregroundPackage = "com.android.settings"),
        )
    }

    @Test
    fun `an event from the app in front is a departure`() {
        assertEquals(
            UnwatchedEvent.LEFT,
            guard().classifyUnwatchedEvent(foregroundPackage = "com.android.chrome"),
        )
    }

    @Test
    fun `an unresolvable foreground is not treated as a departure`() {
        // rootInActiveWindow is null on a settings task restored from the
        // background — the exact case the deferred re-look exists for — so an
        // unknown foreground must not be what cancels it.
        assertEquals(
            UnwatchedEvent.UNKNOWN,
            guard().classifyUnwatchedEvent(foregroundPackage = null),
        )
    }

    @Test
    fun `an unsupported device reports every foreground as a departure`() {
        // No profile means no guarded screen can exist, so nothing is being
        // protected and there is nothing to keep armed.
        assertEquals(
            UnwatchedEvent.LEFT,
            guard(profile = null).classifyUnwatchedEvent(foregroundPackage = "com.android.settings"),
        )
    }

    @Test
    fun `a guarded pane beside the focused app is not a departure`() {
        // Measured on an android-36 emulator in real split screen: Settings sat
        // in one pane reporting focused=false while the clock app in the other
        // owned focus and fired every event. Reading the foreground alone made
        // each of those events look like the user leaving, which cancelled the
        // re-look aimed at the pane that was still sitting there.
        assertEquals(
            UnwatchedEvent.STILL_WATCHED,
            guard().classifyUnwatchedEvent(
                foregroundPackage = "com.google.android.deskclock",
                visiblePackages = listOf("com.google.android.deskclock", "com.android.settings"),
            ),
        )
    }

    @Test
    fun `a split screen with no guarded pane is still a departure`() {
        assertEquals(
            UnwatchedEvent.LEFT,
            guard().classifyUnwatchedEvent(
                foregroundPackage = "com.google.android.deskclock",
                visiblePackages = listOf("com.google.android.deskclock", "com.android.chrome"),
            ),
        )
    }

    @Test
    fun `a visible guarded pane outranks an unresolvable foreground`() {
        // UNKNOWN exists because a foreground that cannot be read must not
        // cancel the re-look. Seeing the guarded window directly is strictly
        // better information than that, so it wins.
        assertEquals(
            UnwatchedEvent.STILL_WATCHED,
            guard().classifyUnwatchedEvent(
                foregroundPackage = null,
                visiblePackages = listOf("com.android.settings"),
            ),
        )
    }

    // --- re-fire suppression ----------------------------------------------

    /** One deliberate visit, spaced past the suppression window each time. */
    private fun attempts(guard: SettingsGuard, screen: ScreenIdentity, n: Int, from: Long = 0) =
        (0 until n).map { guard.evaluate(screen, from + it * (SettingsGuard.BACK_OUT_REFIRE_MILLIS + 50)) }

    /** The same, for the split-screen cases. */
    private fun attempts(guard: SettingsGuard, windows: List<WindowScreen>, n: Int, from: Long = 0) =
        (0 until n).map { guard.evaluate(windows, from + it * (SettingsGuard.BACK_OUT_REFIRE_MILLIS + 50)) }

    @Test
    fun `fires once while a single screen renders`() {
        // Observed on device: the accessibility activity emits three
        // window-state events in ~800 ms as the list renders. Firing BACK on
        // each pops three levels of the navigation stack instead of one, and
        // burns the whole loop budget on a single visit.
        val guard = guard()
        val screen = accessibilityScreen()

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(screen, nowMillis = 0),
        )
        assertEquals(GuardDecision.Ignore, guard.evaluate(screen, nowMillis = 300))
        assertEquals(GuardDecision.Ignore, guard.evaluate(screen, nowMillis = 530))
        assertEquals(GuardDecision.Ignore, guard.evaluate(screen, nowMillis = 800))
    }

    @Test
    fun `fires immediately when the user leaves and comes straight back`() {
        // The attack the suppression window would otherwise open: back out, tap
        // straight back in inside the window, and the guard sits idle. Worse
        // than the gap itself, evaluate only runs when an event fires — once a
        // static screen has rendered, nothing wakes the guard again and the
        // toggle stays reachable indefinitely.
        val guard = guard()

        guard.evaluate(accessibilityScreen(), nowMillis = 0)
        guard.onUnguardedScreen() // the back landed somewhere else

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(accessibilityScreen(), nowMillis = 200),
        )
    }

    @Test
    fun `re-entry does not spend the loop budget`() {
        // Leaving and returning is a fresh visit, not a continuation, so a user
        // tapping in repeatedly must not drive the guard into cover-only.
        val guard = guard()

        repeat(SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS * 3) { i ->
            assertEquals(
                "re-entry $i must still back out",
                GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
                guard.evaluate(accessibilityScreen(), nowMillis = i * 200L),
            )
            guard.onUnguardedScreen()
        }
    }

    @Test
    fun `fires again once the back action has had time to land`() {
        val guard = guard()
        val screen = accessibilityScreen()

        guard.evaluate(screen, nowMillis = 0)

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(screen, nowMillis = SettingsGuard.BACK_OUT_REFIRE_MILLIS + 1),
        )
    }

    // --- back-action bound ------------------------------------------------

    @Test
    fun `degrades to cover only when backing out repeatedly fails`() {
        // If the matcher is wrong on an untested build, or back does not dismiss
        // the window, this is what stops us ejecting the user from Settings
        // forever — including from the App Info page they need to uninstall us.
        val guard = guard()
        val screen = accessibilityScreen()

        attempts(guard, screen, SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS).forEachIndexed { i, d ->
            assertEquals(
                "attempt $i should still back out",
                GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
                d,
            )
        }

        val next = SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS *
            (SettingsGuard.BACK_OUT_REFIRE_MILLIS + 50)
        assertEquals(
            GuardDecision.CoverOnly(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(screen, nowMillis = next),
        )
    }

    @Test
    fun `stays degraded while the screen will not go away`() {
        val guard = guard()
        val screen = accessibilityScreen()
        val n = SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS + 3

        assertEquals(
            GuardDecision.CoverOnly(GuardedSurface.ACCESSIBILITY_SETTINGS),
            attempts(guard, screen, n).last(),
        )
    }

    @Test
    fun `resets the bound once the user actually leaves`() {
        val guard = guard()
        val step = SettingsGuard.BACK_OUT_REFIRE_MILLIS + 50

        attempts(guard, accessibilityScreen(), SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS)
        guard.evaluate(unrelatedScreen(), nowMillis = step * 4)

        // Back worked, so the next visit is a fresh attempt rather than a
        // continuation of a loop that already ended.
        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(accessibilityScreen(), nowMillis = step * 5),
        )
    }

    @Test
    fun `resets the bound after a quiet period`() {
        val guard = guard()

        attempts(guard, accessibilityScreen(), SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS)

        // A later, deliberate return is a new attempt, not the same loop.
        val later = SettingsGuard.BACK_OUT_RESET_MILLIS * 3
        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(accessibilityScreen(), nowMillis = later),
        )
    }

    @Test
    fun `counts each surface separately`() {
        val guard = guard()
        val step = SettingsGuard.BACK_OUT_REFIRE_MILLIS + 50

        attempts(guard, accessibilityScreen(), SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS)

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.APP_INFO_SELF),
            guard.evaluate(appInfoScreen(), nowMillis = step * 4),
        )
    }

    // --- split screen -------------------------------------------------------
    //
    // Two windows are visible at once and only one of them has input focus.
    // `performGlobalAction(GLOBAL_ACTION_BACK)` takes no window argument, so
    // where the focus sits decides which of the two enforcement actions is even
    // available. None of this is reachable through the single-window overload —
    // that one declares its screen focused, which is true of a lone window and
    // false of a split-screen pane.

    private fun focused(screen: ScreenIdentity) = WindowScreen(screen, isFocused = true)
    private fun beside(screen: ScreenIdentity) = WindowScreen(screen, isFocused = false)

    @Test
    fun `backs out of a guarded pane the user is actually in`() {
        val decision = guard().evaluate(
            listOf(beside(unrelatedScreen()), focused(accessibilityScreen())),
            nowMillis = 0,
        )

        assertEquals(GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS), decision)
    }

    @Test
    fun `covers rather than backs out of a guarded pane beside the focused app`() {
        // The reason this is not a BackOut: BACK would be delivered to the app
        // the user is in, not to the pane that matched. That pane would still be
        // there on the next event, so the guard would press BACK through an
        // innocent app until the bound tripped — ~3.6 s of stray presses,
        // ending in a cover over the wrong window anyway.
        val decision = guard().evaluate(
            listOf(focused(unrelatedScreen()), beside(accessibilityScreen())),
            nowMillis = 0,
        )

        assertEquals(GuardDecision.CoverOnly(GuardedSurface.ACCESSIBILITY_SETTINGS), decision)
    }

    @Test
    fun `spends no back-out budget on a pane it only covers`() {
        // The budget exists to stop a BACK loop. Covering fires no back action,
        // so charging it would leave nothing for the moment the user taps into
        // the pane — which is exactly when the toggle becomes reachable.
        val guard = guard()
        val step = SettingsGuard.BACK_OUT_REFIRE_MILLIS + 50

        repeat(SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS * 3) { i ->
            assertEquals(
                "covering pass $i",
                GuardDecision.CoverOnly(GuardedSurface.ACCESSIBILITY_SETTINGS),
                guard.evaluate(
                    listOf(focused(unrelatedScreen()), beside(accessibilityScreen())),
                    nowMillis = i * step,
                ),
            )
        }

        val after = SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS * 3 * step
        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(listOf(focused(accessibilityScreen())), nowMillis = after),
        )
    }

    @Test
    fun `charges one attempt per event however many windows match`() {
        // The hazard in folding a list through `evaluate`: each call used to
        // mutate the counter, so N matching windows would burn N increments and
        // hit the bound within about two events instead of three visits.
        val guard = guard()
        val windows = listOf(
            focused(accessibilityScreen()),
            beside(appInfoScreen()),
            beside(deviceAdminListScreen()),
        )

        attempts(guard, windows, SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS).forEachIndexed { i, d ->
            assertEquals(
                "attempt $i should still back out",
                GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
                d,
            )
        }
    }

    @Test
    fun `still trips the bound when two windows match different surfaces`() {
        // The other direction of the same hazard. Iterating a list through the
        // counter alternates `lastSurface` between the two matches, which resets
        // the run every single event — so the bound never trips and the release
        // valve for a wrong matcher is gone. One decision per event is what
        // makes this hold.
        val guard = guard()
        val windows = listOf(focused(accessibilityScreen()), beside(appInfoScreen()))

        assertEquals(
            GuardDecision.CoverOnly(GuardedSurface.ACCESSIBILITY_SETTINGS),
            attempts(guard, windows, SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS + 1).last(),
        )
    }

    @Test
    fun `prefers a focused match over an unfocused one whatever the surface`() {
        // Focus outranks the surface ordering, because it decides which action
        // is possible. Ranking the covered window first would let a pane the
        // user is not in silence the screen they are looking at.
        val decision = guard().evaluate(
            listOf(beside(deviceAdminListScreen()), focused(appInfoScreen())),
            nowMillis = 0,
        )

        assertEquals(GuardDecision.BackOut(GuardedSurface.APP_INFO_SELF), decision)
    }

    @Test
    fun `names the identified surface rather than the catch-all`() {
        // Between two unfocused matches the ordering only affects which surface
        // is reported and held in `lastSurface` — except at the catch-all, which
        // must lose. The exemptions in `narrow` are keyed on the real surface,
        // so recording a device-admin screen as a generic settings page loses
        // the only distinction that changes behaviour.
        val genericSelfPage = ScreenIdentity(
            packageName = "com.android.settings",
            className = "com.android.settings.SomeOtherHost",
            texts = listOf(label),
        )

        val decision = guard().evaluate(
            listOf(beside(genericSelfPage), beside(deviceAdminListScreen())),
            nowMillis = 0,
        )

        assertEquals(GuardDecision.CoverOnly(GuardedSurface.DEVICE_ADMIN_SETTINGS), decision)
    }

    @Test
    fun `ignores a split screen with nothing guarded in it`() {
        assertEquals(
            GuardDecision.Ignore,
            guard().evaluate(
                listOf(focused(unrelatedScreen()), beside(unrelatedScreen())),
                nowMillis = 0,
            ),
        )
    }

    @Test
    fun `ignores an empty window list`() {
        // The service's fallback when neither the window list nor
        // rootInActiveWindow yields a tree to read.
        assertEquals(GuardDecision.Ignore, guard().evaluate(emptyList(), nowMillis = 0))
    }

    @Test
    fun `resets the bound when the guarded pane closes`() {
        val guard = guard()
        val step = SettingsGuard.BACK_OUT_REFIRE_MILLIS + 50

        attempts(
            guard,
            listOf(focused(accessibilityScreen()), beside(unrelatedScreen())),
            SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS,
        )
        guard.evaluate(listOf(focused(unrelatedScreen())), nowMillis = step * 4)

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(listOf(focused(accessibilityScreen())), nowMillis = step * 5),
        )
    }

    // --- the protection mode ------------------------------------------------

    @Test
    fun `blocks nothing until protection is armed`() {
        // The mode is the whole authority for blocking. Before the user arms it
        // — during onboarding, where the accessibility service is enabled and
        // device admin activated — every one of these screens must be reachable.
        val guard = guard(armed = false)

        assertEquals(GuardDecision.Ignore, guard.evaluate(accessibilityScreen(), nowMillis = 0))
        assertEquals(GuardDecision.Ignore, guard.evaluate(appInfoScreen(), nowMillis = 0))
        assertEquals(GuardDecision.Ignore, guard.evaluate(deviceAdminListScreen(), nowMillis = 0))
        assertEquals(GuardDecision.Ignore, guard.evaluate(deviceAdminScreen(), nowMillis = 0))
        val uninstallDialog = ScreenIdentity(
            packageName = "com.google.android.packageinstaller",
            className = "com.android.packageinstaller.UninstallerActivity",
            texts = listOf("Do you want to uninstall this app?", label),
        )
        assertEquals(GuardDecision.Ignore, guard.evaluate(uninstallDialog, nowMillis = 0))
    }

    @Test
    fun `starts unarmed`() {
        // A guard that blocks before anyone asked it to is the failure this
        // whole mode exists to remove, so the default is off rather than on.
        assertEquals(
            GuardDecision.Ignore,
            SettingsGuard(
                profile = SettingsProfiles.AOSP,
                selfPackage = self,
                selfLabel = label,
                isDeviceAdminActive = { true },
            ).evaluate(accessibilityScreen(), nowMillis = 0),
        )
    }

    @Test
    fun `blocks again as soon as protection is re-armed`() {
        val guard = guard(armed = false)
        guard.evaluate(accessibilityScreen(), nowMillis = 0)

        guard.setGuardActive(true)

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(accessibilityScreen(), nowMillis = 100),
        )
    }

    @Test
    fun `disarming does not leave a spent budget behind`() {
        // The bound counts consecutive back-outs on one screen. A disarm ends
        // that episode, so re-arming must start from a full budget — otherwise a
        // user who disarms mid-loop comes back to a guard that has already given
        // up and only covers.
        val guard = guard()
        attempts(guard, accessibilityScreen(), SettingsGuard.MAX_CONSECUTIVE_BACK_OUTS)

        guard.setGuardActive(false)
        guard.setGuardActive(true)

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(accessibilityScreen(), nowMillis = 999_999),
        )
    }

    // --- unsupported devices ----------------------------------------------

    @Test
    fun `does nothing when the device has no known profile`() {
        // Failing visibly beats failing open: the app tells the user screen
        // protection is unverified rather than pretending to guard.
        val guard = guard(profile = null)

        assertEquals(GuardDecision.Ignore, guard.evaluate(accessibilityScreen(), nowMillis = 0))
        assertTrue(!guard.isDeviceSupported)
    }

    @Test
    fun `reports a known device as supported`() {
        assertTrue(guard().isDeviceSupported)
    }

    @Test
    fun `resolves the AOSP profile for emulator and pixel builds`() {
        assertEquals(SettingsProfiles.AOSP, SettingsProfiles.forManufacturer("Google"))
        assertEquals(SettingsProfiles.AOSP, SettingsProfiles.forManufacturer("unknown"))
    }

    @Test
    fun `has no profile for an untested manufacturer`() {
        assertNull(SettingsProfiles.forManufacturer("Xiaomi"))
    }
}
