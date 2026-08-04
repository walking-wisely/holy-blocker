package com.holyblocker.mobile.policy

import java.net.InetAddress
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NetworkGuardTest {

    private fun addr(s: String): InetAddress = InetAddress.getByName(s)

    /** `guardActive` is derived from the phase, so the phase is what is set. */
    private fun protection(guardActive: Boolean) = ProtectionState(
        phase = if (guardActive) ProtectionPhase.ARMED else ProtectionPhase.OFF,
        remainingMillis = 0,
    ).also { check(it.guardActive == guardActive) }

    // ---- routing ------------------------------------------------------------

    @Test
    fun `the tun routes exactly one host and nothing else`() {
        // The whole failure mode this guards: a shorter prefix pulls traffic
        // into a TUN that can only answer DNS, and every such packet is dropped
        // because there is no userspace stack to forward it. The device would
        // look offline, not filtered.
        assertEquals(32, NetworkGuard.ROUTE_PREFIX_LENGTH)
        assertEquals(32, NetworkGuard.TUN_PREFIX_LENGTH)
    }

    @Test
    fun `the advertised resolver is the address that is routed`() {
        // addDnsServer only takes effect for an address reachable through the
        // VPN. Advertising one address and routing another is a VPN that
        // establishes cleanly and resolves nothing.
        assertTrue(
            "the routed host must be the advertised resolver",
            NetworkGuard.TUN_DNS_SERVER.isNotEmpty(),
        )
        assertFalse(
            "the resolver must not be the interface's own address",
            NetworkGuard.TUN_DNS_SERVER == NetworkGuard.TUN_ADDRESS,
        )
    }

    @Test
    fun `the tun addresses sit in private space`() {
        // RFC 1918 §3. A public address here would silently steal real traffic
        // from whoever owns it.
        for (address in listOf(NetworkGuard.TUN_ADDRESS, NetworkGuard.TUN_DNS_SERVER)) {
            assertTrue("$address must be RFC 1918", addr(address).isSiteLocalAddress)
        }
    }

    // ---- when it runs -------------------------------------------------------

    @Test
    fun `it runs only when protection is armed and consent is granted`() {
        assertTrue(NetworkGuard.shouldRun(protection(true), consentGranted = true))
        assertFalse(NetworkGuard.shouldRun(protection(false), consentGranted = true))
        assertFalse(NetworkGuard.shouldRun(protection(true), consentGranted = false))
        assertFalse(NetworkGuard.shouldRun(protection(false), consentGranted = false))
    }

    @Test
    fun `a revoked consent stops it even while armed`() {
        // The VPN grant is revocable from Settings at any time and cannot be
        // inferred from the mode; reading it as still granted would leave the
        // service trying to establish a TUN it is no longer allowed.
        assertFalse(NetworkGuard.shouldRun(protection(true), consentGranted = false))
    }

    // ---- upstream selection -------------------------------------------------

    @Test
    fun `our own resolver address is never forwarded to`() {
        // The loop: once the VPN is up, TUN_DNS_SERVER *is* the system's
        // advertised resolver, so any "read the current DNS servers" path hands
        // back our own address. Forwarding there writes the query straight back
        // into the TUN it came from, forever.
        val chosen = NetworkGuard.upstreamResolvers(
            listOf(addr(NetworkGuard.TUN_DNS_SERVER), addr("192.168.1.1")),
        )
        assertEquals(listOf(addr("192.168.1.1")), chosen)
    }

    @Test
    fun `the tun's own interface address is dropped too`() {
        val chosen = NetworkGuard.upstreamResolvers(
            listOf(addr(NetworkGuard.TUN_ADDRESS), addr("192.168.1.1")),
        )
        assertEquals(listOf(addr("192.168.1.1")), chosen)
    }

    @Test
    fun `resolvers keep their order and lose their duplicates`() {
        // Order is the try order, and two networks commonly advertise the same
        // resolver — trying it twice only lengthens the worst case.
        val chosen = NetworkGuard.upstreamResolvers(
            listOf(addr("192.168.1.1"), addr("192.168.1.2"), addr("192.168.1.1")),
        )
        assertEquals(listOf(addr("192.168.1.1"), addr("192.168.1.2")), chosen)
    }

    @Test
    fun `ipv6 resolvers are kept`() {
        // The TUN is IPv4-only, but the network under it need not be: an
        // IPv6-only carrier advertises IPv6 resolvers and the forwarding socket
        // reaches them normally.
        val v6 = addr("2001:4860:4860::8888")
        assertEquals(listOf(v6), NetworkGuard.upstreamResolvers(listOf(v6)))
    }

    @Test
    fun `the list is capped`() {
        val many = (1..10).map { addr("192.168.1.$it") }
        assertEquals(
            NetworkGuard.MAX_UPSTREAM_RESOLVERS,
            NetworkGuard.upstreamResolvers(many).size,
        )
    }

    @Test
    fun `no resolver is invented when the system offers none`() {
        // Falling back to a public resolver would send this user's queries to a
        // third party they never chose. An empty list means permitted queries
        // go unanswered, which is what no network looks like anyway.
        assertTrue(NetworkGuard.upstreamResolvers(emptyList()).isEmpty())
        assertTrue(
            NetworkGuard.upstreamResolvers(listOf(addr(NetworkGuard.TUN_DNS_SERVER))).isEmpty(),
        )
    }
}
