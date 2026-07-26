package com.holyblocker.mobile.policy

/**
 * Reads the list of names the network guard refuses.
 *
 * Pure text handling; the file edge is `BlocklistStore`. It exists because the
 * rule set shipped in `net-shield-ffi` is a placeholder by design — a list of
 * real hostnames is not an artifact this repository carries — so without a
 * runtime source the filter has nothing to enforce.
 *
 * **Normalisation is not cosmetic.** `DomainFilter` on the Rust side matches
 * labels exactly, and the DNS parser hands it lowercased names with no trailing
 * root dot (RFC 4343 §3). A rule written as `Ads.Example.COM.` would therefore
 * match nothing at all and fail silently, which is the worst way for a blocklist
 * to be wrong.
 */
object Blocklist {

    /** Comment marker. Everything from here to end of line is ignored. */
    private const val COMMENT = '#'

    /**
     * Maximum length of a domain name, per RFC 1035 §2.3.4.
     *
     * A longer line cannot be a name the resolver will ever be asked about, so
     * it is a typo or a pasted paragraph rather than a rule.
     */
    private const val MAX_NAME_LENGTH = 253

    /**
     * Turns the file's lines into rules.
     *
     * Tolerant in one direction only: junk is dropped, never guessed at. A rule
     * this cannot read is a rule the user thinks is protecting them, so the
     * safest handling is to skip it rather than to coerce it into something
     * that matches the wrong subtree.
     */
    fun parse(lines: List<String>): List<String> = lines
        .map { it.substringBefore(COMMENT).trim() }
        .filter { it.isNotEmpty() }
        // Trailing root dot: valid in a zone file, never present in the name the
        // DNS parser produces.
        .map { it.removeSuffix(".").lowercase() }
        .filter(::isPlausibleName)
        .distinct()

    /**
     * Whether a line can be a domain name at all.
     *
     * Deliberately shallow — this rejects what cannot possibly match, not what
     * fails to resolve. A name that is well formed and simply does not exist is
     * a perfectly good rule.
     */
    private fun isPlausibleName(name: String): Boolean {
        if (name.length > MAX_NAME_LENGTH) return false
        if (name.startsWith(".") || name.contains("..")) return false
        // A whitespace-separated line is a hosts-file entry or a comment
        // someone forgot to mark, not a name.
        if (name.any { it.isWhitespace() }) return false
        // Scheme, path or port: a URL pasted in place of a name. Blocking
        // "https://ads.example.com/x" as a literal label would match nothing,
        // so it is refused rather than half-parsed.
        if (name.contains('/') || name.contains(':')) return false
        return name.split('.').all { label ->
            // RFC 1035 §2.3.4: labels are 1..63 octets. The character set is
            // deliberately not policed beyond this — internationalised names
            // reach the resolver already punycoded.
            label.isNotEmpty() && label.length <= 63
        }
    }
}
