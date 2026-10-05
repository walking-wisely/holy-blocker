package com.holyblocker.mobile.policy

enum class BlocklistFailure { MISSING, REJECTED }

object BlocklistProvisioning {

    private const val KEY_SUFFIX = ".pub"

    fun needsInstall(bundled: ByteArray?, installed: ByteArray?): Boolean =
        bundled != null && !bundled.contentEquals(installed)

    fun keyId(fileName: String): String? =
        fileName.removeSuffix(KEY_SUFFIX).takeIf { fileName.endsWith(KEY_SUFFIX) && it.isNotEmpty() }

    fun eventFor(failure: BlocklistFailure): TamperEvent = when (failure) {
        BlocklistFailure.MISSING -> TamperEvent.BLOCKLIST_MISSING
        BlocklistFailure.REJECTED -> TamperEvent.BLOCKLIST_REJECTED
    }
}
