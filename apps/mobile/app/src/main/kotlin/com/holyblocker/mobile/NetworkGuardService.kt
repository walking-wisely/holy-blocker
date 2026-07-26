package com.holyblocker.mobile

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.VpnService
import android.os.ParcelFileDescriptor
import android.util.Log
import com.holyblocker.mobile.policy.NetworkGuard
import com.holyblocker.mobile.policy.TamperEvent
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.util.concurrent.Executors
import java.util.concurrent.ThreadPoolExecutor
import uniffi.net_shield_ffi.DnsDecision
import uniffi.net_shield_ffi.DnsGuard

/**
 * The network half of the guard: a `VpnService` that answers DNS locally for
 * names the policy blocks, and forwards everything else untouched.
 *
 * **It filters DNS and only DNS, and the TUN is built so that nothing else can
 * reach it.** The interface claims a single /32 route — the resolver address it
 * advertises — so ordinary traffic never enters it and takes its normal path.
 * That is not a staging decision to be relaxed later without thought: on Android
 * a VPN TUN cannot re-inject a packet the way the Windows Wintun path can, so
 * *permitting* a flow means terminating it in userspace and re-originating it on
 * a [protect]ed socket. For UDP that is a socket per flow, which this does. For
 * TCP it is a userspace TCP stack, which is what SNI and IP filtering will need
 * and is a step of its own. Widening the route before that stack exists would
 * not weaken the filter, it would black-hole the device.
 *
 * What this buys, and what it does not: DNS is the step that decides whether a
 * connection is attempted at all, so blocking there is cheap and covers every
 * app on the device at once. It is also the layer with the most obvious way
 * around it — an app that speaks DNS-over-HTTPS to a hardcoded endpoint never
 * asks the system resolver. Android's own Private DNS setting is one such route
 * and is a Settings screen, which is `SettingsGuard`'s department rather than
 * this one's.
 *
 * Every wire-format decision — parsing the query, judging the name, building the
 * refusal, framing the reply — is in `packages/net-shield` behind
 * [DnsGuard] and unit tested there. This class owns the file descriptor, the
 * sockets and the threads, and nothing else.
 *
 * https://developer.android.com/reference/android/net/VpnService
 */
class NetworkGuardService : VpnService() {

    private lateinit var protection: ProtectionStore
    private lateinit var tamperLog: TamperLogStore

    private var tun: ParcelFileDescriptor? = null
    private var readerThread: Thread? = null
    private var guard: DnsGuard? = null

    /**
     * Forwarding runs off the read loop.
     *
     * A resolver round trip is tens of milliseconds and the read loop is the
     * only thing draining the TUN; doing them inline would stall every other
     * query behind the slowest one. The queue is bounded and its rejection
     * policy is to discard, because a query dropped under load is retried by the
     * client's own resolver, while an unbounded queue under the same load is a
     * memory leak on the device's hot path.
     */
    private var forwarders: ThreadPoolExecutor? = null

    /** Serialises writes back into the TUN across the forwarding threads. */
    private val writeLock = Any()
    private var tunOutput: FileOutputStream? = null

    /**
     * Resolvers of the underlying network, kept fresh by [networkCallback].
     *
     * Volatile because the forwarding threads read it and the connectivity
     * callback writes it.
     */
    @Volatile
    private var upstream: List<InetAddress> = emptyList()

