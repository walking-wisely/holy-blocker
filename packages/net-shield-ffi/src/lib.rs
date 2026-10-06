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

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use domain_blocklist::KeyId;
use ed25519_dalek::VerifyingKey;
use net_shield::{
    Allowlist, ArtifactError, BlocklistArtifact, BlocklistLookup,
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

const LOOKUP_BUDGET: Duration = Duration::from_millis(2);
const LOOKUP_CACHE_CAPACITY: usize = 4096;
const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// An Ed25519 public key the caller trusts to sign blocklist artifacts.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TrustedKey {
    pub key_id: String,
    pub public_key: Vec<u8>,
}

/// Why no guard was built from an artifact. There is never a guard over a
/// partially trusted list: the caller records the failure and decides what
/// protection to run without it.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum ArtifactLoadError {
    #[error("no artifact found")]
    Missing,
    #[error("a trusted key is not a 32-byte Ed25519 public key")]
    BadTrustedKey,
    #[error("artifact rejected: {reason}")]
    Rejected { reason: String },
}

impl From<ArtifactError> for ArtifactLoadError {
    fn from(value: ArtifactError) -> Self {
        match value {
            ArtifactError::SlotMissing => Self::Missing,
            other => Self::Rejected {
                reason: other.to_string(),
            },
        }
    }
}

