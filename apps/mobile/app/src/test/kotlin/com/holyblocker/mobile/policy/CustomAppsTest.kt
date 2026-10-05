package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CustomAppsTest {

    private val cooldown = ProtectionSchedule.COOLDOWN_MILLIS
    private val ready = ProtectionSchedule.READY_WINDOW_MILLIS
    private val app = "org.telegram.messenger"
    private val armed = ProtectionState(ProtectionPhase.ARMED, 0)

    private fun listed(vararg ids: String) = CustomAppState(apps = ids.toSet())

    private fun added(state: CustomAppState, id: String, protected: Set<String> = emptySet()) =
        (CustomApps.add(state, id, protected) as AddOutcome.Added).state

    @Test
    fun `adding is immediate and idempotent`() {
        val once = added(CustomAppState(), app)
        val twice = added(once, app)

        assertEquals(setOf(app), once.apps)
        assertEquals(once, twice)
    }

    @Test
    fun `adding an app cancels its pending removal`() {
        val state = CustomAppState(apps = setOf(app), removals = mapOf(app to 100L))

        assertEquals(emptyMap<String, Long>(), added(state, app).removals)
    }

    @Test
    fun `identities that are not package names are refused`() {
        for (bad in listOf("", "   ", "Telegram", "org..telegram", ".org.telegram", "org.1telegram", "a b.c")) {
            assertEquals(
                bad,
                AddOutcome.Refused(RefusalReason.INVALID_IDENTITY),
                CustomApps.add(CustomAppState(), bad, emptySet()),
            )
        }
    }

    @Test
    fun `surrounding whitespace is trimmed from a typed identity`() {
        assertEquals(setOf(app), added(CustomAppState(), "  $app\n").apps)
    }

    @Test
    fun `a protected identity is refused at add time`() {
        assertEquals(
            AddOutcome.Refused(RefusalReason.PROTECTED),
            CustomApps.add(CustomAppState(), "com.example.launcher", setOf("com.example.launcher")),
        )
    }

    @Test
    fun `a never-rated identity is refused at add time`() {
        assertEquals(
            AddOutcome.Refused(RefusalReason.NEVER),
            CustomApps.add(CustomAppState(), "com.android.dialer", emptySet()),
        )
    }

    // --- removal costs the cooldown, in every phase ---------------------------

    @Test
    fun `removal cannot be confirmed without a request`() {
        assertNull(CustomApps.confirmRemoval(listed(app), app, nowElapsed = 10 * cooldown))
    }

    @Test
    fun `a request is pending until the cooldown has passed`() {
        val state = CustomApps.requestRemoval(listed(app), app, nowElapsed = 1_000)

        assertEquals(RemovalPhase.PENDING, CustomApps.removalPhase(state.removals[app], 1_000 + cooldown - 1))
        assertNull(CustomApps.confirmRemoval(state, app, 1_000 + cooldown - 1))
    }

    @Test
    fun `a matured request can be confirmed and removes the app and the request`() {
        val state = CustomApps.requestRemoval(listed(app, "x.y"), app, nowElapsed = 1_000)

        val after = CustomApps.confirmRemoval(state, app, 1_000 + cooldown)!!

        assertEquals(setOf("x.y"), after.apps)
        assertEquals(emptyMap<String, Long>(), after.removals)
    }

    @Test
    fun `an unconfirmed request expires after the ready window`() {
        val state = CustomApps.requestRemoval(listed(app), app, nowElapsed = 1_000)
        val late = 1_000 + cooldown + ready

        assertEquals(RemovalPhase.NONE, CustomApps.removalPhase(state.removals[app], late))
        assertNull(CustomApps.confirmRemoval(state, app, late))
        assertEquals(emptyMap<String, Long>(), CustomApps.prune(state, late).removals)
        assertEquals(setOf(app), CustomApps.prune(state, late).apps)
    }

    @Test
    fun `a request whose start is in the future is void`() {
        // Reboot resets the monotonic clock; a stored start ahead of now must
        // never read as ready.
        val state = CustomApps.requestRemoval(listed(app), app, nowElapsed = 10 * cooldown)

        assertEquals(RemovalPhase.NONE, CustomApps.removalPhase(state.removals[app], 5))
        assertNull(CustomApps.confirmRemoval(state, app, 5))
        assertEquals(emptyMap<String, Long>(), CustomApps.prune(state, 5).removals)
    }

    @Test
    fun `re-requesting while one is live does not restart it`() {
        val first = CustomApps.requestRemoval(listed(app), app, nowElapsed = 1_000)
        val second = CustomApps.requestRemoval(first, app, nowElapsed = 5_000)

        assertEquals(1_000L, second.removals[app])
    }

    @Test
    fun `a request for an app that is not listed records nothing`() {
        assertEquals(CustomAppState(), CustomApps.requestRemoval(CustomAppState(), app, nowElapsed = 0))
    }

    @Test
    fun `a request is per app`() {
        val state = CustomApps.requestRemoval(listed(app, "x.y"), app, nowElapsed = 0)

        assertNull(CustomApps.confirmRemoval(state, "x.y", cooldown))
    }

    @Test
    fun `cancelling drops only that request`() {
        val state = CustomApps.requestRemoval(
            CustomApps.requestRemoval(listed(app, "x.y"), app, 0),
            "x.y",
            0,
        )

        assertEquals(setOf("x.y"), CustomApps.cancelRemoval(state, app).removals.keys)
    }

    // --- enforcement ----------------------------------------------------------

    @Test
    fun `a listed app is enforced while protection is armed`() {
        assertTrue(CustomApps.shouldEnforce(listed(app), app, emptySet(), armed))
    }

    @Test
    fun `an unlisted app is never enforced`() {
        assertFalse(CustomApps.shouldEnforce(listed(app), "x.y", emptySet(), armed))
    }

    @Test
    fun `enforcement continues through a pending or ready disarm and stops when disarmed or off`() {
        fun enforced(phase: ProtectionPhase) =
            CustomApps.shouldEnforce(listed(app), app, emptySet(), ProtectionState(phase, 0))

        assertTrue(enforced(ProtectionPhase.DISARM_PENDING))
        assertTrue(enforced(ProtectionPhase.DISARM_READY))
        assertFalse(enforced(ProtectionPhase.DISARMED))
        assertFalse(enforced(ProtectionPhase.OFF))
    }

    @Test
    fun `an entry that becomes protected later is ignored`() {
        val launcher = "com.example.launcher"

        assertTrue(CustomApps.shouldEnforce(listed(launcher), launcher, emptySet(), armed))
        assertFalse(CustomApps.shouldEnforce(listed(launcher), launcher, setOf(launcher), armed))
    }

    @Test
    fun `an entry that is rated never is ignored even if it was listed`() {
        assertFalse(CustomApps.shouldEnforce(listed("com.android.dialer"), "com.android.dialer", emptySet(), armed))
    }

    // --- rating ---------------------------------------------------------------

    @Test
    fun `life-sustaining and work apps are rated never`() {
        for (id in listOf(
            "com.android.systemui",
            "com.android.settings",
            "com.google.android.dialer",
            "com.google.android.apps.maps",
            "com.google.android.apps.authenticator2",
            "com.Slack",
        )) {
            assertEquals(id, AppRating.NEVER, AppRater.rate(id))
        }
    }

    @Test
    fun `the bundled list matches regardless of case`() {
        assertEquals(AppRating.NEVER, AppRater.rate("com.slack"))
    }

    @Test
    fun `a bank or payment token in the identity rates never`() {
        assertEquals(AppRating.NEVER, AppRater.rate("ua.privatbank.ap24"))
        assertEquals(AppRating.NEVER, AppRater.rate("com.example.mobilebanking"))
        assertEquals(AppRating.NEVER, AppRater.rate("com.example.wallet"))
        assertEquals(AppRating.NEVER, AppRater.rate("com.example.authenticator"))
    }

    @Test
    fun `consumer messengers are ordinary`() {
        for (id in listOf("org.telegram.messenger", "com.instagram.android", "com.facebook.katana")) {
            assertEquals(id, AppRating.ORDINARY, AppRater.rate(id))
        }
    }

    // --- tokens ---------------------------------------------------------------

    @Test
    fun `the distinctive segment and the display name become tokens`() {
        assertEquals(setOf("instagram"), CustomApps.tokens("com.instagram.android", "Instagram"))
        assertEquals(setOf("telegram"), CustomApps.tokens("org.telegram.messenger", "Telegram"))
        assertEquals(setOf("facebook"), CustomApps.tokens("com.facebook.katana", "Facebook"))
    }

    @Test
    fun `a display name is reduced to lowercase letters and digits`() {
        assertTrue("youtubekids" in CustomApps.tokens("com.google.android.apps.youtube.kids", "YouTube Kids"))
    }

    @Test
    fun `short tokens never match`() {
        assertEquals(emptySet<String>(), CustomApps.tokens("com.vk.android", "VK"))
        assertEquals(emptySet<String>(), CustomApps.tokens("jp.naver.line.android", "Line"))
    }

    @Test
    fun `stoplisted tokens never match`() {
        for (word in listOf("music", "games", "photos", "video", "store", "browser")) {
            assertFalse(word, word in CustomApps.tokens("com.example.$word", word))
        }
    }

    @Test
    fun `tokens are derived without a display name`() {
        assertEquals(setOf("telegram"), CustomApps.tokens("org.telegram.messenger", null))
    }

    @Test
    fun `leading domain segments are skipped at any depth`() {
        assertEquals(setOf("kyivstar"), CustomApps.tokens("ua.com.kyivstar.app", null))
        assertEquals(setOf("twitch"), CustomApps.tokens("tv.twitch.android.app", null))
    }
}
