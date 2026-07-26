//! The DNS filter as one pure decision over one TUN packet.
//!
//! This is the whole decision core of the Android VPN path. The edge that owns
//! the TUN file descriptor reads a packet, hands it here, and does exactly what
//! it is told: write bytes back, forward a query, or discard. No I/O, no
//! sockets, no clock — so the interesting half is unit-testable on any host,
//! and the Kotlin side stays free of wire formats.
//!
//! **Why DNS and not the full packet path.** A `VpnService` TUN cannot
//! re-inject a packet the way Wintun can (see [`crate::NetShield`]): on Android
//! the only way to let a flow through is to terminate it in userspace and
//! re-originate it on a `protect()`ed socket. For UDP that is a socket per
//! flow; for TCP it is a userspace TCP stack. Filtering DNS needs only the
//! former, and it is the step that decides whether a connection is attempted at
//! all. SNI and IP filtering, which need the latter, come after.

use crate::dns;
use crate::radix::{DomainFilter, FilterAction};
use crate::udp::{self, Ipv4UdpDatagram};

/// What the edge should do with a packet read from the TUN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsVerdict {
    /// Nothing here to filter — a malformed packet, a fragment, or traffic to
    /// something other than a resolver. The edge discards it.
    ///
    /// Discarding is safe *only because* the TUN is routed to the VPN's own
    /// resolver address and nothing else, so no real traffic can arrive here.
    Ignore,
    /// `name` is blocked. Write `reply` back into the TUN; nothing leaves the
    /// device.
    Blocked { name: String, reply: Vec<u8> },
    /// `name` is permitted. Send `query` to a real resolver over a protected
    /// socket, then pass the answer to [`DnsShield::wrap_response`].
    Forward { name: String, query: Vec<u8> },
}

/// Decides DNS queries against a [`DomainFilter`].
pub struct DnsShield {
    domains: DomainFilter,
}

impl DnsShield {
    pub fn new(domains: DomainFilter) -> Self {
        DnsShield { domains }
    }

    /// Classify one IPv4 packet read from the TUN.
    pub fn inspect(&self, packet: &[u8]) -> DnsVerdict {
        let Some(datagram) = udp::parse_ipv4_udp(packet) else {
            return DnsVerdict::Ignore;
        };
        if datagram.dst_port != dns::PORT_DNS {
            return DnsVerdict::Ignore;
        }
        let Some(question) = dns::parse_query(datagram.payload) else {
            return DnsVerdict::Ignore;
        };

        match self.domains.lookup(&question.name) {
            // `Proxy` is DomainFilter's miss default — "unrecognised, inspect
            // it more deeply downstream". There is no deeper DNS inspection to
            // hand it to, so on this path it means the same thing as `Allow`:
            // let the real resolver answer. The name is still seen again at
            // connect time by the SNI/IP path once that exists.
            FilterAction::Allow | FilterAction::Proxy => DnsVerdict::Forward {
                name: question.name,
                query: datagram.payload.to_vec(),
            },
            FilterAction::Block => match blocked_reply(&datagram) {
                Some(reply) => DnsVerdict::Blocked {
                    name: question.name,
                    reply,
                },
                // Unreachable in practice: the query parsed, so the refusal
                // builds. Refusing to guess keeps the failure a dropped packet
                // — which the client retries — rather than a malformed one.
                None => DnsVerdict::Ignore,
            },
        }
    }

    /// Frame a resolver's answer as a packet addressed back to the client that
    /// sent `request_packet`.
    ///
    /// `request_packet` is the same buffer that produced a
    /// [`DnsVerdict::Forward`]; the edge holds it while the resolver round-trip
    /// is in flight, because the reply's addressing comes entirely from it.
    pub fn wrap_response(&self, request_packet: &[u8], response: &[u8]) -> Option<Vec<u8>> {
        let datagram = udp::parse_ipv4_udp(request_packet)?;
        udp::reply_to(&datagram, response)
    }
}

