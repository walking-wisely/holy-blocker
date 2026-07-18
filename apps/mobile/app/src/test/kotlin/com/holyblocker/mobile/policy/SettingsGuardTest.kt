package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SettingsGuardTest {
    private val self = "com.holyblocker.mobile"
    private val label = "Holy Blocker"

    private fun guard(profile: SettingsProfile? = SettingsProfiles.AOSP) =
        SettingsGuard(profile = profile, selfPackage = self, selfLabel = label)

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
        texts = listOf(label),
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
    fun `backs out of the device admin screen`() {
        val decision = guard().evaluate(deviceAdminScreen(), nowMillis = 0)

        assertEquals(GuardDecision.BackOut(GuardedSurface.DEVICE_ADMIN_SETTINGS), decision)
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

    // --- re-fire suppression ----------------------------------------------

    /** One deliberate visit, spaced past the suppression window each time. */
    private fun attempts(guard: SettingsGuard, screen: ScreenIdentity, n: Int, from: Long = 0) =
        (0 until n).map { guard.evaluate(screen, from + it * (SettingsGuard.BACK_OUT_REFIRE_MILLIS + 50)) }

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

    // --- timed disable ----------------------------------------------------

    @Test
    fun `ignores guarded screens while suspended`() {
        // The exit path: without it, a wrong matcher on an untested device locks
        // the user out of their own settings with no in-app recovery.
        val guard = guard()
        guard.suspendUntil(60_000)

        assertEquals(GuardDecision.Ignore, guard.evaluate(accessibilityScreen(), nowMillis = 30_000))
    }

    @Test
    fun `resumes guarding when the suspension expires`() {
        val guard = guard()
        guard.suspendUntil(60_000)

        assertEquals(
            GuardDecision.BackOut(GuardedSurface.ACCESSIBILITY_SETTINGS),
            guard.evaluate(accessibilityScreen(), nowMillis = 60_001),
        )
    }

    @Test
    fun `reports whether a suspension is in force`() {
        val guard = guard()
        assertTrue(!guard.isSuspended(0))

        guard.suspendUntil(60_000)
        assertTrue(guard.isSuspended(59_999))
        assertTrue(!guard.isSuspended(60_000))
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
