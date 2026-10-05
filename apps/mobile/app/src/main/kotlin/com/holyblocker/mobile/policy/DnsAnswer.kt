package com.holyblocker.mobile.policy

object DnsAnswer {

    private const val HEADER_BYTES = 12
    private const val FLAGS_BYTE = 2
    private const val QR_FLAG = 0x80
    private const val QDCOUNT_HIGH = 4
    private const val QDCOUNT_LOW = 5
    private const val TYPE_AND_CLASS_BYTES = 4
    private const val ASCII_CASE_BIT = 0x20

    /** RFC 1035 §4.1.1 header and §4.1.2 question; [answer] must echo [query]'s id and question. */
    fun matches(query: ByteArray, answer: ByteArray): Boolean {
        val questionEnd = singleQuestionEnd(query) ?: return false
        if (answer.size < questionEnd) return false
        if (answer[FLAGS_BYTE].toInt() and QR_FLAG == 0) return false
        if (query[0] != answer[0] || query[1] != answer[1]) return false
        if (!hasOneQuestion(answer)) return false
        for (i in HEADER_BYTES until questionEnd) {
            if (!sameByte(query[i], answer[i])) return false
        }
        return true
    }

    private fun hasOneQuestion(message: ByteArray): Boolean =
        message.size >= HEADER_BYTES && message[QDCOUNT_HIGH].toInt() == 0 && message[QDCOUNT_LOW].toInt() == 1

    private fun singleQuestionEnd(query: ByteArray): Int? {
        if (!hasOneQuestion(query)) return null
        var i = HEADER_BYTES
        while (true) {
            val length = query.getOrNull(i)?.toInt()?.and(0xFF) ?: return null
            if (length and 0xC0 != 0) return null
            i += 1 + length
            if (length == 0) break
        }
        val end = i + TYPE_AND_CLASS_BYTES
        return if (end <= query.size) end else null
    }

    private fun sameByte(a: Byte, b: Byte): Boolean =
        a == b || (isLetter(a) && (a.toInt() xor b.toInt()) == ASCII_CASE_BIT && isLetter(b))

    private fun isLetter(b: Byte): Boolean = (b.toInt() or ASCII_CASE_BIT) in 'a'.code..'z'.code
}
