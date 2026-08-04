package com.holyblocker.mobile

import android.content.Context
import android.util.Log
import com.holyblocker.mobile.policy.Blocklist
import java.io.File
import java.io.IOException

/**
 * The file the network guard's rules are read from.
 *
 * `filesDir/blocklist.txt`, one name per line, `#` for comments. Private
 * storage, so nothing else on the device can read which names the user chose —
 * that list is as revealing as the browsing history this product refuses to
 * keep.
 *
 * Absent by default and that is a supported state, not a broken one: with no
 * file the guard falls back to the placeholder rules compiled into
 * `net-shield-ffi` and blocks nothing real. A list of actual hostnames is not
 * something this repository ships.
 *
 * Read once when the VPN establishes rather than per query — the trie is built
 * on the Rust side at construction, and the read is disk I/O.
 */
class BlocklistStore(context: Context) {

    private val file = File(context.applicationContext.filesDir, FILE_NAME)

    /**
     * The rules, or an empty list when there is no readable file.
     *
     * Never throws. A missing or unreadable list must leave the rest of the
     * guard running — the accessibility path does not depend on this, and
     * taking the process down over it would trade a weaker filter for no filter
     * at all.
     */
    fun domains(): List<String> = try {
        if (file.exists()) Blocklist.parse(file.readLines()) else emptyList()
    } catch (e: IOException) {
        Log.w(TAG, "could not read the blocklist; falling back to built-in rules")
        emptyList()
    }

    private companion object {
        const val TAG = "Blocklist"
        const val FILE_NAME = "blocklist.txt"
    }
}
