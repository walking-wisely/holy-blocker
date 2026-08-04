package com.holyblocker.mobile.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TextAssemblerTest {
    @Test
    fun `joins fragments with single spaces`() {
        assertEquals("alpha beta gamma", TextAssembler.assemble(listOf("alpha", "beta", "gamma")))
    }

    @Test
    fun `drops blank and whitespace-only fragments`() {
        assertEquals("alpha beta", TextAssembler.assemble(listOf("alpha", "", "   ", "beta")))
    }

    @Test
    fun `collapses internal whitespace runs`() {
        assertEquals("alpha beta", TextAssembler.assemble(listOf("alpha \n\t  beta")))
    }

    @Test
    fun `returns null for empty input`() {
        assertNull(TextAssembler.assemble(emptyList()))
    }

    @Test
    fun `returns null when every fragment is blank`() {
        assertNull(TextAssembler.assemble(listOf("", "  ", "\n")))
    }

    @Test
    fun `keeps text at exactly the cap`() {
        val text = "a".repeat(10)
        assertEquals(text, TextAssembler.assemble(listOf(text), maxChars = 10))
    }

    @Test
    fun `truncates on a word boundary rather than mid-token`() {
        // Cutting at 8 would land inside "gamma"; the assembler must back off to
        // the previous space so the lexicon does not see a half-word.
        val result = TextAssembler.assemble(listOf("al be gamma"), maxChars = 8)
        assertEquals("al be", result)
    }

    @Test
    fun `truncates hard when a single token exceeds the cap`() {
        val result = TextAssembler.assemble(listOf("a".repeat(50)), maxChars = 10)
        assertEquals("a".repeat(10), result)
    }

    @Test
    fun `result never exceeds the cap`() {
        val fragments = List(500) { "token$it" }
        val result = TextAssembler.assemble(fragments, maxChars = 100)!!
        assertTrue(result.length <= 100)
    }

    @Test
    fun `non-positive cap yields null`() {
        assertNull(TextAssembler.assemble(listOf("alpha"), maxChars = 0))
        assertNull(TextAssembler.assemble(listOf("alpha"), maxChars = -1))
    }
}
