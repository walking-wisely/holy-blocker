use std::net::IpAddr;
use std::path::Path;

use domain_blocklist::{KeyId, reverse_key};
use ed25519_dalek::VerifyingKey;
use net_shield::blocklist::{ArtifactError, BlocklistArtifact};
use net_shield::radix::FilterAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostVerdict {
    Allow,
    Block,
}

pub struct HostBlocklist {
    artifact: BlocklistArtifact,
}

impl HostBlocklist {
    pub fn load(dir: &Path, trusted_keys: &[(KeyId, VerifyingKey)]) -> Result<Self, ArtifactError> {
        BlocklistArtifact::load(dir, trusted_keys).map(|artifact| Self { artifact })
    }

    pub fn version(&self) -> u64 {
        self.artifact.version()
    }

    /// `host` is a URI host as hyper reports it: IPv6 literals keep their brackets and a name keeps
    /// its case and trailing dot. A name that cannot be normalized cannot be compared with the
    /// list, so it is refused rather than allowed.
    pub fn verdict(&self, host: &str) -> HostVerdict {
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if bare.parse::<IpAddr>().is_ok() {
            return HostVerdict::Allow;
        }
        let Ok(normalized) = domain_normalize::normalize(host) else {
            return HostVerdict::Block;
        };
        match self.artifact.lookup(&reverse_key(&normalized)) {
            Some(FilterAction::Block) => HostVerdict::Block,
            _ => HostVerdict::Allow,
        }
    }
}

pub fn parse_trusted_key(spec: &str) -> Result<(KeyId, VerifyingKey), String> {
    let (id, hex) = spec
        .split_once(':')
        .ok_or_else(|| format!("expected <key-id>:<64 hex digits>, got {spec:?}"))?;
    if id.is_empty() {
        return Err("key id is empty".to_string());
    }
    let bytes = decode_hex(hex).ok_or_else(|| format!("key for {id:?} is not hexadecimal"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| format!("key for {id:?} is not 32 bytes"))?;
    let key = VerifyingKey::from_bytes(&bytes)
        .map_err(|_| format!("key for {id:?} is not a valid Ed25519 public key"))?;
    Ok((KeyId(id.to_string()), key))
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain_blocklist::{Category, LicenseId, MergedEntry, SourceId};
    use domain_normalize::RuleScope;
    use ed25519_dalek::SigningKey;
    use net_shield::blocklist::{ARTIFACT_FILE, MANIFEST_FILE};
    use tempfile::TempDir;

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn entry(domain: &str, scope: RuleScope) -> MergedEntry {
        MergedEntry {
            domain: domain.to_string(),
            scope,
            sources: vec![SourceId::StevenBlack],
            categories: vec![Category::Adult],
        }
    }

    fn write_current(dir: &Path, entries: &[MergedEntry]) {
        let artifact = domain_blocklist::build(
            entries,
            LicenseId("MIT".to_string()),
            Vec::new(),
            1,
            1_700_000_000,
            &[(KeyId("k1".to_string()), signing_key())],
        )
        .unwrap();
        let slot = dir.join("current");
        std::fs::create_dir_all(&slot).unwrap();
        std::fs::write(slot.join(ARTIFACT_FILE), &artifact.fst_bytes).unwrap();
        std::fs::write(slot.join(MANIFEST_FILE), bincode::serialize(&artifact.manifest).unwrap())
            .unwrap();
    }

    fn blocklist(entries: &[MergedEntry]) -> (TempDir, HostBlocklist) {
        let dir = tempfile::tempdir().unwrap();
        write_current(dir.path(), entries);
        let keys = [(KeyId("k1".to_string()), signing_key().verifying_key())];
        let list = HostBlocklist::load(dir.path(), &keys).unwrap();
        (dir, list)
    }

    fn listed() -> (TempDir, HostBlocklist) {
        blocklist(&[
            entry("blocked.example", RuleScope::Apex),
            entry("only.exact.example", RuleScope::ExactHost),
        ])
    }

    #[test]
    fn a_listed_apex_and_its_subdomains_are_blocked() {
        let (_dir, list) = listed();
        for host in ["blocked.example", "www.blocked.example", "a.b.blocked.example"] {
            assert_eq!(list.verdict(host), HostVerdict::Block, "{host}");
        }
    }

    #[test]
    fn an_unlisted_host_is_allowed() {
        let (_dir, list) = listed();
        for host in ["fine.example", "notblocked.example", "blocked.example.fine.example"] {
            assert_eq!(list.verdict(host), HostVerdict::Allow, "{host}");
        }
    }

    #[test]
    fn an_exact_host_entry_does_not_cover_its_siblings() {
        let (_dir, list) = listed();
        assert_eq!(list.verdict("only.exact.example"), HostVerdict::Block);
        assert_eq!(list.verdict("exact.example"), HostVerdict::Allow);
        assert_eq!(list.verdict("other.exact.example"), HostVerdict::Allow);
    }

    #[test]
    fn case_and_a_trailing_dot_do_not_evade_the_list() {
        let (_dir, list) = listed();
        assert_eq!(list.verdict("BLOCKED.Example"), HostVerdict::Block);
        assert_eq!(list.verdict("blocked.example."), HostVerdict::Block);
    }

    #[test]
    fn a_unicode_spelling_matches_its_punycode_entry() {
        let (_dir, list) = blocklist(&[entry("xn--e1afmkfd.example", RuleScope::Apex)]);
        assert_eq!(list.verdict("пример.example"), HostVerdict::Block);
    }

    #[test]
    fn ip_literals_are_allowed() {
        let (_dir, list) = listed();
        for host in ["127.0.0.1", "[::1]", "[2001:db8::1]"] {
            assert_eq!(list.verdict(host), HostVerdict::Allow, "{host}");
        }
    }

    #[test]
    fn a_name_that_cannot_be_normalized_is_blocked() {
        let (_dir, list) = listed();
        for host in ["blocked.example..", "bad..label.example", "", "."] {
            assert_eq!(list.verdict(host), HostVerdict::Block, "{host:?}");
        }
    }

    #[test]
    fn loading_fails_when_the_artifact_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let keys = [(KeyId("k1".to_string()), signing_key().verifying_key())];
        assert!(HostBlocklist::load(dir.path(), &keys).is_err());
    }

    #[test]
    fn loading_fails_when_signed_by_an_untrusted_key() {
        let dir = tempfile::tempdir().unwrap();
        write_current(dir.path(), &[entry("blocked.example", RuleScope::Apex)]);
        let other = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        assert!(HostBlocklist::load(dir.path(), &[(KeyId("k1".to_string()), other)]).is_err());
    }

    #[test]
    fn a_trusted_key_spec_parses() {
        let key = signing_key().verifying_key();
        let hex: String = key.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        let (id, parsed) = parse_trusted_key(&format!("k1:{hex}")).unwrap();
        assert_eq!(id, KeyId("k1".to_string()));
        assert_eq!(parsed, key);
    }

    #[test]
    fn malformed_trusted_key_specs_are_rejected() {
        for spec in ["", "k1", ":00", "k1:zz", "k1:abc", "k1:00ff", &format!("k1:{}", "00".repeat(33))] {
            assert!(parse_trusted_key(spec).is_err(), "{spec:?}");
        }
    }
}
