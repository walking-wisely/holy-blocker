use std::net::IpAddr;

// ---------------------------------------------------------------------------
// Protocol and port constants
//
// Protocol numbers: IANA "Protocol Numbers" registry (RFC 790 / RFC 791 §3.1).
// Port numbers:     IANA "Service Name and Transport Protocol Port Number
//                   Registry".
// ---------------------------------------------------------------------------

/// IANA IP protocol number for TCP (RFC 793).
pub const PROTO_TCP: u8 = 6;
/// IANA IP protocol number for UDP (RFC 768).
pub const PROTO_UDP: u8 = 17;
/// Well-known port for HTTP (RFC 9110).
pub const PORT_HTTP: u16 = 80;
/// Well-known port for HTTPS / TLS (RFC 9110).
pub const PORT_HTTPS: u16 = 443;

// ---------------------------------------------------------------------------
// Shared types (all platforms)
// ---------------------------------------------------------------------------

/// A raw IP packet captured from the TUN adapter together with its parsed
/// 5-tuple.  The `bytes` field contains the full IP datagram as read from the
/// Wintun ring buffer.
pub struct RawPacket {
    pub bytes: Vec<u8>,
    pub src_ip: IpAddr,
    pub dst_ip: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8,
}

/// Routing sink called by the filter loop for every classified packet.
///
/// The trait decouples routing decisions from the real Wintun device so the
/// dispatch logic can be tested with an in-memory fake sink.
pub trait PacketSink: Send {
    fn drop_packet(&mut self, pkt: &RawPacket);
    fn pass_packet(&mut self, pkt: &RawPacket);
    fn redirect_to_proxy(&mut self, pkt: &RawPacket, proxy_port: u16);
}

// ---------------------------------------------------------------------------
// Packet parsing helpers (all platforms)
// ---------------------------------------------------------------------------

/// Parse the 5-tuple from a raw IPv4 datagram.
///
/// Returns `None` if the buffer is too short or the IP version is not 4.
pub fn parse_ipv4_packet(buf: &[u8]) -> Option<RawPacket> {
    // 20 = minimum IPv4 header size (IHL=5, no options) per RFC 791 §3.1.
    if buf.len() < 20 {
        return None;
    }
    let version = buf[0] >> 4;
    if version != 4 {
        return None;
    }
    let ihl = ((buf[0] & 0x0f) as usize) * 4;
    if buf.len() < ihl + 4 {
        return None;
    }

    let protocol = buf[9];
    let src_ip = IpAddr::V4(std::net::Ipv4Addr::new(buf[12], buf[13], buf[14], buf[15]));
    let dst_ip = IpAddr::V4(std::net::Ipv4Addr::new(buf[16], buf[17], buf[18], buf[19]));

    let (src_port, dst_port) = if protocol == PROTO_TCP || protocol == PROTO_UDP {
        // TCP or UDP: first four bytes of the transport header are src/dst port
        // (RFC 793 §3.1 for TCP; RFC 768 for UDP — both use the same layout).
        if buf.len() < ihl + 4 {
            return None;
        }
        let src = u16::from_be_bytes([buf[ihl], buf[ihl + 1]]);
        let dst = u16::from_be_bytes([buf[ihl + 2], buf[ihl + 3]]);
        (src, dst)
    } else {
        (0, 0)
    };

    Some(RawPacket {
        bytes: buf.to_vec(),
        src_ip,
        dst_ip,
        src_port,
        dst_port,
        protocol,
    })
}

/// Parse the 5-tuple from a raw IPv6 datagram.
///
/// Returns `None` if the buffer is too short or the IP version is not 6.
pub fn parse_ipv6_packet(buf: &[u8]) -> Option<RawPacket> {
    // 40 = fixed IPv6 header size per RFC 8200 §3 (no variable IHL field).
    if buf.len() < 40 {
        return None;
    }
    let version = buf[0] >> 4;
    if version != 6 {
        return None;
    }

    let next_header = buf[6];
    let src_ip = IpAddr::V6(std::net::Ipv6Addr::from(
        <[u8; 16]>::try_from(&buf[8..24]).ok()?,
    ));
    let dst_ip = IpAddr::V6(std::net::Ipv6Addr::from(
        <[u8; 16]>::try_from(&buf[24..40]).ok()?,
    ));

    let (src_port, dst_port) = if next_header == PROTO_TCP || next_header == PROTO_UDP {
        if buf.len() < 44 {
            return None;
        }
        let src = u16::from_be_bytes([buf[40], buf[41]]);
        let dst = u16::from_be_bytes([buf[42], buf[43]]);
        (src, dst)
    } else {
        (0, 0)
    };

    Some(RawPacket {
        bytes: buf.to_vec(),
        src_ip,
        dst_ip,
        src_port,
        dst_port,
        protocol: next_header,
    })
}

/// Dispatch a parsed packet to `sink` based on `action`.
///
/// `proxy_port` is only used when the action is `Proxy`.
pub fn dispatch<S: PacketSink + ?Sized>(
    sink: &mut S,
    pkt: &RawPacket,
    action: crate::FilterAction,
    proxy_port: u16,
) {
    match action {
        crate::FilterAction::Block => sink.drop_packet(pkt),
        crate::FilterAction::Allow => sink.pass_packet(pkt),
        crate::FilterAction::Proxy => sink.redirect_to_proxy(pkt, proxy_port),
    }
}

// ---------------------------------------------------------------------------
// Windows TUN adapter
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub use windows::TunAdapter;

#[cfg(target_os = "windows")]
mod windows {
    use anyhow::Context as _;
    use std::sync::Arc;

