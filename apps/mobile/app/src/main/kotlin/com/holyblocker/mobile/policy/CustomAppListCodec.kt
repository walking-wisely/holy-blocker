package com.holyblocker.mobile.policy

object CustomAppListCodec {

    private const val SEPARATOR = '\t'
    private const val APP = "app"
    private const val REMOVAL = "removal"

    private val identityPattern = Regex("""[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+""")

    fun encode(state: CustomAppState): List<String> =
        state.apps.sorted().map { "$APP$SEPARATOR$it" } +
            state.removals.toSortedMap().map { (id, at) -> "$REMOVAL$SEPARATOR$id$SEPARATOR$at" }

    fun decode(lines: List<String>): CustomAppState {
        val apps = linkedSetOf<String>()
        val removals = linkedMapOf<String, Long>()
        for (line in lines) {
            val fields = line.split(SEPARATOR)
            val id = fields.getOrNull(1)?.takeIf { identityPattern.matches(it) } ?: continue
            when (fields[0]) {
                APP -> apps += id
                REMOVAL -> fields.getOrNull(2)?.toLongOrNull()?.takeIf { it >= 0 }?.let { removals[id] = it }
            }
        }
        return CustomAppState(apps, removals.filterKeys { it in apps })
    }
}
