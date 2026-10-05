package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Test

class CustomAppListCodecTest {

    @Test
    fun `state survives a round trip`() {
        val state = CustomAppState(
            apps = setOf("org.telegram.messenger", "com.instagram.android"),
            removals = mapOf("org.telegram.messenger" to 1_234L),
        )

        assertEquals(state, CustomAppListCodec.decode(CustomAppListCodec.encode(state)))
    }

    @Test
    fun `an empty file is an empty state`() {
        assertEquals(CustomAppState(), CustomAppListCodec.decode(emptyList()))
    }

    @Test
    fun `damaged and unknown lines are skipped and the rest is kept`() {
        val lines = listOf(
            "app\torg.telegram.messenger",
            "app",
            "removal\torg.telegram.messenger\tnot-a-number",
            "future\tx.y",
            "app\tnot a package",
            "removal\torg.telegram.messenger\t42",
        )

        assertEquals(
            CustomAppState(setOf("org.telegram.messenger"), mapOf("org.telegram.messenger" to 42L)),
            CustomAppListCodec.decode(lines),
        )
    }

    @Test
    fun `a removal for an app that is not listed is dropped`() {
        assertEquals(
            CustomAppState(),
            CustomAppListCodec.decode(listOf("removal\tx.y\t5")),
        )
    }
}
