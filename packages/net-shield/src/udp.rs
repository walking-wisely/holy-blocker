//! IPv4/UDP framing for a TUN device.
//!
//! A TUN interface carries bare IP datagrams, so a filter that answers a
//! request itself — rather than forwarding it — has to build the reply packet
//! by hand, checksums included. This module is that framing and nothing else:
//! it has no opinion about the payload it carries.
//!
//! IPv4 only, deliberately. The Android VPN in `apps/mobile` addresses its TUN
//! itself and gives it a single IPv4 resolver address, so nothing else can
//! arrive; keeping the module to one address family keeps one checksum
//! pseudo-header instead of two.
//!
//! Wire formats: RFC 791 §3.1 (IPv4 header), RFC 768 (UDP header and the
//! checksum pseudo-header), RFC 1071 (the checksum algorithm itself).

use std::net::Ipv4Addr;

use crate::tun::PROTO_UDP;

/// Minimum IPv4 header size, IHL=5 with no options — RFC 791 §3.1.
const IPV4_MIN_HEADER_LEN: usize = 20;
/// Fixed UDP header size: source, destination, length, checksum — RFC 768.
const UDP_HEADER_LEN: usize = 8;
/// An IPv4 total-length field is 16 bits — RFC 791 §3.1.
const IPV4_MAX_TOTAL_LEN: usize = u16::MAX as usize;

/// "More fragments" flag, bit 13 of the flags/fragment-offset word — RFC 791 §3.1.
const IPV4_FLAG_MORE_FRAGMENTS: u16 = 0x2000;
/// Fragment offset occupies the low 13 bits of the same word — RFC 791 §3.1.
const IPV4_FRAGMENT_OFFSET_MASK: u16 = 0x1fff;

/// Default TTL for datagrams this module originates. 64 is the IANA-recommended
/// default for the "IP TIME TO LIVE parameter"; RFC 1122 §3.2.1.7 requires the
/// value be configurable but recommends a value in this range. The reply never
/// leaves the local TUN, so any value above 1 would do.
const DEFAULT_TTL: u8 = 64;

/// A parsed IPv4/UDP datagram, borrowing its payload from the input buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ipv4UdpDatagram<'a> {
    pub src_ip: Ipv4Addr,
    pub dst_ip: Ipv4Addr,
    pub src_port: u16,
    pub dst_port: u16,
    pub payload: &'a [u8],
}

/// Parse an IPv4/UDP datagram read from a TUN device.
///
/// Returns `None` unless the buffer is a complete, unfragmented IPv4 datagram
/// carrying UDP. Fragments are refused rather than reassembled: the payload
/// this filter reads lives in the first fragment only some of the time, and a
/// reassembly buffer is state an on-device filter should not carry.
pub fn parse_ipv4_udp(packet: &[u8]) -> Option<Ipv4UdpDatagram<'_>> {
    if packet.len() < IPV4_MIN_HEADER_LEN {
        return None;
    }
    if packet[0] >> 4 != 4 {
        return None;
    }
    let ihl = ((packet[0] & 0x0f) as usize) * 4;
    if ihl < IPV4_MIN_HEADER_LEN {
        return None;
    }

    // The total-length field is authoritative; a TUN read can hand back a
    // buffer longer than the datagram, and trusting the buffer instead would
    // let trailing bytes into the payload.
    let total_len = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if total_len < ihl || packet.len() < total_len {
        return None;
    }

    let frag = u16::from_be_bytes([packet[6], packet[7]]);
    if frag & IPV4_FLAG_MORE_FRAGMENTS != 0 || frag & IPV4_FRAGMENT_OFFSET_MASK != 0 {
        return None;
    }

    if packet[9] != PROTO_UDP {
        return None;
    }

    let src_ip = Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]);
    let dst_ip = Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]);

    let udp = packet.get(ihl..total_len)?;
    if udp.len() < UDP_HEADER_LEN {
        return None;
    }
    let src_port = u16::from_be_bytes([udp[0], udp[1]]);
    let dst_port = u16::from_be_bytes([udp[2], udp[3]]);

    // The UDP length field covers the header plus the payload — RFC 768.
    let udp_len = u16::from_be_bytes([udp[4], udp[5]]) as usize;
    if udp_len < UDP_HEADER_LEN || udp_len > udp.len() {
        return None;
    }

    Some(Ipv4UdpDatagram {
        src_ip,
        dst_ip,
        src_port,
        dst_port,
        payload: &udp[UDP_HEADER_LEN..udp_len],
    })
}

