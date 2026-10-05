package com.holyblocker.mobile

import android.content.Context
import android.os.SystemClock
import com.holyblocker.mobile.policy.AddOutcome
import com.holyblocker.mobile.policy.CustomAppListCodec
import com.holyblocker.mobile.policy.CustomAppState
import com.holyblocker.mobile.policy.CustomApps
import java.io.File
import java.io.IOException

class CustomAppStore(
    context: Context,
    private val protectedPackages: ProtectedPackages = ProtectedPackages.of(context),
) {

    private val file = File(context.applicationContext.filesDir, FILE_NAME)

    @Synchronized
    fun state(nowElapsed: Long = SystemClock.elapsedRealtime()): CustomAppState {
        val loaded = read()
        val pruned = CustomApps.prune(loaded, nowElapsed)
        if (pruned != loaded) write(pruned)
        return pruned
    }

    @Synchronized
    fun add(identity: String, nowElapsed: Long = SystemClock.elapsedRealtime()): AddOutcome {
        val outcome = CustomApps.add(state(nowElapsed), identity, protectedPackages.current())
        if (outcome is AddOutcome.Added) write(outcome.state)
        return outcome
    }

    @Synchronized
    fun requestRemoval(identity: String, nowElapsed: Long = SystemClock.elapsedRealtime()) {
        write(CustomApps.requestRemoval(state(nowElapsed), identity, nowElapsed))
    }

    @Synchronized
    fun cancelRemoval(identity: String) {
        write(CustomApps.cancelRemoval(read(), identity))
    }

    @Synchronized
    fun confirmRemoval(identity: String, nowElapsed: Long = SystemClock.elapsedRealtime()): Boolean {
        val after = CustomApps.confirmRemoval(state(nowElapsed), identity, nowElapsed) ?: return false
        write(after)
        return true
    }

    private fun read(): CustomAppState = try {
        if (file.exists()) CustomAppListCodec.decode(file.readLines()) else CustomAppState()
    } catch (e: IOException) {
        CustomAppState()
    }

    private fun write(state: CustomAppState) {
        val temp = File(file.parentFile, "$FILE_NAME.tmp")
        temp.writeText(CustomAppListCodec.encode(state).joinToString("\n", postfix = "\n"))
        if (!temp.renameTo(file)) throw IOException("could not replace the custom-app list")
    }

    private companion object {
        const val FILE_NAME = "custom_apps.txt"
    }
}