    /// Wraps a Wintun virtual adapter and reads raw IP packets from its ring
    /// buffer.  Constructed by `NetShield::new` on Windows targets.
    pub struct TunAdapter {
        session: Arc<wintun::Session>,
    }

    impl TunAdapter {
        pub fn open(adapter_name: &str) -> anyhow::Result<Self> {
            let wintun = unsafe { wintun::load() }
                .context("failed to load wintun.dll")?;
            let adapter = wintun::Adapter::open(&wintun, adapter_name)
                .or_else(|_| {
                    wintun::Adapter::create(&wintun, adapter_name, "HolyBlocker", None)
                })
                .context("failed to open/create Wintun adapter")?;
            let session = Arc::new(
                adapter
                    .start_session(wintun::MAX_RING_CAPACITY)
                    .context("failed to start Wintun session")?,
            );
            Ok(TunAdapter { session })
        }

        /// Read one packet from the Wintun ring buffer, blocking until one arrives.
        pub fn recv_packet(&self) -> anyhow::Result<Vec<u8>> {
            let pkt = self
                .session
                .receive_blocking()
                .context("Wintun receive error")?;
            Ok(pkt.bytes().to_vec())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{
        dispatch, parse_ipv4_packet, parse_ipv6_packet, IpAddr, PacketSink, RawPacket,
    };
    use crate::FilterAction;

    /// In-memory sink that records every routing decision for assertions.
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

    // --- IPv4 parsing ---

    fn make_ipv4_tcp(src: [u8; 4], dst: [u8; 4], src_port: u16, dst_port: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 40]; // 20-byte IP header + 20-byte TCP header
        buf[0] = 0x45; // version=4, IHL=5
        buf[9] = 6;    // TCP
        buf[12..16].copy_from_slice(&src);
        buf[16..20].copy_from_slice(&dst);
        buf[20..22].copy_from_slice(&src_port.to_be_bytes());
        buf[22..24].copy_from_slice(&dst_port.to_be_bytes());
        buf
    }

    #[test]
    fn parse_ipv4_extracts_five_tuple() {
        let buf = make_ipv4_tcp([1, 2, 3, 4], [5, 6, 7, 8], 54321, 443);
        let pkt = parse_ipv4_packet(&buf).unwrap();
        assert_eq!(pkt.src_ip, "1.2.3.4".parse::<IpAddr>().unwrap());
        assert_eq!(pkt.dst_ip, "5.6.7.8".parse::<IpAddr>().unwrap());
        assert_eq!(pkt.src_port, 54321);
        assert_eq!(pkt.dst_port, 443);
        assert_eq!(pkt.protocol, 6);
    }

    #[test]
    fn parse_ipv4_returns_none_on_short_buffer() {
        assert!(parse_ipv4_packet(&[0x45u8; 10]).is_none());
    }

    #[test]
    fn parse_ipv4_returns_none_for_wrong_version() {
        let mut buf = make_ipv4_tcp([1, 2, 3, 4], [5, 6, 7, 8], 80, 80);
        buf[0] = 0x65; // version=6
        assert!(parse_ipv4_packet(&buf).is_none());
    }

    // --- IPv6 parsing ---

    fn make_ipv6_tcp(dst_port: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 60]; // 40-byte IP header + 20-byte TCP
        buf[0] = 0x60; // version=6
        buf[6] = 6;    // next header: TCP
        // src: ::1, dst: ::2
        buf[23] = 1;
        buf[39] = 2;
        buf[42..44].copy_from_slice(&dst_port.to_be_bytes());
        buf
    }

    #[test]
    fn parse_ipv6_extracts_dst_port() {
        let buf = make_ipv6_tcp(443);
        let pkt = parse_ipv6_packet(&buf).unwrap();
        assert_eq!(pkt.dst_port, 443);
        assert_eq!(pkt.protocol, 6);
    }

    #[test]
    fn parse_ipv6_returns_none_on_short_buffer() {
        assert!(parse_ipv6_packet(&[0x60u8; 10]).is_none());
    }

    // --- dispatch ---

    #[test]
    fn dispatch_block_calls_drop() {
        let buf = make_ipv4_tcp([1, 2, 3, 4], [5, 6, 7, 8], 1234, 443);
        let pkt = parse_ipv4_packet(&buf).unwrap();
        let mut sink = FakeSink::default();
        dispatch(&mut sink, &pkt, FilterAction::Block, 8080);
        assert_eq!(sink.dropped.len(), 1);
        assert!(sink.passed.is_empty());
        assert!(sink.proxied.is_empty());
    }

    #[test]
    fn dispatch_allow_calls_pass() {
        let buf = make_ipv4_tcp([1, 2, 3, 4], [5, 6, 7, 8], 1234, 80);
        let pkt = parse_ipv4_packet(&buf).unwrap();
        let mut sink = FakeSink::default();
        dispatch(&mut sink, &pkt, FilterAction::Allow, 8080);
        assert_eq!(sink.passed.len(), 1);
        assert!(sink.dropped.is_empty());
        assert!(sink.proxied.is_empty());
    }

    #[test]
    fn dispatch_proxy_calls_redirect_with_port() {
        let buf = make_ipv4_tcp([1, 2, 3, 4], [5, 6, 7, 8], 1234, 443);
        let pkt = parse_ipv4_packet(&buf).unwrap();
        let mut sink = FakeSink::default();
        dispatch(&mut sink, &pkt, FilterAction::Proxy, 8080);
        assert_eq!(sink.proxied.len(), 1);
        assert_eq!(sink.proxied[0].1, 8080);
        assert!(sink.dropped.is_empty());
        assert!(sink.passed.is_empty());
    }
}
