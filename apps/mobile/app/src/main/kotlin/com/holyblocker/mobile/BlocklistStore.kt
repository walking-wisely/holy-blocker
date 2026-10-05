package com.holyblocker.mobile

import android.content.Context
import android.util.Log
import com.holyblocker.mobile.policy.BlocklistFailure
import com.holyblocker.mobile.policy.BlocklistProvisioning
import com.holyblocker.mobile.policy.SlotFingerprint
import uniffi.net_shield_ffi.ArtifactLoadException
import uniffi.net_shield_ffi.DnsGuard
import uniffi.net_shield_ffi.TrustedKey
import java.io.File
import java.io.IOException

sealed interface BlocklistLoad {
    class Loaded(val guard: DnsGuard) : BlocklistLoad
    class Failed(val failure: BlocklistFailure) : BlocklistLoad
}

/**
 * Installs the signed blocklist bundled in the APK under `filesDir` and loads it.
 *
 * Assets: `blocklist/{current,previous}/{artifact.fst,manifest.bin}` and
 * `blocklist/keys/<key id>.pub` (raw 32-byte Ed25519 public keys). The loader
 * mmaps files, so the assets are copied out; a slot is recopied whenever its
 * bundled manifest differs from the installed one.
 */
class BlocklistStore(context: Context) {

    private val assets = context.applicationContext.assets
    private val appFiles = context.applicationContext.filesDir
    private val root = File(appFiles, INSTALL_DIR)

    fun load(): BlocklistLoad {
        File(appFiles, LEGACY_FILE).delete()
        SLOTS.forEach { slot ->
            try {
                install(slot)
            } catch (e: IOException) {
                Log.w(TAG, "could not install the $slot slot")
            }
        }
        return try {
            val guard = DnsGuard.withArtifact(root.path, trustedKeys())
            Log.i(TAG, "blocklist loaded")
            BlocklistLoad.Loaded(guard)
        } catch (e: ArtifactLoadException.Missing) {
            failed(BlocklistFailure.MISSING)
        } catch (e: Exception) {
            failed(BlocklistFailure.REJECTED)
        }
    }

    private fun failed(failure: BlocklistFailure): BlocklistLoad {
        Log.w(TAG, "blocklist not loaded: $failure")
        return BlocklistLoad.Failed(failure)
    }

    private fun trustedKeys(): List<TrustedKey> =
        (assets.list("$ASSET_DIR/keys") ?: emptyArray()).mapNotNull { name ->
            val id = BlocklistProvisioning.keyId(name) ?: return@mapNotNull null
            TrustedKey(id, assets.open("$ASSET_DIR/keys/$name").use { it.readBytes() })
        }

    private fun install(slot: String) {
        val bundledManifest = readAsset("$ASSET_DIR/$slot/$MANIFEST") ?: return
        val bundledLength = assets.openFd("$ASSET_DIR/$slot/$ARTIFACT").use { it.length }
        val dir = File(root, slot)
        val installedManifest = File(dir, MANIFEST).takeIf { it.exists() }?.readBytes()
        val installed = installedManifest?.let {
            SlotFingerprint(it, File(dir, ARTIFACT).takeIf { f -> f.exists() }?.length() ?: -1)
        }
        if (!BlocklistProvisioning.needsInstall(SlotFingerprint(bundledManifest, bundledLength), installed)) return
        dir.mkdirs()
        assets.open("$ASSET_DIR/$slot/$ARTIFACT").use { input ->
            File(dir, "$ARTIFACT.tmp").outputStream().use { input.copyTo(it) }
        }
        replace(File(dir, "$ARTIFACT.tmp"), File(dir, ARTIFACT))
        File(dir, "$MANIFEST.tmp").writeBytes(bundledManifest)
        replace(File(dir, "$MANIFEST.tmp"), File(dir, MANIFEST))
    }

    private fun replace(from: File, to: File) {
        if (!from.renameTo(to)) throw IOException("could not replace ${to.name}")
    }

    private fun readAsset(path: String): ByteArray? = try {
        assets.open(path).use { it.readBytes() }
    } catch (e: IOException) {
        null
    }

    private companion object {
        const val TAG = "Blocklist"
        const val ASSET_DIR = "blocklist"
        const val LEGACY_FILE = "blocklist.txt"
        const val INSTALL_DIR = "blocklist-artifact"
        const val ARTIFACT = "artifact.fst"
        const val MANIFEST = "manifest.bin"
        val SLOTS = listOf("current", "previous")
    }
}
