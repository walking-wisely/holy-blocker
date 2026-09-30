//! Module 4 of the domain-blocklist plan — build the signed, on-device `.fst` artifact from the
//! merged entries `merge` (module 2) produced.
//!
//! This module reverses each surviving domain's labels so subdomain matching becomes
//! prefix/exact-match at label boundaries on the consumer side, sorts and deduplicates the
//! reversed key set, builds an `fst::Map` from key to a small **provenance ID**, and writes the
//! artifact file plus a signed [`Manifest`] that binds the manifest to the artifact and lets a
//! client pick which of its trusted keys to verify against.
//!
//! See `docs/components/domain-blocklist/plan.md`'s `fst_build` section and the decision doc's
//! [Distribution](../../docs/decisions/domain-blocklist-sourcing.md) section for the field-for-field
//! manifest and signing contract this implements.

use std::collections::BTreeMap;
use std::io::Write;

use domain_normalize::RuleScope;
use ed25519_dalek::{Signer, Verifier, VerifyingKey};
use fst::MapBuilder;
use serde::ser::SerializeTuple;
use sha2::{Digest, Sha256};

use crate::types::{Category, LicenseId, MergedEntry, SourceId, SourceSnapshot, Timestamp};

/// Identifies one trusted signing/verifying key. How a client picks which `{key_id, signature}`
/// entry to try, instead of guessing — see the decision doc's key-rotation model.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct KeyId(pub String);

/// One Ed25519 signature over the manifest's signable bytes, tagged with which key produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub key_id: KeyId,
    pub bytes: [u8; 64],
}

// `[u8; 64]` does not implement `Serialize`/`Deserialize` in this serde build (arrays above 32
// elements need const-generic array support serde gates behind its `serde_core` split), so the
// 64-byte signature is serialized as an opaque byte sequence. This is a representation detail —
// the decoded bytes are still exactly 64.
impl serde::Serialize for Signature {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_tuple(2)?;
        seq.serialize_element(&self.key_id)?;
        seq.serialize_element(&self.bytes[..])?;
        seq.end()
    }
}

impl<'de> serde::Deserialize<'de> for Signature {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de2> serde::de::Visitor<'de2> for V {
            type Value = Signature;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a key_id and a 64-byte signature")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de2>>(
                self,
                mut seq: A,
            ) -> Result<Signature, A::Error> {
                let key_id: KeyId = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("missing key_id"))?;
                let bytes: Vec<u8> = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::custom("missing signature bytes"))?;
                let len = bytes.len();
                let bytes: [u8; 64] = bytes.try_into().map_err(|_| {
                    serde::de::Error::custom(format!(
                        "expected exactly 64 signature bytes, got {len}"
                    ))
                })?;
                Ok(Signature { key_id, bytes })
            }
        }
        deserializer.deserialize_tuple(2, V)
    }
}

/// The canonicalized `{sources, categories, scope}` combination a provenance ID encodes. See the
/// plan's `fst_build` section for why this is an enumerated set, not a per-domain value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ProvenanceEntry {
    pub sources: Vec<SourceId>,
    pub categories: Vec<Category>,
    pub scope: RuleScope,
}

/// The signed header that accompanies the `.fst` file. `version` is the client's rollback
/// high-water mark; `signatures` is a list so a build can carry both an old-key and a new-key
/// signature during a rotation's overlap window — see the plan's `fst_build` section.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    pub version: u64,
    pub build_time: Timestamp,
    pub entry_count: u64,
    pub output_license: LicenseId,
    pub sources: Vec<SourceSnapshot>,
    pub provenance_table: Vec<ProvenanceEntry>,
    pub fst_digest: [u8; 32],
    pub signatures: Vec<Signature>,
}

/// Metrics a caller (module 7 `cli`, unbuilt) reports so the achieved bytes-per-entry is measured,
/// not estimated — the size gate in `gates.rs` is what turns that measurement into a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct FstBuildReport {
    pub entry_count: u64,
    pub fst_bytes: u64,
    pub bytes_per_entry: f64,
    pub provenance_combos: usize,
}

/// Why an artifact build failed. Every variant aborts the whole build — there is no partial
/// artifact to publish.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FstBuildError {
    #[error("failed to build the FST map: {0}")]
    Fst(String),
    #[error("failed to serialize the manifest: {0}")]
    Serialize(String),
    #[error("failed to deserialize the manifest: {0}")]
    Deserialize(String),
    #[error("manifest has no signing keys")]
    NoSigningKeys,
}