fn verifying_keys(keys: Vec<TrustedKey>) -> Result<Vec<(KeyId, VerifyingKey)>, ArtifactLoadError> {
    keys.into_iter()
        .map(|key| {
            let bytes: [u8; ED25519_PUBLIC_KEY_LEN] = key
                .public_key
                .try_into()
                .map_err(|_| ArtifactLoadError::BadTrustedKey)?;
            let verifying =
                VerifyingKey::from_bytes(&bytes).map_err(|_| ArtifactLoadError::BadTrustedKey)?;
            Ok((KeyId(key.key_id), verifying))
        })
        .collect()
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

/// The form of `raw` a keyword rule can use, or `None` when it is too short or on the stoplist.
#[uniffi::export]
pub fn normalize_keyword_token(raw: String) -> Option<String> {
    net_shield::normalize_token(&raw)
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

    /// Builds a guard over the signed blocklist artifact under `artifact_dir`
    /// (`current/` then `previous/`), verified against `trusted_keys`.
    #[uniffi::constructor]
    pub fn with_artifact(
        artifact_dir: String,
        trusted_keys: Vec<TrustedKey>,
    ) -> Result<Arc<Self>, ArtifactLoadError> {
        let keys = verifying_keys(trusted_keys)?;
        let artifact = BlocklistArtifact::load(Path::new(&artifact_dir), &keys)?;
        let lookup = BlocklistLookup::new(Arc::new(artifact), LOOKUP_BUDGET, LOOKUP_CACHE_CAPACITY);
        Ok(Arc::new(Self {
            inner: DnsShield::with_policy(
                DomainFilter::from_rules(&[]),
                Allowlist::new(),
                Some(lookup),
            ),
        }))
    }

    /// Replaces the confirmed keyword tokens. A name whose label equals or contains a token is
    /// blocked unless an allowlist or explicit rule exempts it. Tokens that
    /// [`normalize_keyword_token`] refuses are ignored.
    pub fn set_keywords(&self, tokens: Vec<String>) {
        self.inner.set_keywords(&tokens);
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
                matches!(
                    guard.inspect(query_packet(name)),
                    DnsDecision::Blocked { .. }
                ),
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
    fn keywords_set_through_the_boundary_block_matching_names_only() {
        let guard = DnsGuard::with_blocked_domains(vec![]);
        guard.set_keywords(vec!["Instagram".into(), "meta".into()]);
        for name in ["instagram.com", "cdninstagram.com"] {
            assert!(
                matches!(guard.inspect(query_packet(name)), DnsDecision::Blocked { .. }),
                "{name}"
            );
        }
        for name in ["meta.com", "example.com"] {
            assert!(
                matches!(guard.inspect(query_packet(name)), DnsDecision::Forward { .. }),
                "{name}"
            );
        }
        guard.set_keywords(vec![]);
        assert!(matches!(
            guard.inspect(query_packet("instagram.com")),
            DnsDecision::Forward { .. }
        ));
    }

    #[test]
    fn token_normalization_is_exposed_for_the_panel() {
        assert_eq!(normalize_keyword_token("Insta-Gram".into()).as_deref(), Some("instagram"));
        assert_eq!(normalize_keyword_token("meta".into()), None);
        assert_eq!(normalize_keyword_token("Photos".into()), None);
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
        assert_eq!(
            guard.inspect(vec![0xde, 0xad, 0xbe, 0xef]),
            DnsDecision::Ignore
        );
    }

    #[test]
    fn wrap_response_round_trips_through_the_boundary() {
        let guard = DnsGuard::with_builtin_rules();
        let request = query_packet("allowed.example");
        let answer = b"\x12\x34\x81\x80 answer".to_vec();

        let framed = guard.wrap_response(request, answer.clone()).expect("wraps");
        let parsed = udp::parse_ipv4_udp(&framed).expect("valid datagram");
        assert_eq!(parsed.dst_ip, CLIENT);
        assert_eq!(parsed.payload, answer.as_slice());
    }

    #[test]
    fn wrap_response_returns_none_for_a_packet_it_cannot_parse() {
        let guard = DnsGuard::with_builtin_rules();
        assert!(
            guard
                .wrap_response(vec![0xde, 0xad], vec![1, 2, 3])
                .is_none()
        );
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

    mod artifact {
        use super::*;
        use domain_blocklist::{Category, KeyId, LicenseId, MergedEntry, SourceId};
        use domain_normalize::RuleScope;
        use net_shield::blocklist::{ARTIFACT_FILE, MANIFEST_FILE};
        use std::path::Path;
        use tempfile::tempdir;

        const KEY_ID: &str = "release-1";

        fn signing_key(seed: u8) -> ed25519_dalek::SigningKey {
            ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
        }

        fn trusted(seed: u8) -> Vec<TrustedKey> {
            vec![TrustedKey {
                key_id: KEY_ID.into(),
                public_key: signing_key(seed).verifying_key().to_bytes().to_vec(),
            }]
        }

        fn write_slot(base: &Path, slot: &str, domains: &[&str], seed: u8) {
            let entries: Vec<MergedEntry> = domains
                .iter()
                .map(|d| MergedEntry {
                    domain: d.to_string(),
                    scope: RuleScope::Apex,
                    sources: vec![SourceId::StevenBlack],
                    categories: vec![Category::Adult],
                })
                .collect();
            let built = domain_blocklist::build(
                &entries,
                LicenseId("MIT".into()),
                Vec::new(),
                1,
                1_700_000_000,
                &[(KeyId(KEY_ID.into()), signing_key(seed))],
            )
            .unwrap();
            let dir = base.join(slot);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(ARTIFACT_FILE), &built.fst_bytes).unwrap();
            std::fs::write(
                dir.join(MANIFEST_FILE),
                bincode::serialize(&built.manifest).unwrap(),
            )
            .unwrap();
        }

        fn path(dir: &tempfile::TempDir) -> String {
            dir.path().to_str().unwrap().to_string()
        }

        #[test]
        fn a_producer_artifact_blocks_its_names_and_subtree_only() {
            let dir = tempdir().unwrap();
            write_slot(dir.path(), "current", &["listed.example"], 7);
            let guard = DnsGuard::with_artifact(path(&dir), trusted(7)).unwrap();
            for name in ["listed.example", "cdn.listed.example"] {
                assert!(
                    matches!(
                        guard.inspect(query_packet(name)),
                        DnsDecision::Blocked { .. }
                    ),
                    "{name}"
                );
            }
            assert!(matches!(
                guard.inspect(query_packet("other.example")),
                DnsDecision::Forward { .. }
            ));
        }

        #[test]
        fn the_artifact_replaces_the_placeholder_rules() {
            let dir = tempdir().unwrap();
            write_slot(dir.path(), "current", &["listed.example"], 7);
            let guard = DnsGuard::with_artifact(path(&dir), trusted(7)).unwrap();
            assert!(matches!(
                guard.inspect(query_packet("blocked.example")),
                DnsDecision::Forward { .. }
            ));
        }

        #[test]
        fn an_artifact_signed_by_an_untrusted_key_is_refused() {
            let dir = tempdir().unwrap();
            write_slot(dir.path(), "current", &["listed.example"], 7);
            let err = DnsGuard::with_artifact(path(&dir), trusted(8))
                .err()
                .unwrap();
            assert!(matches!(err, ArtifactLoadError::Rejected { .. }), "{err:?}");
        }

        #[test]
        fn a_missing_artifact_is_an_error_not_an_empty_list() {
            let dir = tempdir().unwrap();
            let err = DnsGuard::with_artifact(path(&dir), trusted(7))
                .err()
                .unwrap();
            assert!(matches!(err, ArtifactLoadError::Missing), "{err:?}");
        }

        #[test]
        fn a_corrupt_current_slot_falls_back_to_previous() {
            let dir = tempdir().unwrap();
            write_slot(dir.path(), "previous", &["old.example"], 7);
            write_slot(dir.path(), "current", &["new.example"], 7);
            std::fs::write(dir.path().join("current").join(ARTIFACT_FILE), b"corrupt").unwrap();
            let guard = DnsGuard::with_artifact(path(&dir), trusted(7)).unwrap();
            assert!(matches!(
                guard.inspect(query_packet("old.example")),
                DnsDecision::Blocked { .. }
            ));
        }

        #[test]
        fn a_malformed_trusted_key_is_refused() {
            let dir = tempdir().unwrap();
            write_slot(dir.path(), "current", &["listed.example"], 7);
            let keys = vec![TrustedKey {
                key_id: KEY_ID.into(),
                public_key: vec![1, 2, 3],
            }];
            let err = DnsGuard::with_artifact(path(&dir), keys).err().unwrap();
            assert!(matches!(err, ArtifactLoadError::BadTrustedKey), "{err:?}");
        }

        #[test]
        fn no_trusted_keys_refuses_every_artifact() {
            let dir = tempdir().unwrap();
            write_slot(dir.path(), "current", &["listed.example"], 7);
            let err = DnsGuard::with_artifact(path(&dir), vec![]).err().unwrap();
            assert!(matches!(err, ArtifactLoadError::Rejected { .. }), "{err:?}");
        }
    }
}
