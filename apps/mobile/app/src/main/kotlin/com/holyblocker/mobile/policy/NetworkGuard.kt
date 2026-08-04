package com.holyblocker.mobile.policy

import java.net.InetAddress

/**
 * The shape of the network guard: what its TUN advertises, when it should be
 * up, and where a permitted query is sent.
 *
 * Every wire-format decision lives in `packages/net-shield` and reaches this app
 * over UniFFI; what is left here is the Android-side configuration, and it is
 * here rather than in `NetworkGuardService` because getting it wrong is the most
 * expensive mistake available in this module. A `VpnService` TUN that claims a
 * route it cannot serve does not degrade — it black-holes every packet matching
 * that route, and a DNS filter that accidentally claims `0.0.0.0/0` takes the
 * whole device offline. So the routing is stated as constants with an invariant
 * around them, and tested.
 */
object NetworkGuard {

    /**
     * The address the TUN gives this device.
     *
     * RFC 1918 §3 private space, and a deliberately unusual corner of it: the
     * TUN's addresses must not collide with a network the user is actually on,
     * or the collision silently steals real traffic.
     */
    const val TUN_ADDRESS = "10.111.222.1"

    /**
     * Prefix length for [TUN_ADDRESS]. /32 — a single host, claiming nothing
     * beyond the address itself.
     */
    const val TUN_PREFIX_LENGTH = 32

    /**
     * The resolver this VPN advertises, and the only destination it routes.
     *
     * `VpnService.Builder.addDnsServer` only takes effect for an address that is
     * reachable through the VPN, so this is both advertised **and** routed —
     * they are one decision, not two.
     */
    const val TUN_DNS_SERVER = "10.111.222.2"

    /**
     * Prefix length of the single route the TUN claims.
     *
     * **The load-bearing constant in this file.** Anything shorter than /32
     * pulls traffic this guard cannot forward into a TUN that answers only DNS.
     * There is no userspace TCP stack behind it — see the Rust side's
     * `dns_shield` for why the Android VPN path cannot re-inject a packet the
     * way the Windows Wintun path can.
     */
    const val ROUTE_PREFIX_LENGTH = 32

    /**
     * MTU of the TUN.
     *
     * 1500 is the Ethernet default (RFC 894), and generous for this link: the
     * only datagrams crossing it are DNS messages, whose EDNS0 buffer sizes are
     * commonly 1232 or smaller (RFC 6891 §6.2.5).
     */
    const val TUN_MTU = 1500

    /** Well-known DNS port — RFC 1035 §4.2, IANA port registry. */
    const val DNS_PORT = 53

    /**
     * Cap on a reply read from an upstream resolver.
     *
     * 4096 is the largest EDNS0 requester payload size in common use (RFC 6891
     * §6.2.5); anything larger is a resolver the client and we both have to
     * treat as truncated.
     */
    const val MAX_DNS_MESSAGE_BYTES = 4096

    /**
     * How long to wait for an upstream answer before giving up on it.
     *
     * A dropped query is retried by the client's own resolver, so the cost of
     * being too impatient is a retry, and the cost of being too patient is a
     * forwarding thread parked on a dead network.
     */
    const val UPSTREAM_TIMEOUT_MILLIS = 5_000

    /**
     * How many resolvers to keep from the system's list.
     *
     * They are tried in order until one answers, and beyond a couple the extras
     * only lengthen the worst case.
     */
    const val MAX_UPSTREAM_RESOLVERS = 3

    /**
     * Whether the VPN should be running.
     *
     * Gated on the same [ProtectionState.guardActive] the settings guard uses,
     * so arming and disarming move the whole product at once rather than
     * leaving the network half of it on after the user has disarmed.
     *
     * Consent is separate and cannot be inferred: `VpnService.prepare` returns
     * an intent the user has to accept, the grant is revocable from Settings at
     * any time, and this must read false the moment it is gone.
     */
    fun shouldRun(protection: ProtectionState, consentGranted: Boolean): Boolean =
        consentGranted && protection.guardActive

    /**
     * Picks the resolvers a permitted query is forwarded to.
     *
     * @param candidates DNS servers of the underlying, non-VPN networks.
     *
     * **Dropping [TUN_DNS_SERVER] is the point of this function.** Once the VPN
     * is up it is the system's advertised resolver, so any code path that reads
     * "the current DNS servers" without excluding ourselves hands back our own
     * address — and forwarding there writes the query back into the TUN it came
     * from, where it is read, forwarded, and written again. That is a packet
     * loop with no natural end, built out of two individually reasonable steps.
     *
     * There is no fallback to a public resolver, deliberately. Sending this
     * user's queries to a third party they did not choose is not a detail to
     * default; an empty list means permitted queries go unanswered until the
     * network comes back, which is what "no network" looks like anyway.
     */
    fun upstreamResolvers(candidates: List<InetAddress>): List<InetAddress> = candidates
        .filter { it.hostAddress != TUN_DNS_SERVER && it.hostAddress != TUN_ADDRESS }
        .distinct()
        .take(MAX_UPSTREAM_RESOLVERS)
}