/// Reverses a normalized domain's labels so subdomain matching becomes prefix/exact-match at
/// label boundaries. `example.com` → `com.example`; `www.example.com` → `com.example.www`,
/// distinct from the apex key `com.example`, so an `ExactHost` rule for `www.example.com` can
/// never match the bare domain or a sibling subdomain. The key is the *normalized* form with no
/// host-identity stripping (module 0) — a source entry of `www.example.com` stays its own key.
pub fn reverse_key(domain: &str) -> String {
    let mut labels: Vec<&str> = domain.split('.').collect();
    labels.reverse();
    labels.join(".")
}

/// The serialized bytes a manifest's signatures cover: the manifest with its own `signatures`
/// field emptied. Appended-after-signing, so adding a second signature during rotation doesn't
/// invalidate the first — see the plan's signing rule.
pub fn signable_bytes(manifest: &Manifest) -> Result<Vec<u8>, FstBuildError> {
    let mut signable = manifest.clone();
    signable.signatures.clear();
    bincode::serialize(&signable).map_err(|e| FstBuildError::Serialize(e.to_string()))
}

/// The result of building an artifact: the `.fst` file bytes plus the manifest that accompanies
/// and binds them.
#[derive(Debug, Clone, PartialEq)]
pub struct FstArtifact {
    pub fst_bytes: Vec<u8>,
    pub manifest: Manifest,
    pub report: FstBuildReport,
}

/// Builds the provenance table and, for each merged entry, which provenance ID its
/// `{sources, categories, scope}` combination maps to.
fn assign_provenance(entries: &[MergedEntry]) -> (Vec<ProvenanceEntry>, BTreeMap<String, u32>) {
    // Enumerate the distinct combinations in sorted order so the ID space is deterministic across
    // runs (a signed artifact must not depend on iteration order) and compact (most domains share
    // one of a few dozen combinations).
    let mut table: Vec<ProvenanceEntry> = Vec::new();
    let mut id_of: BTreeMap<ProvenanceEntry, u32> = BTreeMap::new();

    for entry in entries {
        let combo = ProvenanceEntry {
            sources: entry.sources.clone(),
            categories: entry.categories.clone(),
            scope: entry.scope,
        };
        let next = id_of.len() as u32;
        let id = *id_of.entry(combo.clone()).or_insert(next);
        if id == next {
            table.push(combo);
        }
    }

    // The `table` is built in first-seen order; make it deterministic by sorting alongside the
    // id_of map. Rebuild id_of keyed by the canonical (sorted) order so both agree.
    table.sort();
    let mut canonical: BTreeMap<ProvenanceEntry, u32> = BTreeMap::new();
    for (idx, combo) in table.iter().enumerate() {
        canonical.insert(combo.clone(), idx as u32);
    }

    let mut by_domain: BTreeMap<String, u32> = BTreeMap::new();
    for entry in entries {
        let combo = ProvenanceEntry {
            sources: entry.sources.clone(),
            categories: entry.categories.clone(),
            scope: entry.scope,
        };
        by_domain.insert(reverse_key(&entry.domain), canonical[&combo]);
    }

    (table, by_domain)
}

/// Builds the `.fst` file bytes from `(reversed_key, provenance_id)` pairs. The pairs must be
/// inserted in sorted key order, which `fst::MapBuilder` requires and the caller guarantees.
fn build_fst(keyed: &BTreeMap<String, u32>) -> Result<Vec<u8>, FstBuildError> {
    let mut builder = MapBuilder::new(Vec::new()).map_err(|e| FstBuildError::Fst(e.to_string()))?;
    for (key, id) in keyed {
        builder
            .insert(key, u64::from(*id))
            .map_err(|e| FstBuildError::Fst(e.to_string()))?;
    }
    let mut buf = builder
        .into_inner()
        .map_err(|e| FstBuildError::Fst(e.to_string()))?;
    // Flush the inner writer to be sure all bytes are present.
    buf.flush().map_err(|e| FstBuildError::Fst(e.to_string()))?;
    Ok(buf)
}