/// Build a complete IPv4/UDP datagram carrying `payload`.
///
/// Returns `None` if the result would overflow the IPv4 total-length field.
pub fn build_ipv4_udp(
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Option<Vec<u8>> {
    let udp_len = UDP_HEADER_LEN.checked_add(payload.len())?;
    let total_len = IPV4_MIN_HEADER_LEN.checked_add(udp_len)?;
    if total_len > IPV4_MAX_TOTAL_LEN {
        return None;
    }

    let mut packet = vec![0u8; total_len];

    // --- IPv4 header — RFC 791 §3.1 -------------------------------------
    packet[0] = 0x45; // Version 4, IHL 5 (20 bytes, no options)
    packet[1] = 0; // DSCP/ECN — RFC 2474 §3, RFC 3168 §5; default class
    packet[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    // Identification 0: RFC 6864 §4.1 permits any value in an atomic (never
    // fragmented) datagram, and this one never leaves the local TUN.
    packet[4..6].copy_from_slice(&0u16.to_be_bytes());
    packet[6..8].copy_from_slice(&0u16.to_be_bytes()); // flags + fragment offset
    packet[8] = DEFAULT_TTL;
    packet[9] = PROTO_UDP;
    // packet[10..12] is the header checksum, computed below over a zeroed field.
    packet[12..16].copy_from_slice(&src_ip.octets());
    packet[16..20].copy_from_slice(&dst_ip.octets());
    let header_checksum = ones_complement_checksum(&[&packet[..IPV4_MIN_HEADER_LEN]]);
    packet[10..12].copy_from_slice(&header_checksum.to_be_bytes());

    // --- UDP header — RFC 768 -------------------------------------------
    let udp = &mut packet[IPV4_MIN_HEADER_LEN..];
    udp[0..2].copy_from_slice(&src_port.to_be_bytes());
    udp[2..4].copy_from_slice(&dst_port.to_be_bytes());
    udp[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    // udp[6..8] is the checksum, computed below over a zeroed field.
    udp[UDP_HEADER_LEN..].copy_from_slice(payload);

    let checksum = udp_checksum(src_ip, dst_ip, &packet[IPV4_MIN_HEADER_LEN..]);
    packet[IPV4_MIN_HEADER_LEN + 6..IPV4_MIN_HEADER_LEN + 8].copy_from_slice(&checksum.to_be_bytes());

    Some(packet)
}

/// Build the datagram that answers `request`, swapping the endpoints.
pub fn reply_to(request: &Ipv4UdpDatagram<'_>, payload: &[u8]) -> Option<Vec<u8>> {
    build_ipv4_udp(
        request.dst_ip,
        request.src_ip,
        request.dst_port,
        request.src_port,
        payload,
    )
}

/// UDP checksum over IPv4, including the pseudo-header — RFC 768.
///
/// `udp` must be the complete UDP header and payload with the checksum field
/// already zeroed.
fn udp_checksum(src_ip: Ipv4Addr, dst_ip: Ipv4Addr, udp: &[u8]) -> u16 {
    // Pseudo-header: source address, destination address, a zero octet, the
    // protocol number, and the UDP length — RFC 768.
    let mut pseudo = [0u8; 12];
    pseudo[0..4].copy_from_slice(&src_ip.octets());
    pseudo[4..8].copy_from_slice(&dst_ip.octets());
    pseudo[8] = 0;
    pseudo[9] = PROTO_UDP;
    pseudo[10..12].copy_from_slice(&(udp.len() as u16).to_be_bytes());

    let sum = ones_complement_checksum(&[&pseudo, udp]);
    // "If the computed checksum is zero, it is transmitted as all ones" — RFC
    // 768 — because zero is the value that means "no checksum sent".
    if sum == 0 { 0xffff } else { sum }
}

/// One's-complement of the one's-complement sum of 16-bit words — RFC 1071 §1.
///
/// Takes several byte runs so a pseudo-header can be summed with the datagram
/// without concatenating them first. Each run is padded independently, which is
/// safe here because every caller passes even-length runs except the last.
fn ones_complement_checksum(runs: &[&[u8]]) -> u16 {
    let mut sum: u32 = 0;
    for run in runs {
        let mut i = 0;
        while i + 1 < run.len() {
            sum += u16::from_be_bytes([run[i], run[i + 1]]) as u32;
            i += 2;
        }
        if i < run.len() {
            // Odd trailing byte is padded on the right with zero — RFC 1071 §1.
            sum += (run[i] as u32) << 8;
        }
        while sum >> 16 != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    /// Verify a checksum the way a receiver does: summing the covered bytes
    /// *including* the stored checksum must yield zero — RFC 1071 §1.
    fn verify(runs: &[&[u8]]) -> bool {
        ones_complement_checksum(runs) == 0
    }

    fn sample() -> Vec<u8> {
        build_ipv4_udp(ip("10.0.0.2"), ip("10.0.0.1"), 40000, 53, b"payload").unwrap()
    }

    #[test]
    fn round_trips_a_built_datagram() {
        let packet = build_ipv4_udp(ip("10.0.0.2"), ip("1.1.1.1"), 40000, 53, b"hello").unwrap();
        let parsed = parse_ipv4_udp(&packet).expect("parses");
        assert_eq!(parsed.src_ip, ip("10.0.0.2"));
        assert_eq!(parsed.dst_ip, ip("1.1.1.1"));
        assert_eq!(parsed.src_port, 40000);
        assert_eq!(parsed.dst_port, 53);
        assert_eq!(parsed.payload, b"hello");
    }

    #[test]
    fn built_header_checksum_verifies() {
        let packet = sample();
        assert!(verify(&[&packet[..IPV4_MIN_HEADER_LEN]]), "IPv4 header checksum");
    }

    #[test]
    fn built_udp_checksum_verifies_against_the_pseudo_header() {
        let packet = sample();
        let mut pseudo = [0u8; 12];
        pseudo[0..4].copy_from_slice(&packet[12..16]);
        pseudo[4..8].copy_from_slice(&packet[16..20]);
        pseudo[9] = PROTO_UDP;
        let udp = &packet[IPV4_MIN_HEADER_LEN..];
        pseudo[10..12].copy_from_slice(&(udp.len() as u16).to_be_bytes());
        assert!(verify(&[&pseudo, udp]), "UDP checksum");
    }

    #[test]
    fn checksums_verify_for_odd_and_even_payload_lengths() {
        // The odd case exercises the trailing-byte padding in RFC 1071 §1,
        // which is the easiest half of the algorithm to get wrong.
        for len in 0..9usize {
            let payload = vec![0xa5u8; len];
            let packet = build_ipv4_udp(ip("10.0.0.2"), ip("10.0.0.1"), 1, 53, &payload).unwrap();
            assert!(verify(&[&packet[..IPV4_MIN_HEADER_LEN]]), "header, len={len}");

            let mut pseudo = [0u8; 12];
            pseudo[0..4].copy_from_slice(&packet[12..16]);
            pseudo[4..8].copy_from_slice(&packet[16..20]);
            pseudo[9] = PROTO_UDP;
            let udp = &packet[IPV4_MIN_HEADER_LEN..];
            pseudo[10..12].copy_from_slice(&(udp.len() as u16).to_be_bytes());
            assert!(verify(&[&pseudo, udp]), "udp, len={len}");
        }
    }

    #[test]
    fn udp_checksum_is_never_transmitted_as_zero() {
        // RFC 768 reserves an all-zero checksum for "no checksum computed", so
        // a genuine zero must go out as 0xFFFF.
        //
        // Contrived to land exactly on that value: with zero addresses and
        // ports, the covered words are the pseudo-header protocol (0x0011),
        // the length twice (pseudo-header and UDP header, 0x000a each), and
        // the payload — so a payload of 0xFFDA makes the sum 0xFFFF and the
        // complement 0.
        let udp = [0, 0, 0, 0, 0, 10, 0, 0, 0xff, 0xda];
        let mut pseudo = [0u8; 12];
        pseudo[9] = PROTO_UDP;
        pseudo[10..12].copy_from_slice(&(udp.len() as u16).to_be_bytes());
        assert_eq!(
            ones_complement_checksum(&[&pseudo, &udp]),
            0,
            "test input must actually produce a zero checksum"
        );

        assert_eq!(udp_checksum(ip("0.0.0.0"), ip("0.0.0.0"), &udp), 0xffff);
    }

    #[test]
    fn reply_swaps_the_endpoints() {
        let request = build_ipv4_udp(ip("10.0.0.2"), ip("10.0.0.1"), 40000, 53, b"q").unwrap();
        let parsed = parse_ipv4_udp(&request).unwrap();
        let reply = reply_to(&parsed, b"answer").unwrap();
        let parsed_reply = parse_ipv4_udp(&reply).unwrap();

        assert_eq!(parsed_reply.src_ip, parsed.dst_ip);
        assert_eq!(parsed_reply.dst_ip, parsed.src_ip);
        assert_eq!(parsed_reply.src_port, parsed.dst_port);
        assert_eq!(parsed_reply.dst_port, parsed.src_port);
        assert_eq!(parsed_reply.payload, b"answer");
    }

    #[test]
    fn trailing_bytes_beyond_total_length_stay_out_of_the_payload() {
        // A TUN read can return a buffer larger than the datagram. The
        // total-length field, not the buffer length, bounds the payload.
        let mut packet = sample();
        packet.extend_from_slice(b"garbage");
        let parsed = parse_ipv4_udp(&packet).expect("parses");
        assert_eq!(parsed.payload, b"payload");
    }

    #[test]
    fn short_udp_length_field_bounds_the_payload() {
        let mut packet = sample();
        let udp_off = IPV4_MIN_HEADER_LEN;
        let shortened = (UDP_HEADER_LEN + 3) as u16;
        packet[udp_off + 4..udp_off + 6].copy_from_slice(&shortened.to_be_bytes());
        assert_eq!(parse_ipv4_udp(&packet).unwrap().payload, b"pay");
    }

    #[test]
    fn fragments_are_refused() {
        // More-fragments set, then a non-zero offset — RFC 791 §3.1.
        for frag in [IPV4_FLAG_MORE_FRAGMENTS, 1u16] {
            let mut packet = sample();
            packet[6..8].copy_from_slice(&frag.to_be_bytes());
            assert!(parse_ipv4_udp(&packet).is_none(), "frag word {frag:#06x}");
        }
    }

    #[test]
    fn non_udp_and_non_ipv4_are_refused() {
        let mut tcp = sample();
        tcp[9] = crate::tun::PROTO_TCP;
        assert!(parse_ipv4_udp(&tcp).is_none(), "TCP");

        let mut v6 = sample();
        v6[0] = 0x60;
        assert!(parse_ipv4_udp(&v6).is_none(), "IPv6");
    }

    #[test]
    fn header_options_are_skipped_to_find_the_udp_header() {
        // IHL 6 — one 32-bit option word before the transport header, RFC 791 §3.1.
        let payload = b"opt";
        let mut packet = vec![0u8; 24 + UDP_HEADER_LEN + payload.len()];
        packet[0] = 0x46;
        let total = packet.len() as u16;
        packet[2..4].copy_from_slice(&total.to_be_bytes());
        packet[9] = PROTO_UDP;
        packet[12..16].copy_from_slice(&ip("10.0.0.2").octets());
        packet[16..20].copy_from_slice(&ip("10.0.0.1").octets());
        packet[24..26].copy_from_slice(&40000u16.to_be_bytes());
        packet[26..28].copy_from_slice(&53u16.to_be_bytes());
        let udp_len = (UDP_HEADER_LEN + payload.len()) as u16;
        packet[28..30].copy_from_slice(&udp_len.to_be_bytes());
        packet[32..].copy_from_slice(payload);

        let parsed = parse_ipv4_udp(&packet).expect("parses");
        assert_eq!(parsed.dst_port, 53);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn truncated_buffers_are_refused_without_panicking() {
        let full = sample();
        for cut in 0..full.len() {
            assert!(parse_ipv4_udp(&full[..cut]).is_none(), "cut at {cut}");
        }
        assert!(parse_ipv4_udp(&full).is_some());
    }

    #[test]
    fn oversized_payload_is_refused() {
        let payload = vec![0u8; IPV4_MAX_TOTAL_LEN];
        assert!(build_ipv4_udp(ip("10.0.0.2"), ip("10.0.0.1"), 1, 53, &payload).is_none());
    }
}
