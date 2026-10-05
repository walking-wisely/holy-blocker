package com.holyblocker.mobile

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.Telephony
import android.telecom.TelecomManager

fun interface ProtectedPackages {
    fun current(): Set<String>

    companion object {
        fun of(context: Context): ProtectedPackages = AndroidProtectedPackages(context.applicationContext)
    }
}

private class AndroidProtectedPackages(private val context: Context) : ProtectedPackages {

    override fun current(): Set<String> = buildSet {
        add(context.packageName)
        add(SYSTEM_UI)
        add(SETTINGS)
        launcher()?.let(::add)
        dialer()?.let(::add)
        sms()?.let(::add)
    }

    private fun launcher(): String? {
        val home = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_HOME)
        return context.packageManager
            .resolveActivity(home, PackageManager.MATCH_DEFAULT_ONLY)
            ?.activityInfo?.packageName
    }

    private fun dialer(): String? =
        context.getSystemService(TelecomManager::class.java)?.defaultDialerPackage

    private fun sms(): String? = Telephony.Sms.getDefaultSmsPackage(context)

    private companion object {
        const val SYSTEM_UI = "com.android.systemui"
        const val SETTINGS = "com.android.settings"
    }
}
