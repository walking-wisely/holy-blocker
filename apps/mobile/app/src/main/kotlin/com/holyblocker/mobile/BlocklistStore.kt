package com.holyblocker.mobile

import android.content.Context
import android.util.Log
import com.holyblocker.mobile.policy.BlocklistFailure
import com.holyblocker.mobile.policy.BlocklistProvisioning
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
    private val root = File(context.applicationContext.filesDir, INSTALL_DIR)

    fun load(): BlocklistLoad = try {
        SLOTS.forEach(::install)
        BlocklistLoad.Loaded(DnsGuard.withArtifact(root.path, trustedKeys()))
    } catch (e: ArtifactLoadException.Missing) {
        failed(BlocklistFailure.MISSING)
    } catch (e: ArtifactLoadException) {
        failed(BlocklistFailure.REJECTED)
    } catch (e: IOException) {
        failed(BlocklistFailure.MISSING)
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
        val bundled = readAsset("$ASSET_DIR/$slot/$MANIFEST")
        val installed = File(root, "$slot/$MANIFEST").takeIf { it.exists() }?.readBytes()
        if (!BlocklistProvisioning.needsInstall(bundled, installed)) return
        val dir = File(root, slot).apply { mkdirs() }
        assets.open("$ASSET_DIR/$slot/$ARTIFACT").use { input ->
            File(dir, "$ARTIFACT.tmp").outputStream().use { input.copyTo(it) }
        }
        File(dir, "$ARTIFACT.tmp").renameTo(File(dir, ARTIFACT))
        File(dir, "$MANIFEST.tmp").writeBytes(bundled!!)
        File(dir, "$MANIFEST.tmp").renameTo(File(dir, MANIFEST))
    }

    private fun readAsset(path: String): ByteArray? = try {
        assets.open(path).use { it.readBytes() }
    } catch (e: IOException) {
        null
    }

    private companion object {
        const val TAG = "Blocklist"
        const val ASSET_DIR = "blocklist"
        const val INSTALL_DIR = "blocklist-artifact"
        const val ARTIFACT = "artifact.fst"
        const val MANIFEST = "manifest.bin"
        val SLOTS = listOf("current", "previous")
    }
}
