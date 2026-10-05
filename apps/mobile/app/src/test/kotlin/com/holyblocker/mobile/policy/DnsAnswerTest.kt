package com.holyblocker.mobile.policy

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DnsAnswerTest {

    private fun name(vararg labels: String): ByteArray =
        labels.flatMap { listOf(it.length.toByte()) + it.toByteArray().toList() }
            .plus(0.toByte())
            .toByteArray()

    private fun message(
        id: Int,
        response: Boolean,
        qname: ByteArray = name("example", "com"),
        qtype: Int = 1,
        qdCount: Int = 1,
        tail: ByteArray = byteArrayOf(),
    ): ByteArray {
        val header = byteArrayOf(
            (id shr 8).toByte(), id.toByte(),
            if (response) 0x81.toByte() else 0x01, 0,
            (qdCount shr 8).toByte(), qdCount.toByte(),
            0, 0, 0, 0, 0, 0,
        )
        val question = qname + byteArrayOf((qtype shr 8).toByte(), qtype.toByte(), 0, 1)
        return header + question + tail
    }

    private val query = message(0x1234, response = false)

    @Test
    fun `an answer to the same id and question matches`() {
        assertTrue(DnsAnswer.matches(query, message(0x1234, response = true, tail = byteArrayOf(1, 2, 3))))
    }

    @Test
    fun `a different transaction id does not match`() {
        assertFalse(DnsAnswer.matches(query, message(0x1235, response = true)))
    }

    @Test
    fun `a different name does not match`() {
        assertFalse(DnsAnswer.matches(query, message(0x1234, response = true, qname = name("example", "org"))))
    }

    @Test
    fun `a different type does not match`() {
        assertFalse(DnsAnswer.matches(query, message(0x1234, response = true, qtype = 28)))
    }

    @Test
    fun `name case is ignored`() {
        assertTrue(DnsAnswer.matches(query, message(0x1234, response = true, qname = name("EXAMPLE", "Com"))))
    }

    @Test
    fun `a message that is not a response does not match`() {
        assertFalse(DnsAnswer.matches(query, message(0x1234, response = false)))
    }

    @Test
    fun `an answer with no question section does not match`() {
        assertFalse(DnsAnswer.matches(query, message(0x1234, response = true, qdCount = 0)))
    }

    @Test
    fun `a truncated answer does not match`() {
        val answer = message(0x1234, response = true)
        assertFalse(DnsAnswer.matches(query, answer.copyOf(answer.size - 3)))
        assertFalse(DnsAnswer.matches(query, answer.copyOf(5)))
        assertFalse(DnsAnswer.matches(query, byteArrayOf()))
    }

    @Test
    fun `a query that is not a single question matches nothing`() {
        val multi = message(0x1234, response = false, qdCount = 2)
        assertFalse(DnsAnswer.matches(multi, message(0x1234, response = true, qdCount = 2)))
        assertFalse(DnsAnswer.matches(query.copyOf(8), message(0x1234, response = true)))
    }

    @Test
    fun `a compression pointer in the question does not match`() {
        val pointer = byteArrayOf(0xC0.toByte(), 12)
        assertFalse(DnsAnswer.matches(query, message(0x1234, response = true, qname = pointer)))
    }

    @Test
    fun `arbitrary bytes never throw`() {
        val random = java.util.Random(1)
        repeat(5_000) {
            val bytes = ByteArray(random.nextInt(80)).also(random::nextBytes)
            DnsAnswer.matches(bytes, bytes)
            DnsAnswer.matches(query, bytes)
            DnsAnswer.matches(bytes, query)
        }
    }

    @Test
    fun `a differing type or class byte does not match`() {
        val answer = message(0x1234, response = true)
        val classByte = answer.size - 1
        answer[classByte] = (answer[classByte].toInt() xor 0x20).toByte()
        assertFalse(DnsAnswer.matches(query, answer))
    }
}
