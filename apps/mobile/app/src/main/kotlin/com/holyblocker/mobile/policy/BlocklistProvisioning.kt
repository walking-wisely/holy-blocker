package com.holyblocker.mobile.policy

enum class BlocklistFailure { MISSING, REJECTED }

class SlotFingerprint(val manifest: ByteArray, val artifactLength: Long) {
    fun matches(other: SlotFingerprint) =
        artifactLength == other.artifactLength && manifest.contentEquals(other.manifest)
}

object BlocklistProvisioning {

    private const val KEY_SUFFIX = ".pub"

    private const val VERSION_BYTES = 8

    /** The manifest is bincode with fixed-width integers and `version: u64` as its first field. */
    fun manifestVersion(manifest: ByteArray): Long? =
        if (manifest.size < VERSION_BYTES) null
        else (VERSION_BYTES - 1 downTo 0).fold(0L) { acc, i -> (acc shl 8) or (manifest[i].toLong() and 0xFF) }

    fun needsInstall(bundled: SlotFingerprint?, installed: SlotFingerprint?): Boolean {
        if (bundled == null) return false
        if (installed == null) return true
        val bundledVersion = manifestVersion(bundled.manifest)
        val installedVersion = manifestVersion(installed.manifest)
        if (bundledVersion != null && installedVersion != null && bundledVersion < installedVersion) return false
        return !bundled.matches(installed)
    }

    fun fellBackToPrevious(loadedVersion: Long?, currentManifest: ByteArray?): Boolean =
        loadedVersion != null && loadedVersion != currentManifest?.let(::manifestVersion)

    fun keyId(fileName: String): String? =
        fileName.removeSuffix(KEY_SUFFIX).takeIf { fileName.endsWith(KEY_SUFFIX) && it.isNotEmpty() }

    fun eventFor(failure: BlocklistFailure): TamperEvent = when (failure) {
        BlocklistFailure.MISSING -> TamperEvent.BLOCKLIST_MISSING
        BlocklistFailure.REJECTED -> TamperEvent.BLOCKLIST_REJECTED
    }
}
