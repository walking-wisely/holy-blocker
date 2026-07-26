//! UniFFI surface over `net-shield`'s DNS path.
//!
//! Exists for the same reason as `text-policy-ffi`: the Android VPN edge should
//! reach one filter rather than reimplement DNS and IPv4 wire formats in
//! Kotlin. Everything here is type translation across the boundary — no
//! decision is made in this crate.
//!
//! The Kotlin side owns the TUN file descriptor and the sockets; it reads a
//! packet, calls [`DnsGuard::inspect`], and does what the returned
//! [`DnsDecision`] says.

use std::sync::Arc;

use net_shield::{
    dns_shield::{DnsShield, DnsVerdict},
    radix::{DomainFilter, FilterAction},
};

uniffi::setup_scaffolding!();

/// What the caller should do with the packet it just read from the TUN.
///
/// Mirrors [`net_shield::DnsVerdict`]. Carried as an enum with fields so the
/// Kotlin binding is a sealed class and the payload cannot be read for the
/// wrong case.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum DnsDecision {
    /// Nothing to filter — discard the packet.
    Ignore,
    /// `name` is blocked. Write `reply` back to the TUN; nothing leaves the
    /// device.
    Blocked { name: String, reply: Vec<u8> },
    /// `name` is permitted. Send `query` to a real resolver over a protected
    /// socket, then pass the answer to [`DnsGuard::wrap_response`] along with
    /// the original packet.
    Forward { name: String, query: Vec<u8> },
}

impl From<DnsVerdict> for DnsDecision {
    fn from(value: DnsVerdict) -> Self {
        match value {
            DnsVerdict::Ignore => Self::Ignore,
            DnsVerdict::Blocked { name, reply } => Self::Blocked { name, reply },
            DnsVerdict::Forward { name, query } => Self::Forward { name, query },
        }
    }
}

/// Starter rule set, mirroring how `text-policy-ffi` ships a placeholder
/// dictionary.
///
/// These are RFC 2606 §2 reserved names and block nothing real. A blocklist of
/// actual hostnames is exactly the kind of artifact this repository does not
/// carry, so the shipped list is a placeholder and a real one is supplied at
/// runtime through [`DnsGuard::with_blocked_domains`].
///
/// A rule covers its whole subtree: `DomainFilter` stores labels TLD-first, so
/// blocking `blocked.example` also blocks `cdn.blocked.example`.
fn builtin_rules() -> DomainFilter {
    DomainFilter::from_rules(&[("blocked.example", FilterAction::Block)])
}

/// Handle held by the foreign caller for the lifetime of the VPN session.
///
/// Construction builds the domain trie, so build it once per session rather
/// than once per packet.
#[derive(uniffi::Object)]
pub struct DnsGuard {
    inner: DnsShield,
}

#[uniffi::export]
impl DnsGuard {
    /// Builds a guard over the built-in placeholder rules.
    #[uniffi::constructor]
    pub fn with_builtin_rules() -> Arc<Self> {
        Arc::new(Self {
            inner: DnsShield::new(builtin_rules()),
        })
    }

    /// Builds a guard that blocks each name in `domains` and everything under
    /// it. Any other name resolves normally.
    #[uniffi::constructor]
    pub fn with_blocked_domains(domains: Vec<String>) -> Arc<Self> {
        let rules: Vec<(&str, FilterAction)> = domains
            .iter()
            .map(|d| (d.as_str(), FilterAction::Block))
            .collect();
        Arc::new(Self {
            inner: DnsShield::new(DomainFilter::from_rules(&rules)),
        })
    }

    /// Classify one IPv4 packet read from the TUN. Never fails: anything it
    /// cannot make sense of comes back as [`DnsDecision::Ignore`].
    pub fn inspect(&self, packet: Vec<u8>) -> DnsDecision {
        self.inner.inspect(&packet).into()
    }

