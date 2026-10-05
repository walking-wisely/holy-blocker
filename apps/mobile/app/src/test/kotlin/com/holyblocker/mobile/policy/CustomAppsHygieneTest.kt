package com.holyblocker.mobile.policy

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CustomAppsHygieneTest {

    private val main = File("src/main")

    private val customAppSources: List<File> = main.walkTopDown()
        .filter { it.isFile && it.extension == "kt" && it.name.startsWith("CustomApp") }
        .toList()

    @Test
    fun `the custom-app sources scanned are the expected ones`() {
        assertEquals(
            setOf(
                "CustomApps.kt",
                "CustomAppListCodec.kt",
                "CustomAppStore.kt",
                "CustomAppProtectedPackages.kt",
                "CustomAppEnforcement.kt",
            ),
            customAppSources.map { it.name }.toSet(),
        )
    }

    @Test
    fun `custom-app code never touches the tamper log`() {
        for (file in customAppSources) {
            val text = file.readText()
            for (forbidden in listOf("TamperLog", "TamperEntry", "TamperEvent")) {
                assertFalse("${file.name} mentions $forbidden", text.contains(forbidden))
            }
        }
    }

    @Test
    fun `custom-app code never logs`() {
        for (file in customAppSources) {
            assertFalse("${file.name} calls Log", Regex("""\bLog\.\w""").containsMatchIn(file.readText()))
        }
    }

    @Test
    fun `the manifest keeps backup off so the list stays on the device`() {
        val manifest = File(main, "AndroidManifest.xml").readText()

        assertEquals(1, Regex("""android:allowBackup="false"""").findAll(manifest).count())
        assertFalse(manifest.contains("""allowBackup="true""""))
    }

    @Test
    fun `the screen guard never puts a package name in a log call`() {
        val text = File(main, "kotlin/com/holyblocker/mobile/ScreenGuardService.kt").readText()
        val logCalls = Regex("""Log\.\w\(""").findAll(text).map { call ->
            var depth = 1
            var end = call.range.last + 1
            while (depth > 0) {
                when (text[end]) {
                    '(' -> depth++
                    ')' -> depth--
                }
                end++
            }
            text.substring(call.range.first, end)
        }.toList()

        assertTrue(logCalls.isNotEmpty())
        for (call in logCalls) {
            assertFalse("log call names a package: $call", Regex("""(?i)pkg|packageName""").containsMatchIn(call))
        }
    }

    @Test
    fun `enforcement passes no identity to the tamper log`() {
        val text = File(main, "kotlin/com/holyblocker/mobile/ScreenGuardService.kt").readText()
        val body = text.substringAfter("private fun enforceCustomApps()").substringBefore("private fun guardSettingsScreen")

        assertFalse(body.contains("tamperLog"))
        assertFalse(body.contains("Log."))
    }
}
