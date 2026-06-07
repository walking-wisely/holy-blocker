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

    /// Classify one raw IP datagram and dispatch it to `sink`.
    ///
    /// This is the platform-agnostic core of the filter loop.  It is called by
    /// the Windows Wintun loop and by unit tests that supply a fake sink.
    /// Returns `false` if the packet could not be parsed and was silently ignored.
    pub fn process_packet(&self, raw: &[u8], sink: &mut dyn PacketSink, proxy_port: u16) -> bool {
        use tun::{dispatch, parse_ipv4_packet, parse_ipv6_packet, PORT_HTTP, PORT_HTTPS, PROTO_TCP};

        let pkt = if let Some(p) = parse_ipv4_packet(raw) {
            p
        } else if let Some(p) = parse_ipv6_packet(raw) {
            p
        } else {
            return false;
        };

        let action = if pkt.protocol == PROTO_TCP && pkt.dst_port == PORT_HTTPS {
            // SNI lives in the TLS handshake payload that follows the TCP header.
            let ihl = ((raw[0] & 0x0f) as usize) * 4;
            let tcp_header_len = ((raw[ihl + 12] >> 4) as usize) * 4;
            let payload_start = ihl + tcp_header_len;
            if let Some(hostname) = raw.get(payload_start..).and_then(extract_sni) {
                self.domain_filter.lookup(&hostname)
            } else {
                self.ip_filter.lookup(pkt.dst_ip)
            }
        } else if pkt.protocol == PROTO_TCP && pkt.dst_port == PORT_HTTP {
            self.ip_filter.lookup(pkt.dst_ip)
        } else {
            FilterAction::Allow
        };

        dispatch(sink, &pkt, action, proxy_port);
        true
    }

    #[cfg(target_os = "windows")]
    async fn run_windows(self) -> anyhow::Result<()> {
        use tun::TunAdapter;

        let adapter = TunAdapter::open("HolyBlocker").context("TunAdapter::open")?;

        // Phase 1 sink: drop is implemented; pass and proxy are stubs because
        // both need a raw-socket relay (see docs/net-shield/PLAN.md step 5).
        struct WintunSink;
        impl PacketSink for WintunSink {
            fn drop_packet(&mut self, _pkt: &RawPacket) {
                // Discard by not re-injecting.
            }
            fn pass_packet(&mut self, _pkt: &RawPacket) {
                // TODO (step 5): re-inject via a raw socket.
            }
            fn redirect_to_proxy(&mut self, _pkt: &RawPacket, _proxy_port: u16) {
                // TODO (step 5): splice to 127.0.0.1:proxy_port.
            }
        }

        let mut sink = WintunSink;
        let proxy_port = self.proxy_port;

        loop {
            let raw = adapter.recv_packet().context("recv_packet")?;
            self.process_packet(&raw, &mut sink, proxy_port);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tun::RawPacket;

    #[derive(Default)]
    struct FakeSink {
        dropped: Vec<Vec<u8>>,
        passed: Vec<Vec<u8>>,
        proxied: Vec<(Vec<u8>, u16)>,
    }

    impl PacketSink for FakeSink {
        fn drop_packet(&mut self, pkt: &RawPacket) {
            self.dropped.push(pkt.bytes.clone());
        }
        fn pass_packet(&mut self, pkt: &RawPacket) {
            self.passed.push(pkt.bytes.clone());
        }
        fn redirect_to_proxy(&mut self, pkt: &RawPacket, port: u16) {
            self.proxied.push((pkt.bytes.clone(), port));
        }
    }

    /// Build a minimal well-formed TLS 1.2 ClientHello carrying the given SNI.
    ///
    /// Wire format follows RFC 8446 §5.1 (record), §4 (handshake header),
    /// §4.1.2 (ClientHello body), and RFC 6066 §3 (SNI extension).
    fn make_client_hello(sni: &str) -> Vec<u8> {
        let name_bytes = sni.as_bytes();
        let name_len = name_bytes.len();

        // SNI extension body: list_len(2) + name_type(1) + name_len(2) + name
        // RFC 6066 §3
        let sni_list_len = 1 + 2 + name_len;           // name_type + name_len + name
        let sni_ext_data_len = 2 + sni_list_len;        // list_len field + list body
        let sni_ext_len = 2 + 2 + sni_ext_data_len;     // ext_type(2) + ext_len(2) + data

        // ClientHello body: legacy_version(2) + random(32) + sid_len(1) +
        //   cipher_suites_len(2) + cipher_suite(2) + comp_methods_len(1) +
        //   comp_method(1) + extensions_len(2) + sni_ext
        let exts_len = sni_ext_len;
        let ch_body_len = 2 + 32 + 1 + 2 + 2 + 1 + 1 + 2 + exts_len;

        // Handshake header: msg_type(1) + length(3)
        let hs_len = ch_body_len;

        // TLS record header: content_type(1) + legacy_version(2) + length(2)
        let record_len = 1 + 3 + hs_len; // handshake header + body

        let mut buf = Vec::new();

        // TLS record header
        buf.push(0x16);                                          // Handshake
        buf.extend_from_slice(&[0x03, 0x01]);                   // legacy version TLS 1.0
        buf.extend_from_slice(&(record_len as u16).to_be_bytes());

        // Handshake header
        buf.push(0x01);                                          // ClientHello
        buf.push(((hs_len >> 16) & 0xff) as u8);
        buf.push(((hs_len >> 8) & 0xff) as u8);
        buf.push((hs_len & 0xff) as u8);

        // ClientHello body
        buf.extend_from_slice(&[0x03, 0x03]);                   // legacy_version TLS 1.2
        buf.extend_from_slice(&[0u8; 32]);                      // random
        buf.push(0x00);                                          // session_id_len = 0
        buf.extend_from_slice(&[0x00, 0x02]);                   // cipher_suites_len = 2
        buf.extend_from_slice(&[0x00, 0x2f]);                   // TLS_RSA_WITH_AES_128_CBC_SHA
        buf.push(0x01);                                          // compression_methods_len = 1
        buf.push(0x00);                                          // null compression

        // extensions_len
        buf.extend_from_slice(&(exts_len as u16).to_be_bytes());

        // SNI extension: type=0x0000, len, data
        buf.extend_from_slice(&[0x00, 0x00]);                   // extension_type = server_name
        buf.extend_from_slice(&(sni_ext_data_len as u16).to_be_bytes());
        buf.extend_from_slice(&(sni_list_len as u16).to_be_bytes()); // ServerNameList length
        buf.push(0x00);                                          // name_type = host_name
        buf.extend_from_slice(&(name_len as u16).to_be_bytes());
        buf.extend_from_slice(name_bytes);

        buf
    }

    /// Build an IPv4/TCP packet to port 443 whose TCP payload is `payload`.
    fn make_ipv4_tcp_443_with_payload(payload: &[u8]) -> Vec<u8> {
        let ip_header_len = 20;
        let tcp_header_len = 20;
        let total_len = ip_header_len + tcp_header_len + payload.len();

        let mut buf = vec![0u8; total_len];

        // IPv4 header — RFC 791 §3.1
        buf[0] = 0x45;  // version=4, IHL=5 (20 bytes)
        buf[2..4].copy_from_slice(&(total_len as u16).to_be_bytes()); // total length
        buf[9] = 6;     // protocol = TCP
        buf[12..16].copy_from_slice(&[10, 0, 0, 2]);  // src 10.0.0.2
        buf[16..20].copy_from_slice(&[93, 184, 216, 34]); // dst 93.184.216.34 (example.com)

        // TCP header — RFC 9293 §3.1
        buf[20..22].copy_from_slice(&60000u16.to_be_bytes()); // src port
        buf[22..24].copy_from_slice(&443u16.to_be_bytes());   // dst port 443
        buf[32] = 0x50; // data offset = 5 (20 bytes), flags = 0

        // TCP payload
        buf[40..].copy_from_slice(payload);

        buf
    }

    #[test]
    fn smoke_block_domain() {
        let domain_filter = DomainFilter::from_rules(&[("ads.example.com", FilterAction::Block)]);
        let ip_filter = IpFilter::from_rules(&[]).unwrap();
        let shield = NetShield::new(domain_filter, ip_filter, 8080);

        let tls_payload = make_client_hello("ads.example.com");
        let raw_packet = make_ipv4_tcp_443_with_payload(&tls_payload);

        let mut sink = FakeSink::default();
        let processed = shield.process_packet(&raw_packet, &mut sink, 8080);

        assert!(processed, "packet should be parseable");
        assert_eq!(sink.dropped.len(), 1, "blocked domain should be dropped");
        assert!(sink.passed.is_empty(), "should not be passed");
        assert!(sink.proxied.is_empty(), "should not be proxied");
    }

    #[test]
    fn smoke_proxy_unknown_domain() {
        // DomainFilter returns Proxy on a miss — unrecognised domains are sent
        // to mitm-proxy for deeper inspection rather than passed through raw.
        let domain_filter = DomainFilter::from_rules(&[("ads.example.com", FilterAction::Block)]);
        let ip_filter = IpFilter::from_rules(&[]).unwrap();
        let shield = NetShield::new(domain_filter, ip_filter, 8080);

        let tls_payload = make_client_hello("safe.example.com");
        let raw_packet = make_ipv4_tcp_443_with_payload(&tls_payload);

        let mut sink = FakeSink::default();
        shield.process_packet(&raw_packet, &mut sink, 8080);

        assert_eq!(sink.proxied.len(), 1, "unknown domain should be proxied");
        assert_eq!(sink.proxied[0].1, 8080);
        assert!(sink.dropped.is_empty());
        assert!(sink.passed.is_empty());
    }

    #[test]
    fn smoke_unrecognised_packet_ignored() {
        let shield = NetShield::new(
            DomainFilter::from_rules(&[]),
            IpFilter::from_rules(&[]).unwrap(),
            8080,
        );
        let mut sink = FakeSink::default();
        let processed = shield.process_packet(&[0xde, 0xad, 0xbe, 0xef], &mut sink, 8080);
        assert!(!processed);
        assert!(sink.dropped.is_empty());
        assert!(sink.passed.is_empty());
        assert!(sink.proxied.is_empty());
    }
}