    /// Frame a resolver's answer as a packet addressed back to the client that
    /// sent `request_packet` — the same bytes that produced the
    /// [`DnsDecision::Forward`].
    ///
    /// `None` if `request_packet` is not the packet it came from.
    pub fn wrap_response(&self, request_packet: Vec<u8>, response: Vec<u8>) -> Option<Vec<u8>> {
        self.inner.wrap_response(&request_packet, &response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use net_shield::{dns, udp};
    use std::net::Ipv4Addr;

    const CLIENT: Ipv4Addr = Ipv4Addr::new(10, 111, 0, 2);
    const RESOLVER: Ipv4Addr = Ipv4Addr::new(10, 111, 0, 1);

    /// A standard IN/A query — RFC 1035 §4.1.1 header, §4.1.2 question.
    fn query_packet(name: &str) -> Vec<u8> {
        let mut msg = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.split('.') {
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.extend_from_slice(&[0, 0, 1, 0, 1]); // root, QTYPE A, QCLASS IN
        udp::build_ipv4_udp(CLIENT, RESOLVER, 41234, dns::PORT_DNS, &msg).unwrap()
    }

    #[test]
    fn builtin_rules_block_the_placeholder_name_and_its_subtree() {
        let guard = DnsGuard::with_builtin_rules();
        for name in ["blocked.example", "cdn.blocked.example"] {
            assert!(
                matches!(guard.inspect(query_packet(name)), DnsDecision::Blocked { .. }),
                "{name}"
            );
        }
    }

    #[test]
    fn a_name_with_no_rule_is_forwarded() {
        let guard = DnsGuard::with_builtin_rules();
        match guard.inspect(query_packet("allowed.example")) {
            DnsDecision::Forward { name, query } => {
                assert_eq!(name, "allowed.example");
                assert!(!query.is_empty());
            }
            other => panic!("expected Forward, got {other:?}"),
        }
    }

    #[test]
    fn runtime_rules_replace_the_builtin_ones() {
        let guard = DnsGuard::with_blocked_domains(vec!["ads.example".into()]);
        assert!(matches!(
            guard.inspect(query_packet("ads.example")),
            DnsDecision::Blocked { .. }
        ));
        assert!(
            matches!(
                guard.inspect(query_packet("blocked.example")),
                DnsDecision::Forward { .. }
            ),
            "the built-in placeholder must not survive an explicit rule set"
        );
    }

    #[test]
    fn an_empty_rule_set_blocks_nothing() {
        let guard = DnsGuard::with_blocked_domains(vec![]);
        assert!(matches!(
            guard.inspect(query_packet("blocked.example")),
            DnsDecision::Forward { .. }
        ));
    }

    #[test]
    fn unparseable_input_is_ignored_rather_than_erroring() {
        // The edge calls this once per packet on a hot path; an exception per
        // stray packet would be the wrong shape entirely.
        let guard = DnsGuard::with_builtin_rules();
        assert_eq!(guard.inspect(vec![]), DnsDecision::Ignore);
        assert_eq!(guard.inspect(vec![0xde, 0xad, 0xbe, 0xef]), DnsDecision::Ignore);
    }

    #[test]
    fn wrap_response_round_trips_through_the_boundary() {
        let guard = DnsGuard::with_builtin_rules();
        let request = query_packet("allowed.example");
        let answer = b"\x12\x34\x81\x80 answer".to_vec();

        let framed = guard
            .wrap_response(request, answer.clone())
            .expect("wraps");
        let parsed = udp::parse_ipv4_udp(&framed).expect("valid datagram");
        assert_eq!(parsed.dst_ip, CLIENT);
        assert_eq!(parsed.payload, answer.as_slice());
    }

    #[test]
    fn wrap_response_returns_none_for_a_packet_it_cannot_parse() {
        let guard = DnsGuard::with_builtin_rules();
        assert!(guard.wrap_response(vec![0xde, 0xad], vec![1, 2, 3]).is_none());
    }

    #[test]
    fn decision_maps_from_every_verdict_variant() {
        // Guards against a variant being added on one side only.
        let cases = [
            (DnsVerdict::Ignore, DnsDecision::Ignore),
            (
                DnsVerdict::Blocked {
                    name: "a".into(),
                    reply: vec![1],
                },
                DnsDecision::Blocked {
                    name: "a".into(),
                    reply: vec![1],
                },
            ),
            (
                DnsVerdict::Forward {
                    name: "b".into(),
                    query: vec![2],
                },
                DnsDecision::Forward {
                    name: "b".into(),
                    query: vec![2],
                },
            ),
        ];
        for (verdict, decision) in cases {
            assert_eq!(DnsDecision::from(verdict), decision);
        }
    }
}
