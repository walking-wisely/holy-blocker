package com.holyblocker.mobile

import com.holyblocker.mobile.policy.AddOutcome
import com.holyblocker.mobile.policy.ProtectionSchedule
import java.io.File
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class CustomAppStoreTest {

    @get:Rule
    val folder = TemporaryFolder()

    private val app = "org.telegram.messenger"
    private val cooldown = ProtectionSchedule.COOLDOWN_MILLIS

    private fun store(protected: Set<String> = emptySet()) =
        CustomAppStore(folder.root, ProtectedPackages { protected })

    @Test
    fun `an added app survives a new store instance`() {
        store().add(app, nowElapsed = 0)

        assertEquals(setOf(app), store().state(nowElapsed = 0).apps)
    }

    @Test
    fun `removal needs a matured request`() {
        val s = store()
        s.add(app, nowElapsed = 0)

        assertFalse(s.confirmRemoval(app, nowElapsed = cooldown * 10))
        s.requestRemoval(app, nowElapsed = 1_000)
        assertFalse(s.confirmRemoval(app, nowElapsed = 1_000 + cooldown - 1))
        assertTrue(s.confirmRemoval(app, nowElapsed = 1_000 + cooldown))
        assertEquals(emptySet<String>(), store().state(nowElapsed = 1_000 + cooldown).apps)
    }

    @Test
    fun `an expired request is deleted from disk when the state is read`() {
        val s = store()
        s.add(app, nowElapsed = 0)
        s.requestRemoval(app, nowElapsed = 0)

        s.state(nowElapsed = cooldown * 10)

        assertFalse(File(folder.root, "custom_apps.txt").readText().contains("removal"))
    }

    @Test
    fun `a protected identity is refused by the store`() {
        assertTrue(store(setOf("x.y")).add("x.y", nowElapsed = 0) is AddOutcome.Refused)
    }

    @Test
    fun `an unreadable file is not treated as an empty list`() {
        File(folder.root, "custom_apps.txt").mkdir()

        var failed = false
        try {
            store().add(app, nowElapsed = 0)
        } catch (e: IOException) {
            failed = true
        }

        assertTrue(failed)
        assertTrue(File(folder.root, "custom_apps.txt").isDirectory)
    }

    @Test
    fun `no temporary file is left behind`() {
        store().add(app, nowElapsed = 0)

        assertEquals(listOf("custom_apps.txt"), folder.root.list()!!.toList())
    }
}