    /**
     * Watches the networks *under* the VPN.
     *
     * `NET_CAPABILITY_NOT_VPN` is what makes this correct rather than a loop:
     * once the TUN is up, the default network is this VPN and its advertised
     * resolver is our own address. Asking the system for "the current DNS
     * servers" without excluding VPNs answers with us.
     * https://developer.android.com/reference/android/net/NetworkCapabilities#NET_CAPABILITY_NOT_VPN
     */
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) {
            // Belt and braces over the capability filter above: NetworkGuard
            // drops our own addresses whatever the system reports.
            upstream = NetworkGuard.upstreamResolvers(linkProperties.dnsServers)
            // The count, not the addresses: a resolver address is not content,
            // but it is one of the few things about a user's network worth not
            // writing down by habit. Zero here is the whole explanation for
            // "nothing resolves", so it has to be visible.
            Log.i(TAG, "upstream resolvers available: ${upstream.size}")
        }

        override fun onLost(network: Network) {
            // Not cleared: another matching network may still be up, and the
            // next onLinkPropertiesChanged replaces the list wholesale. Sending
            // to a resolver that has gone away costs one timeout and a retry;
            // clearing here would drop every query in the gap between two
            // networks.
        }
    }

    override fun onCreate() {
        super.onCreate()
        protection = ProtectionStore(this)
        tamperLog = TamperLogStore.of(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            teardown(TamperEvent.NETWORK_GUARD_STOPPED)
            stopSelf()
            return START_NOT_STICKY
        }

        // prepare() returning non-null means consent has not been granted, or
        // was revoked while we were not looking. Only an Activity can ask for
        // it, so there is nothing to do here but stop.
        if (VpnService.prepare(this) != null) {
            Log.w(TAG, "no VPN consent; not establishing")
            stopSelf()
            return START_NOT_STICKY
        }

        if (!NetworkGuard.shouldRun(protection.state(), consentGranted = true)) {
            teardown(TamperEvent.NETWORK_GUARD_STOPPED)
            stopSelf()
            return START_NOT_STICKY
        }

        if (tun != null) return START_STICKY // already established
        return if (establish()) START_STICKY else START_NOT_STICKY
    }

    /**
     * The user turned the VPN off in Settings, or another VPN app took over.
     *
     * Recorded rather than resisted: plain Device Admin has no
     * `DISALLOW_CONFIG_VPN` — see `plan.md` §7 — so this is one more thing the
     * guard can see and cannot prevent.
     */
    override fun onRevoke() {
        teardown(TamperEvent.NETWORK_GUARD_REVOKED)
        stopSelf()
        super.onRevoke()
    }

    override fun onDestroy() {
        teardown(TamperEvent.NETWORK_GUARD_STOPPED)
        super.onDestroy()
    }

    private fun establish(): Boolean {
        val descriptor = try {
            Builder()
                .setSession(getString(R.string.app_name))
                .addAddress(NetworkGuard.TUN_ADDRESS, NetworkGuard.TUN_PREFIX_LENGTH)
                // Advertised and routed together — one is useless without the
                // other, and the route is the single /32 discussed on the class.
                .addDnsServer(NetworkGuard.TUN_DNS_SERVER)
                .addRoute(NetworkGuard.TUN_DNS_SERVER, NetworkGuard.ROUTE_PREFIX_LENGTH)
                .setMtu(NetworkGuard.TUN_MTU)
                // Blocking reads: the loop below has one job and no other work
                // to interleave, so a poll would only burn battery.
                .setBlocking(true)
                .setConfigureIntent(openApp())
                .establish()
        } catch (e: Exception) {
            // establish() throws IllegalStateException for a malformed builder
            // and IllegalArgumentException for a bad address; neither is
            // recoverable here, and both must not take the process with them.
            Log.e(TAG, "could not establish the VPN", e)
            null
        }

        if (descriptor == null) {
            Log.e(TAG, "VPN not established")
            return false
        }

        tun = descriptor
        tunOutput = FileOutputStream(descriptor.fileDescriptor)

        // Read once per session, not per query: the trie is built on the Rust
        // side at construction, and this is disk I/O. An empty list means no
        // blocklist file, which falls back to the placeholder rules compiled
        // into net-shield-ffi — a supported state, not a broken one.
        val domains = BlocklistStore(this).domains()
        guard = if (domains.isEmpty()) {
            Log.i(TAG, "no blocklist file; using built-in rules")
            DnsGuard.withBuiltinRules()
        } else {
            // The count, never the names: the list is as revealing as the
            // browsing history this product refuses to keep.
            Log.i(TAG, "loaded ${domains.size} blocklist rules")
            DnsGuard.withBlockedDomains(domains)
        }

        forwarders = Executors.newFixedThreadPool(FORWARDER_THREADS) as ThreadPoolExecutor

        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        connectivityManager()?.registerNetworkCallback(request, networkCallback)

        readerThread = Thread(::readLoop, "network-guard").apply { start() }
        tamperLog.record(TamperEvent.NETWORK_GUARD_STARTED)
        return true
    }

    /**
     * Reads the TUN until it is closed.
     *
     * Closing the descriptor is how this loop is stopped: the pending blocking
     * read fails with an [IOException], which is the exit rather than an error.
     */
    private fun readLoop() {
        val descriptor = tun ?: return
        val input = FileInputStream(descriptor.fileDescriptor)
        val buffer = ByteArray(NetworkGuard.TUN_MTU)

        while (!Thread.currentThread().isInterrupted) {
            val length = try {
                input.read(buffer)
            } catch (e: IOException) {
                return // the descriptor was closed; this is the stop path
            }
            if (length <= 0) continue

            val packet = buffer.copyOf(length)
            val engine = guard ?: return

            when (val decision = engine.inspect(packet)) {
                // Nothing routed here is anything but DNS, so this is a
                // malformed or unexpected packet. Dropping it is the only
                // option available — there is no stack to forward it with.
                is DnsDecision.Ignore -> Unit

                // Answered without a byte leaving the device. The name is
                // deliberately not logged, here or in the tamper log: it is
                // content, and this product does not keep a record of what its
                // user looked at.
                is DnsDecision.Blocked -> {
                    // The decision, never the name. A line saying which host was
                    // refused would put in logcat exactly the record this
                    // product exists to not keep.
                    Log.d(TAG, "answered a blocked name locally")
                    writeToTun(decision.reply)
                }

                is DnsDecision.Forward -> {
                    try {
                        forwarders?.execute { forward(packet, decision.query) }
                    } catch (e: Exception) {
                        // Queue full or executor shut down. The client's own
                        // resolver retries; a query lost here is not an error
                        // worth a log line per packet.
                    }
                }
            }
        }
    }

    private fun forward(requestPacket: ByteArray, query: ByteArray) {
        val engine = guard ?: return
        val resolvers = upstream
        if (resolvers.isEmpty()) {
            Log.w(TAG, "no upstream resolver; the query goes unanswered")
            return
        }
        for (resolver in resolvers) {
            val answer = ask(resolver, query) ?: continue
            engine.wrapResponse(requestPacket, answer)?.let(::writeToTun)
            return
        }
        Log.w(TAG, "no resolver answered; the client will retry")
    }

    /** One query to one resolver, over a socket that bypasses this VPN. */
    private fun ask(resolver: InetAddress, query: ByteArray): ByteArray? = try {
        DatagramSocket().use { socket ->
            // Without protect() the socket's own packets are routed by the VPN
            // we just installed, and a query to a resolver we route would come
            // straight back to us.
            // https://developer.android.com/reference/android/net/VpnService#protect(java.net.DatagramSocket)
            if (!protect(socket)) {
                Log.w(TAG, "protect() refused the forwarding socket")
                return null
            }

            socket.soTimeout = NetworkGuard.UPSTREAM_TIMEOUT_MILLIS
            socket.send(DatagramPacket(query, query.size, resolver, NetworkGuard.DNS_PORT))

            val buffer = ByteArray(NetworkGuard.MAX_DNS_MESSAGE_BYTES)
            val response = DatagramPacket(buffer, buffer.size)
            socket.receive(response)
            buffer.copyOf(response.length)
        }
    } catch (e: IOException) {
        // Timeout, unreachable network, or a socket closed under us. The next
        // resolver in the list gets a turn; if none answers the client retries.
        Log.d(TAG, "upstream query failed: ${e.javaClass.simpleName}")
        null
    }

    private fun writeToTun(packet: ByteArray) {
        synchronized(writeLock) {
            try {
                tunOutput?.write(packet)
            } catch (e: IOException) {
                // The descriptor closed between the decision and the write.
                Log.d(TAG, "TUN write failed; the interface is going down")
            }
        }
    }

    /**
     * Takes the VPN down and records why, at most once.
     *
     * Called from three places that can overlap — an explicit stop, a revoke,
     * and `onDestroy` following either — so it has to be idempotent, and the
     * `tun == null` check is what makes it so. Without it a stop would write a
     * pair of entries and the log would read as two teardowns.
     */
    private fun teardown(reason: TamperEvent) {
        val descriptor = tun ?: return
        tun = null

        readerThread?.interrupt()
        readerThread = null

        // Closed before the executor is drained: it is what unblocks the read.
        try {
            descriptor.close()
        } catch (e: IOException) {
            Log.d(TAG, "TUN already closed")
        }
        tunOutput = null

        forwarders?.shutdownNow()
        forwarders = null

        try {
            connectivityManager()?.unregisterNetworkCallback(networkCallback)
        } catch (e: IllegalArgumentException) {
            // Not registered — establish() failed after the descriptor was
            // taken but before the callback went on.
        }

        guard?.close()
        guard = null

        tamperLog.record(reason)
    }

    private fun connectivityManager(): ConnectivityManager? =
        getSystemService(ConnectivityManager::class.java)

    private fun openApp(): PendingIntent {
        val intent = Intent(this, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        return PendingIntent.getActivity(
            this,
            0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }

    companion object {
        private const val TAG = "NetworkGuard"

        private const val ACTION_STOP = "com.holyblocker.mobile.action.STOP_NETWORK_GUARD"

        /**
         * Threads available for upstream queries.
         *
         * Small on purpose: each is parked on a socket for at most
         * [NetworkGuard.UPSTREAM_TIMEOUT_MILLIS], and DNS bursts are short.
         */
        private const val FORWARDER_THREADS = 4

        /**
         * Starts the VPN if the mode and the grant both allow it.
         *
         * **Never allowed to throw**, for the same reason as
         * `GuardStatusService.start`: every caller is doing something that
         * matters more, and the network guard is the newest and least proven
         * half of the product.
         */
        fun start(context: Context) {
            // Consent cannot be requested from here — VpnService.prepare needs
            // an Activity — so this only starts what is already permitted.
            // MainActivity owns asking.
            if (VpnService.prepare(context) != null) return
            if (!NetworkGuard.shouldRun(ProtectionStore(context).state(), consentGranted = true)) {
                return
            }
            try {
                context.startService(Intent(context, NetworkGuardService::class.java))
            } catch (e: Exception) {
                Log.w(TAG, "could not start the network guard", e)
            }
        }

        /** Tears the VPN down. Safe to call when it is not running. */
        fun stop(context: Context) {
            try {
                context.startService(
                    Intent(context, NetworkGuardService::class.java).setAction(ACTION_STOP),
                )
            } catch (e: Exception) {
                Log.w(TAG, "could not stop the network guard", e)
            }
        }

        /**
         * Whether the user has granted this app the VPN capability.
         *
         * `prepare` returning null means "already prepared"; a non-null intent
         * is the consent dialog and must be launched from an Activity.
         */
        fun hasConsent(context: Context): Boolean = VpnService.prepare(context) == null
    }
}
