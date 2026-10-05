package com.holyblocker.mobile.policy

enum class BlocklistFailure { MISSING, REJECTED }

class SlotFingerprint(val manifest: ByteArray, val artifactLength: Long) {
    fun matches(other: SlotFingerprint) =
        artifactLength == other.artifactLength && manifest.contentEquals(other.manifest)
}

object BlocklistProvisioning {

    private const val KEY_SUFFIX = ".pub"

    fun needsInstall(bundled: SlotFingerprint?, installed: SlotFingerprint?): Boolean =
        bundled != null && (installed == null || !bundled.matches(installed))

    fun keyId(fileName: String): String? =
        fileName.removeSuffix(KEY_SUFFIX).takeIf { fileName.endsWith(KEY_SUFFIX) && it.isNotEmpty() }

    fun eventFor(failure: BlocklistFailure): TamperEvent = when (failure) {
        BlocklistFailure.MISSING -> TamperEvent.BLOCKLIST_MISSING
        BlocklistFailure.REJECTED -> TamperEvent.BLOCKLIST_REJECTED
    }
}
