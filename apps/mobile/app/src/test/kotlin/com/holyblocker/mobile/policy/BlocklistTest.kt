package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Test

class BlocklistTest {

    @Test
    fun `plain names come through in order`() {
        assertEquals(
            listOf("ads.example.com", "tracker.example.org"),
            Blocklist.parse(listOf("ads.example.com", "tracker.example.org")),
        )
    }

    @Test
    fun `names are lowercased and lose the root dot`() {
        // DomainFilter matches labels exactly, and the DNS parser hands it
        // lowercased names with no trailing dot (RFC 4343 §3). A rule in any
        // other shape matches nothing and fails silently.
        assertEquals(
            listOf("ads.example.com"),
            Blocklist.parse(listOf("Ads.Example.COM.")),
        )
    }

    @Test
    fun `blank lines and comments are ignored`() {
        assertEquals(
            listOf("ads.example.com"),
            Blocklist.parse(
                listOf(
                    "# the list starts here",
                    "",
                    "   ",
                    "ads.example.com   # inline note",
                ),
            ),
        )
    }

    @Test
    fun `duplicates collapse`() {
        assertEquals(
            listOf("ads.example.com"),
            Blocklist.parse(listOf("ads.example.com", "ADS.example.com.")),
        )
    }

    @Test
    fun `a pasted url is refused rather than half parsed`() {
        // Blocking "https://ads.example.com/x" as a literal label matches
        // nothing. Coercing it into a name would be guessing at intent.
        assertEquals(
            emptyList<String>(),
            Blocklist.parse(listOf("https://ads.example.com/x", "ads.example.com:53")),
        )
    }

    @Test
    fun `hosts file lines are refused`() {
        assertEquals(
            emptyList<String>(),
            Blocklist.parse(listOf("0.0.0.0 ads.example.com")),
        )
    }

    @Test
    fun `malformed names are dropped without taking the rest with them`() {
        assertEquals(
            listOf("ads.example.com"),
            Blocklist.parse(
                listOf(
                    ".leading.dot.example",
                    "double..dot.example",
                    "a".repeat(64) + ".example",
                    "a".repeat(300),
                    "ads.example.com",
                ),
            ),
        )
    }

    @Test
    fun `an empty file yields no rules`() {
        assertEquals(emptyList<String>(), Blocklist.parse(emptyList()))
    }
}
