package com.holyblocker.mobile.policy

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ProtectionScheduleVectorsTest {

    private fun vectorsFile(): File {
        var dir: File? = File(System.getProperty("user.dir")).absoluteFile
        while (dir != null) {
            val candidate = File(dir, "test-vectors/protection-schedule.tsv")
            if (candidate.isFile) return candidate
            dir = dir.parentFile
        }
        error("test-vectors/protection-schedule.tsv not found above ${System.getProperty("user.dir")}")
    }

    private fun optional(field: String): Long? = if (field == "-") null else field.toLong()

    @Test
    fun `every shared vector evaluates to its recorded state`() {
        val rows = vectorsFile().readLines().filter { it.isNotBlank() && !it.startsWith("#") }
        assertTrue(rows.size >= 15)
        for (row in rows) {
            val f = row.split("\t")
            assertEquals("malformed vector: $row", 7, f.size)
            val state = ProtectionSchedule.evaluate(
                armed = f[1] == "1",
                disarmRequestedAtElapsed = optional(f[2]),
                disarmedAtElapsed = optional(f[3]),
                nowElapsed = f[4].toLong(),
            )
            assertEquals(f[0], f[5], state.phase.name.lowercase())
            assertEquals(f[0], f[6].toLong(), state.remainingMillis)
        }
    }
}
