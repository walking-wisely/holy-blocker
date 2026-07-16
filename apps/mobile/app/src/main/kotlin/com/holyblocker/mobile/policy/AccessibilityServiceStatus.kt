package com.holyblocker.mobile.policy

/**
 * Parses `Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES` to tell whether our
 * service is currently enabled.
 *
 * The setting is a colon-separated list of `packageName/serviceClass` entries.
 * The class part may be fully qualified (`com.pkg/com.pkg.Svc`) or relative
 * (`com.pkg/.Svc`) depending on how the entry was written, and OEM builds vary
 * in the whitespace and trailing separators they leave behind — hence a parser
 * with tests rather than a `contains` check.
 *
 * Reference: https://developer.android.com/reference/android/provider/Settings.Secure#ENABLED_ACCESSIBILITY_SERVICES
 */
object AccessibilityServiceStatus {

    fun isEnabled(
        enabledServicesSetting: String?,
        packageName: String,
        serviceClassName: String,
    ): Boolean {
        if (enabledServicesSetting.isNullOrBlank()) return false

        val relative = serviceClassName.removePrefix(packageName)
        val wanted = setOf(
            "$packageName/$serviceClassName",
            "$packageName/$relative",
        )

        return enabledServicesSetting
            .split(':')
            .asSequence()
            .map { it.trim() }
            .filter { it.isNotEmpty() }
            .map { normalise(it, packageName) }
            .any { it in wanted }
    }

    /**
     * Rewrites an entry to its fully-qualified form so `com.pkg/.Svc` and
     * `com.pkg/com.pkg.Svc` compare equal.
     */
    private fun normalise(entry: String, packageName: String): String {
        val slash = entry.indexOf('/')
        if (slash < 0) return entry

        val pkg = entry.substring(0, slash)
        val cls = entry.substring(slash + 1)
        val qualified = if (cls.startsWith(".")) "$pkg$cls" else cls
        return if (pkg == packageName) "$packageName/$qualified" else entry
    }
}