fn blocked_reply(datagram: &Ipv4UdpDatagram<'_>) -> Option<Vec<u8>> {
    let refusal = dns::nxdomain_response(datagram.payload)?;
    udp::reply_to(datagram, &refusal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const CLIENT: Ipv4Addr = Ipv4Addr::new(10, 111, 0, 2);
    const RESOLVER: Ipv4Addr = Ipv4Addr::new(10, 111, 0, 1);
    const CLIENT_PORT: u16 = 41234;

    /// QTYPE 1 = A, QCLASS 1 = IN — RFC 1035 §3.2.2 and §3.2.4.
    fn dns_query(name: &str) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.extend_from_slice(&0xbeefu16.to_be_bytes()); // ID
        msg.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
        msg.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
        msg.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // AN/NS/AR counts
        for label in name.split('.') {
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.push(0);
        msg.extend_from_slice(&1u16.to_be_bytes()); // QTYPE A
        msg.extend_from_slice(&1u16.to_be_bytes()); // QCLASS IN
        msg
    }

    fn query_packet(name: &str) -> Vec<u8> {
        packet_to_port(name, dns::PORT_DNS)
    }

    fn packet_to_port(name: &str, port: u16) -> Vec<u8> {
        udp::build_ipv4_udp(CLIENT, RESOLVER, CLIENT_PORT, port, &dns_query(name)).unwrap()
    }

    fn shield() -> DnsShield {
        DnsShield::new(DomainFilter::from_rules(&[
            ("ads.example.com", FilterAction::Block),
            ("safe.example.com", FilterAction::Allow),
        ]))
    }

    #[test]
    fn blocked_name_is_answered_locally() {
        let packet = query_packet("ads.example.com");
        match shield().inspect(&packet) {
            DnsVerdict::Blocked { name, reply } => {
                assert_eq!(name, "ads.example.com");

                let framed = udp::parse_ipv4_udp(&reply).expect("reply is a valid datagram");
                assert_eq!(framed.src_ip, RESOLVER, "must come from the address asked");
                assert_eq!(framed.dst_ip, CLIENT);
                assert_eq!(framed.src_port, dns::PORT_DNS);
                assert_eq!(framed.dst_port, CLIENT_PORT);

                // ID echoed, QR set, RCODE 3 (NXDOMAIN) — RFC 1035 §4.1.1.
                assert_eq!(&framed.payload[0..2], &0xbeefu16.to_be_bytes());
                let flags = u16::from_be_bytes([framed.payload[2], framed.payload[3]]);
                assert_ne!(flags & 0x8000, 0, "QR");
                assert_eq!(flags & 0x000f, 3, "RCODE");
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn subdomain_of_a_blocked_name_is_blocked() {
        // DomainFilter stores labels TLD-first, so a rule covers its subtree.
        let packet = query_packet("cdn.ads.example.com");
        assert!(matches!(
            shield().inspect(&packet),
            DnsVerdict::Blocked { .. }
        ));
    }

    #[test]
    fn allowed_name_is_forwarded_verbatim() {
        let packet = query_packet("safe.example.com");
        match shield().inspect(&packet) {
            DnsVerdict::Forward { name, query } => {
                assert_eq!(name, "safe.example.com");
                assert_eq!(query, dns_query("safe.example.com"), "byte-identical query");
            }
            other => panic!("expected Forward, got {other:?}"),
        }
    }

    #[test]
    fn unknown_name_is_forwarded_rather_than_blocked() {
        // DomainFilter returns Proxy on a miss. Failing open here is the point:
        // a name we have no rule for must still resolve.
        let packet = query_packet("unheard-of.example.org");
        assert!(matches!(
            shield().inspect(&packet),
            DnsVerdict::Forward { .. }
        ));
    }

    #[test]
    fn traffic_that_is_not_a_dns_query_is_ignored() {
        assert_eq!(shield().inspect(&[]), DnsVerdict::Ignore, "empty");
        assert_eq!(
            shield().inspect(&[0xde, 0xad, 0xbe, 0xef]),
            DnsVerdict::Ignore,
            "garbage"
        );
        assert_eq!(
            shield().inspect(&packet_to_port("ads.example.com", 5353)),
            DnsVerdict::Ignore,
            "port 5353 is mDNS, not the resolver we route"
        );

        let mut not_a_query = query_packet("ads.example.com");
        // Flip QR on the DNS payload: byte 2 of the payload, which starts after
        // the 20-byte IPv4 header and the 8-byte UDP header.
        not_a_query[28 + 2] |= 0x80;
        assert_eq!(
            shield().inspect(&not_a_query),
            DnsVerdict::Ignore,
            "a response arriving on the TUN is not ours to answer"
        );
    }

    #[test]
    fn wrap_response_addresses_the_answer_back_to_the_client() {
        let packet = query_packet("safe.example.com");
        let answer = b"\xbe\xef\x81\x80 pretend answer";
        let framed = shield().wrap_response(&packet, answer).expect("wraps");

        let parsed = udp::parse_ipv4_udp(&framed).expect("valid datagram");
        assert_eq!(parsed.src_ip, RESOLVER);
        assert_eq!(parsed.dst_ip, CLIENT);
        assert_eq!(parsed.src_port, dns::PORT_DNS);
        assert_eq!(parsed.dst_port, CLIENT_PORT);
        assert_eq!(parsed.payload, answer);
    }

    #[test]
    fn wrap_response_declines_a_request_it_cannot_parse() {
        assert!(shield().wrap_response(&[0xde, 0xad], b"answer").is_none());
    }

    #[test]
    fn every_truncation_of_a_query_packet_is_handled_without_panicking() {
        let full = query_packet("ads.example.com");
        let shield = shield();
        for cut in 0..full.len() {
            assert_eq!(shield.inspect(&full[..cut]), DnsVerdict::Ignore, "cut {cut}");
        }
    }
}
