pub mod radix;
pub mod sni;
pub mod tun;

pub use radix::{DomainFilter, FilterAction, IpFilter};
pub use sni::extract_sni;
pub use tun::{PacketSink, RawPacket};

use anyhow::Context as _;

/// Top-level entry point that wires a TUN adapter, domain filter, IP filter,
/// and SNI extractor into a running async packet-filter loop.
pub struct NetShield {
    domain_filter: DomainFilter,
    ip_filter: IpFilter,
    proxy_port: u16,
}

impl NetShield {
    pub fn new(domain_filter: DomainFilter, ip_filter: IpFilter, proxy_port: u16) -> Self {
        NetShield { domain_filter, ip_filter, proxy_port }
    }

    /// Run the filter loop.
    ///
    /// On Windows this opens a real Wintun adapter and loops until an
    /// unrecoverable error occurs or the future is dropped.  On other
    /// platforms the loop is a stub that returns immediately so the crate
    /// compiles for cross-platform development and testing.
    pub async fn run(self) -> anyhow::Result<()> {
        #[cfg(target_os = "windows")]
        {
            self.run_windows().await
        }
        #[cfg(not(target_os = "windows"))]
        {
            // stub — real packet loop only runs on Windows
            Ok(())
        }
    }

    #[cfg(target_os = "windows")]
    async fn run_windows(self) -> anyhow::Result<()> {
        use tun::{
            TunAdapter, dispatch, parse_ipv4_packet, parse_ipv6_packet,
            PORT_HTTP, PORT_HTTPS, PROTO_TCP,
        };

        let adapter =
            TunAdapter::open("HolyBlocker").context("TunAdapter::open")?;

        // Phase 1 sink: drop is implemented; pass and proxy are stubs because
        // both need a raw-socket relay (see docs/net-shield/PLAN.md step 5).
        struct WintunSink;
        impl PacketSink for WintunSink {
            fn drop_packet(&mut self, _pkt: &RawPacket) {
                // Discard the packet by simply not re-injecting it.
            }
            fn pass_packet(&mut self, _pkt: &RawPacket) {
                // TODO (step 5): re-inject via a raw socket so the packet
                // continues to its original destination.
            }
            fn redirect_to_proxy(&mut self, _pkt: &RawPacket, _proxy_port: u16) {
                // TODO (step 5): open a TCP connection to 127.0.0.1:proxy_port
                // and relay the original stream (userspace socket splice).
            }
        }

        let mut sink = WintunSink;
        let proxy_port = self.proxy_port;

        loop {
            let raw = adapter.recv_packet().context("recv_packet")?;

            // Try IPv4, then IPv6.
            let pkt = if let Some(p) = parse_ipv4_packet(&raw) {
                p
            } else if let Some(p) = parse_ipv6_packet(&raw) {
                p
            } else {
                continue;
            };

            // For TCP port 443 try to extract SNI from the payload.
            let action = if pkt.protocol == PROTO_TCP && pkt.dst_port == PORT_HTTPS {
                // SNI is in the TLS handshake that arrives in data packets
                // after the SYN.  We attempt to parse whatever bytes are in
                // the current datagram; the result may be None for SYN/ACK
                // frames that carry no application data.
                let ihl = ((raw[0] & 0x0f) as usize) * 4;
                let tcp_header_len = ((raw[ihl + 12] >> 4) as usize) * 4;
                let payload_start = ihl + tcp_header_len;
                let action = if let Some(hostname) =
                    raw.get(payload_start..).and_then(extract_sni)
                {
                    self.domain_filter.lookup(&hostname)
                } else {
                    self.ip_filter.lookup(pkt.dst_ip)
                };
                action
            } else if pkt.protocol == PROTO_TCP && pkt.dst_port == PORT_HTTP {
                self.ip_filter.lookup(pkt.dst_ip)
            } else {
                FilterAction::Allow
            };

            dispatch(&mut sink, &pkt, action, proxy_port);
        }
    }
}