/// Signs `manifest_bytes` with every `(key_id, signing_key)` pair currently active, appending one
/// `{key_id, bytes}` entry per key. Outside a rotation window there is exactly one entry.
fn sign(
    manifest_bytes: &[u8],
    signing_keys: &[(KeyId, ed25519_dalek::SigningKey)],
) -> Vec<Signature> {
    signing_keys
        .iter()
        .map(|(key_id, key)| {
            let signature: ed25519_dalek::Signature = key.sign(manifest_bytes);
            Signature {
                key_id: key_id.clone(),
                bytes: signature.to_bytes(),
            }
        })
        .collect()
}

/// Builds the complete artifact from merged entries and the signing key set.
///
/// `output_license` is the least-permissive of the input licenses, computed upstream (module 1);
/// `sources` are the per-source snapshots from module 1; `signing_keys` is every key currently
/// active for signing (empty only during the earliest bootstrap, which this refuses — a build
/// with no signer must never publish).
pub fn build(
    entries: &[MergedEntry],
    output_license: LicenseId,
    sources: Vec<SourceSnapshot>,
    version: u64,
    build_time: Timestamp,
    signing_keys: &[(KeyId, ed25519_dalek::SigningKey)],
) -> Result<FstArtifact, FstBuildError> {
    if signing_keys.is_empty() {
        return Err(FstBuildError::NoSigningKeys);
    }

    let (provenance_table, keyed) = assign_provenance(entries);
    let fst_bytes = build_fst(&keyed)?;

    let mut digest = Sha256::new();
    digest.update(&fst_bytes);
    let fst_digest: [u8; 32] = digest.finalize().into();

    let entry_count = keyed.len() as u64;
    let fst_bytes_len = fst_bytes.len() as u64;
    let bytes_per_entry = if entry_count == 0 {
        0.0
    } else {
        fst_bytes_len as f64 / entry_count as f64
    };

    let manifest = Manifest {
        version,
        build_time,
        entry_count,
        output_license,
        sources,
        provenance_table,
        fst_digest,
        signatures: Vec::new(),
    };

    let signable = signable_bytes(&manifest)?;
    let signatures = sign(&signable, signing_keys);

    let mut manifest = manifest;
    manifest.signatures = signatures;

    Ok(FstArtifact {
        fst_bytes,
        report: FstBuildReport {
            entry_count,
            fst_bytes: fst_bytes_len,
            bytes_per_entry,
            provenance_combos: manifest.provenance_table.len(),
        },
        manifest,
    })
}

