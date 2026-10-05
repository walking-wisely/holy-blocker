package com.holyblocker.mobile

import android.content.Context
import android.os.SystemClock
import com.holyblocker.mobile.policy.AddOutcome
import com.holyblocker.mobile.policy.CustomAppListCodec
import com.holyblocker.mobile.policy.CustomAppState
import com.holyblocker.mobile.policy.CustomApps
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.StandardCopyOption

class CustomAppStore internal constructor(
    dir: File,
    private val protectedPackages: ProtectedPackages,
) {

    constructor(
        context: Context,
        protectedPackages: ProtectedPackages = ProtectedPackages.of(context),
    ) : this(context.applicationContext.filesDir, protectedPackages)

    private val file = File(dir, FILE_NAME)
    private val temp = File(dir, "$FILE_NAME.tmp")

    fun state(nowElapsed: Long = SystemClock.elapsedRealtime()): CustomAppState = synchronized(LOCK) {
        val loaded = read()
        val pruned = CustomApps.prune(loaded, nowElapsed)
        if (pruned != loaded) write(pruned)
        pruned
    }

    fun add(identity: String, nowElapsed: Long = SystemClock.elapsedRealtime()): AddOutcome =
        synchronized(LOCK) {
            val outcome = CustomApps.add(state(nowElapsed), identity, protectedPackages.current())
            if (outcome is AddOutcome.Added) write(outcome.state)
            outcome
        }

    fun requestRemoval(identity: String, nowElapsed: Long = SystemClock.elapsedRealtime()) {
        synchronized(LOCK) {
            write(CustomApps.requestRemoval(state(nowElapsed), identity, nowElapsed))
        }
    }

    fun cancelRemoval(identity: String) {
        synchronized(LOCK) { write(CustomApps.cancelRemoval(read(), identity)) }
    }

    fun confirmRemoval(identity: String, nowElapsed: Long = SystemClock.elapsedRealtime()): Boolean =
        synchronized(LOCK) {
            val after = CustomApps.confirmRemoval(state(nowElapsed), identity, nowElapsed)
                ?: return@synchronized false
            write(after)
            true
        }

    /** A missing file is an empty list; a file that exists but cannot be read throws. */
    private fun read(): CustomAppState =
        if (file.exists()) CustomAppListCodec.decode(file.readLines()) else CustomAppState()

    private fun write(state: CustomAppState) {
        val bytes = CustomAppListCodec.encode(state).joinToString("\n", postfix = "\n").toByteArray()
        try {
            FileOutputStream(temp).use {
                it.write(bytes)
                it.fd.sync()
            }
            Files.move(temp.toPath(), file.toPath(), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } catch (e: IOException) {
            temp.delete()
            throw e
        }
    }

    private companion object {
        const val FILE_NAME = "custom_apps.txt"
        val LOCK = Any()
    }
}
