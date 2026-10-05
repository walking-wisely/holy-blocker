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
    fun `the custom-app sources were found`() {
        assertTrue(customAppSources.size >= 3)
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
}