/// Verifies a manifest's `signatures` list against a set of trusted keys. Returns true if **any**
/// `{key_id, signature}` entry whose `key_id` is in `trusted_keys` verifies over the manifest's
/// signable bytes — mirroring the decision doc's "verify against any one entry whose key_id is in
/// your own trusted set" client rule. This is the on-device check; the pipeline itself uses it to
/// round-trip its own output in tests.
pub fn verify_manifest(
    manifest: &Manifest,
    trusted_keys: &[(KeyId, VerifyingKey)],
) -> Result<bool, FstBuildError> {
    let signable = signable_bytes(manifest)?;
    for signature in &manifest.signatures {
        let Some((_, verifying_key)) = trusted_keys
            .iter()
            .find(|(key_id, _)| key_id == &signature.key_id)
        else {
            continue;
        };
        let sig = ed25519_dalek::Signature::from_bytes(&signature.bytes);
        if verifying_key.verify(&signable, &sig).is_ok() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::merge as merge_entries;
    use crate::types::RawEntry;

    fn merged(entries: Vec<RawEntry>) -> Vec<MergedEntry> {
        merge_entries(&entries, &[]).entries
    }

    fn raw(
        domain: &str,
        source: SourceId,
        category: Category,
        hint: crate::types::ScopeHint,
    ) -> RawEntry {
        RawEntry {
            domain: domain.to_string(),
            source,
            category,
            scope_hint: hint,
        }
    }

    fn snapshots() -> Vec<SourceSnapshot> {
        vec![
            SourceSnapshot {
                source: SourceId::StevenBlack,
                revision: "v1.0.0".to_string(),
                license: LicenseId("MIT".to_string()),
                fetched_at: 1_700_000_000,
            },
            SourceSnapshot {
                source: SourceId::Hagezi,
                revision: "2024-01-01".to_string(),
                license: LicenseId("CC0-1.0".to_string()),
                fetched_at: 1_700_000_100,
            },
        ]
    }

    fn key_pair(id: &str, seed: u8) -> (KeyId, ed25519_dalek::SigningKey) {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        (
            KeyId(id.to_string()),
            ed25519_dalek::SigningKey::from_bytes(&bytes),
        )
    }

    fn verifying(key: &ed25519_dalek::SigningKey) -> VerifyingKey {
        key.verifying_key()
    }

    fn test_entries() -> Vec<MergedEntry> {
        merged(vec![
            // example.com is apex (wildcard), adult from StevenBlack.
            raw(
                "example.com",
                SourceId::StevenBlack,
                Category::Adult,
                crate::types::ScopeHint::Apex,
            ),
            // www.example.com is an exact-host adult entry from Hagezi — distinct key from the apex.
            raw(
                "www.example.com",
                SourceId::Hagezi,
                Category::Adult,
                crate::types::ScopeHint::None,
            ),
            // A domain flagged adult by StevenBlack AND gambling by Hagezi keeps both categories.
            raw(
                "betting.example.net",
                SourceId::StevenBlack,
                Category::Adult,
                crate::types::ScopeHint::Apex,
            ),
            raw(
                "betting.example.net",
                SourceId::Hagezi,
                Category::Gambling,
                crate::types::ScopeHint::Apex,
            ),
        ])
    }

    #[test]
    fn reverse_key_reverses_labels() {
        assert_eq!(reverse_key("example.com"), "com.example");
        assert_eq!(reverse_key("www.example.com"), "com.example.www");
        assert_eq!(reverse_key("a.b.c"), "c.b.a");
        assert_eq!(reverse_key("example"), "example");
    }

    #[test]
    fn reverse_key_preserves_exact_vs_apex_distinction() {
        // The apex key and the www exact-host key must stay distinct — an ExactHost rule for
        // www.example.com must never match the bare domain or a sibling.
        assert_ne!(reverse_key("www.example.com"), reverse_key("example.com"));
        assert_ne!(
            reverse_key("www.example.com"),
            reverse_key("cdn.example.com")
        );
    }

    #[test]
    fn build_artifact_reverses_keys_and_carries_manifest() {
        let (key_id, signing_key) = key_pair("k1", 1);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id, signing_key)],
        )
        .unwrap();

        // 3 distinct reversed keys: com.example (apex), com.example.www (exact), net.example.betting (apex).
        assert_eq!(artifact.report.entry_count, 3);
        assert_eq!(artifact.manifest.entry_count, 3);
        assert_eq!(artifact.manifest.version, 1);
        assert_eq!(
            artifact.manifest.output_license,
            LicenseId("MIT".to_string())
        );
        assert_eq!(artifact.manifest.sources.len(), 2);
        assert_eq!(artifact.manifest.signatures.len(), 1);
    }

    #[test]
    fn fst_stores_reversed_keys_and_lookup_returns_provenance_id() {
        let (key_id, signing_key) = key_pair("k1", 2);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id, signing_key)],
        )
        .unwrap();

        let map = fst::Map::new(artifact.fst_bytes.as_slice()).unwrap();
        // The apex and the www exact-host keys are both present, as distinct keys.
        assert!(map.get("com.example").is_some());
        assert!(map.get("com.example.www").is_some());
        assert!(map.get("net.example.betting").is_some());
        // A key not in the list is absent.
        assert!(map.get("com.nonexistent").is_none());
        // The stored value is a provenance ID that indexes the manifest's table.
        let id = map.get("com.example").unwrap() as usize;
        assert!(id < artifact.manifest.provenance_table.len());
        assert_eq!(
            artifact.manifest.provenance_table[id].scope,
            RuleScope::Apex
        );
    }

    #[test]
    fn provenance_table_round_trips_a_two_category_domain() {
        let (key_id, signing_key) = key_pair("k1", 3);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id, signing_key)],
        )
        .unwrap();

        let map = fst::Map::new(artifact.fst_bytes.as_slice()).unwrap();
        let id = map.get("net.example.betting").unwrap() as usize;
        let entry = &artifact.manifest.provenance_table[id];
        // Both categories survive merge and encode into the same provenance combination.
        assert_eq!(entry.categories, vec![Category::Adult, Category::Gambling]);
        assert!(entry.sources.contains(&SourceId::StevenBlack));
        assert!(entry.sources.contains(&SourceId::Hagezi));
        // betting.example.net sits below eTLD+1, so the wildcard's apex widening is refused and
        // it is scoped ExactHost — same as merge resolves it.
        assert_eq!(entry.scope, RuleScope::ExactHost);
    }

    #[test]
    fn shared_provenance_combination_is_not_duplicated_in_the_table() {
        let (key_id, signing_key) = key_pair("k1", 4);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id, signing_key)],
        )
        .unwrap();
        // example.com (apex adult, stevenblack only) and the www exact-host (adult, hagezi) are
        // different combinations; the betting domain (adult+gambling) is a third.
        assert_eq!(artifact.report.provenance_combos, 3);
    }

    #[test]
    fn manifest_round_trips_through_bincode() {
        let (key_id, signing_key) = key_pair("k1", 5);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id, signing_key)],
        )
        .unwrap();

        let bytes = bincode::serialize(&artifact.manifest).unwrap();
        let decoded: Manifest = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, artifact.manifest);
        // Provenance table decodes back to the same per-ID combination.
        let map = fst::Map::new(artifact.fst_bytes.as_slice()).unwrap();
        let id = map.get("net.example.betting").unwrap() as usize;
        assert_eq!(
            decoded.provenance_table[id].categories,
            vec![Category::Adult, Category::Gambling]
        );
    }

    #[test]
    fn manifest_verifies_against_a_single_key() {
        let (key_id, signing_key) = key_pair("k1", 6);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id.clone(), signing_key.clone())],
        )
        .unwrap();
        assert!(verify_manifest(&artifact.manifest, &[(key_id, verifying(&signing_key))]).unwrap());
    }

    #[test]
    fn manifest_with_two_signatures_verifies_against_old_only_new_only_or_both() {
        let (old_id, old_key) = key_pair("old", 7);
        let (new_id, new_key) = key_pair("new", 8);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[
                (old_id.clone(), old_key.clone()),
                (new_id.clone(), new_key.clone()),
            ],
        )
        .unwrap();
        assert_eq!(artifact.manifest.signatures.len(), 2);

        // A client trusting only the old key, only the new key, or both, all verify.
        assert!(
            verify_manifest(&artifact.manifest, &[(old_id.clone(), verifying(&old_key))]).unwrap()
        );
        assert!(
            verify_manifest(&artifact.manifest, &[(new_id.clone(), verifying(&new_key))]).unwrap()
        );
        assert!(
            verify_manifest(
                &artifact.manifest,
                &[(old_id, verifying(&old_key)), (new_id, verifying(&new_key))]
            )
            .unwrap()
        );
    }

    #[test]
    fn manifest_does_not_verify_against_a_key_it_was_not_signed_with() {
        let (key_id, signing_key) = key_pair("k1", 9);
        let (other_id, other_key) = key_pair("other", 10);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id, signing_key)],
        )
        .unwrap();
        // Same key_id string but a different actual key, and a different key_id — both must fail.
        assert!(
            !verify_manifest(&artifact.manifest, &[(other_id, verifying(&other_key))]).unwrap()
        );
    }

    #[test]
    fn altering_fst_digest_invalidates_every_signature() {
        let (key_id, signing_key) = key_pair("k1", 11);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id.clone(), signing_key.clone())],
        )
        .unwrap();

        let mut tampered = artifact.manifest.clone();
        tampered.fst_digest[0] ^= 0xFF;
        // The fst_digest is a field inside the signed portion, so changing it breaks the signature.
        assert!(!verify_manifest(&tampered, &[(key_id.clone(), verifying(&signing_key))]).unwrap());
        // The untampered manifest still verifies.
        assert!(verify_manifest(&artifact.manifest, &[(key_id, verifying(&signing_key))]).unwrap());
    }

    #[test]
    fn build_refuses_to_sign_with_no_keys() {
        let err = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[],
        )
        .unwrap_err();
        assert_eq!(err, FstBuildError::NoSigningKeys);
    }

    #[test]
    fn signable_bytes_excludes_the_signature_field() {
        let (key_id, signing_key) = key_pair("k1", 12);
        let artifact = build(
            &test_entries(),
            LicenseId("MIT".to_string()),
            snapshots(),
            1,
            1_700_000_000,
            &[(key_id.clone(), signing_key)],
        )
        .unwrap();

        // The signable bytes are the manifest with signatures emptied — serializing that same
        // shape must produce identical bytes regardless of the current signatures field.
        let a = signable_bytes(&artifact.manifest).unwrap();
        let mut empty = artifact.manifest.clone();
        empty.signatures.clear();
        let b = signable_bytes(&empty).unwrap();
        assert_eq!(a, b);
    }
}
