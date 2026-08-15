//! The on-device two-slot artifact layout — `<base>/current/{artifact.fst,manifest.bin}`,
//! mirrored under `<base>/previous/` — read on the pipeline's own "load the previous published
//! build" path and written on publish.
//!
//! `domain-blocklist` cannot depend on `net-shield` (the dependency direction runs the other way:
//! `net-shield` depends on `domain-blocklist` for `Manifest`/`reverse_key`/`verify_manifest`), so
//! this module mirrors the slot-layout constants **by value** rather than importing them — see
//! `packages/net-shield/src/blocklist.rs`'s `ARTIFACT_FILE`/`MANIFEST_FILE`/`current`/`previous`
//! constants, which this module's own constants below must keep matching. Module 7's own plan
//! section names this explicitly: "rotate the existing `current/` into `previous/` before writing
//! a new `current/` ... write both files atomically per slot, and track the high-water mark itself
//! ... rather than inventing an on-disk high-water-mark file format that `net-shield` never reads."

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use domain_blocklist::{FstArtifact, KeyId, Manifest, reverse_key, verify_manifest};
use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};

/// Must match `net-shield::blocklist::ARTIFACT_FILE`.
pub const ARTIFACT_FILE: &str = "artifact.fst";
/// Must match `net-shield::blocklist::MANIFEST_FILE`.
pub const MANIFEST_FILE: &str = "manifest.bin";
const CURRENT_SLOT: &str = "current";
const PREVIOUS_SLOT: &str = "previous";
const PREVIOUS_TMP_SLOT: &str = "previous.tmp";

fn slot_dir(base: &Path, slot: &str) -> PathBuf {
    base.join(slot)
}

/// What loading the previously published build found. Distinguished from a bare `Option` for the
/// same reason `gates::PreviousBuild` is: a caller that failed to load the previous manifest and a
/// caller correctly reporting a genuine first build must never collapse onto the same value, or
/// `shrinkage_gate`/`growth_gate` silently disable their percentage checks on the easiest mistake
/// to make.
#[derive(Debug)]
pub enum PreviousLoad {
    /// `<base>/current/` does not exist at all — a genuine first build.
    NoPreviousBuild,
    /// A previous build was found, verified (signature, then digest), and its key set streamed
    /// back out of the FST (by un-reversing each stored key with [`reverse_key`], which is its own
    /// inverse — the FST stores `reverse_key(domain)`, so reversing again recovers `domain`).
    Existing {
        manifest: Manifest,
        domains: BTreeSet<String>,
    },
}

/// Loads and verifies `<base>/current/`. **A previous build that exists but fails to verify is a
/// hard abort (`Err`), never a silent [`PreviousLoad::NoPreviousBuild`]** — collapsing "verification
/// failed" onto "nothing published yet" is exactly the ambiguity `gates::PreviousBuild` was built
/// to make impossible, and doing it here at the loader would reopen it one layer up.
pub fn load_previous(base: &Path, trusted_keys: &[(KeyId, VerifyingKey)]) -> anyhow::Result<PreviousLoad> {
    let dir = slot_dir(base, CURRENT_SLOT);
    let fst_path = dir.join(ARTIFACT_FILE);
    let manifest_path = dir.join(MANIFEST_FILE);

    if !fst_path.exists() && !manifest_path.exists() {
        return Ok(PreviousLoad::NoPreviousBuild);
    }

    let fst_bytes = std::fs::read(&fst_path)
        .map_err(|e| anyhow::anyhow!("previous build exists but {} unreadable: {e}", fst_path.display()))?;
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|e| {
        anyhow::anyhow!("previous build exists but {} unreadable: {e}", manifest_path.display())
    })?;
    let manifest: Manifest = bincode::deserialize(&manifest_bytes).map_err(|e| {
        anyhow::anyhow!("previous manifest at {} failed to deserialize: {e}", manifest_path.display())
    })?;

    let verified = verify_manifest(&manifest, trusted_keys)
        .map_err(|e| anyhow::anyhow!("previous manifest signature verification errored: {e}"))?;
    if !verified {
        anyhow::bail!(
            "previous manifest at {} does not verify against any trusted key — refusing to treat \
             this as either a valid previous build or a first build",
            manifest_path.display()
        );
    }

    let mut hasher = Sha256::new();
    hasher.update(&fst_bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    if digest != manifest.fst_digest {
        anyhow::bail!(
            "previous artifact at {} does not match its manifest's fst_digest — the slot is \
             corrupt or tampered",
            fst_path.display()
        );
    }

    let map = fst::Map::new(fst_bytes).map_err(|e| anyhow::anyhow!("previous artifact is not a valid FST: {e}"))?;
    let mut domains = BTreeSet::new();
    {
        use fst::Streamer;
        let mut stream = map.stream();
        while let Some((key, _value)) = stream.next() {
            let reversed = String::from_utf8_lossy(key).into_owned();
            domains.insert(reverse_key(&reversed));
        }
    }

    Ok(PreviousLoad::Existing { manifest, domains })
}

/// Publishes a newly built artifact: rotates the existing `current/` into `previous/` via
/// [`rotate_current_into_previous`] (so a rebuild never destroys the fallback slot the previous
/// build populated), then writes the new `current/` atomically per file (temp-sibling + rename).
/// Every write happening under a temp name first means a crash mid-publish can never leave a
/// half-written slot for `net-shield`'s loader to trip over — `BlocklistArtifact::load` already
/// falls back to `previous/` on any verification failure, but a torn write is worse than a
/// verification failure because it can crash the decoder outright rather than failing a clean
/// signature/digest check.
pub fn publish(base: &Path, artifact: &FstArtifact) -> anyhow::Result<()> {
    let current = slot_dir(base, CURRENT_SLOT);

    rotate_current_into_previous(base)?;

    std::fs::create_dir_all(&current)
        .map_err(|e| anyhow::anyhow!("failed to create {}: {e}", current.display()))?;
    let manifest_bytes = bincode::serialize(&artifact.manifest)
        .map_err(|e| anyhow::anyhow!("failed to serialize manifest for publish: {e}"))?;
    atomic_write(&current.join(ARTIFACT_FILE), &artifact.fst_bytes)?;
    atomic_write(&current.join(MANIFEST_FILE), &manifest_bytes)?;
    Ok(())
}

/// Moves the existing `current/` slot into `previous/` as a single directory-level operation
/// rather than two independent file renames.
///
/// The rotation this replaces moved `artifact.fst` then `manifest.bin` with two separate
/// `std::fs::rename` calls. An interruption between the two left `previous/` holding the fresh
/// (new-old) artifact file next to the stale (old-old) manifest file — a digest mismatch that
/// makes `net-shield`'s loader reject the whole slot, on top of `current/` already being
/// incomplete at that point. That is worse than the pair simply being briefly missing: a missing
/// pair fails closed cleanly (nothing to load), while a mismatched pair fails a digest check the
/// same way corruption would, so `previous/` no longer serves as a usable fallback.
///
/// Instead, this builds the new `previous/` contents in a fresh sibling directory
/// (`<base>/previous.tmp`) from the files currently in `current/`, then renames that ONE directory
/// into place as the last step, so the pair moves as a single filesystem operation.
///
/// This narrows but does not fully close the gap: an interruption between the
/// `remove_dir_all(&previous)` and the final `rename(&previous_tmp, &previous)` still leaves
/// `previous/` briefly absent. But that state is "both slots temporarily missing/incomplete,"
/// which `net-shield`'s loader already treats as a clean failure (nothing to load, not a corrupt
/// state to reject), and unlike today's bug, no interruption window ever produces a
/// plausible-but-wrong (mismatched-digest) `previous/`.
fn rotate_current_into_previous(base: &Path) -> anyhow::Result<()> {
    let current = slot_dir(base, CURRENT_SLOT);
    let previous = slot_dir(base, PREVIOUS_SLOT);
    let previous_tmp = base.join(PREVIOUS_TMP_SLOT);

    if !current.join(ARTIFACT_FILE).exists() && !current.join(MANIFEST_FILE).exists() {
        return Ok(());
    }

    // A stale `previous.tmp` from an earlier interrupted run must never be silently merged with
    // this run's rotation — clear it first (a "not found" is just as good as a successful clear).
    clear_dir(&previous_tmp)?;
    std::fs::create_dir_all(&previous_tmp)
        .map_err(|e| anyhow::anyhow!("failed to create {}: {e}", previous_tmp.display()))?;
    rotate_file(&current.join(ARTIFACT_FILE), &previous_tmp.join(ARTIFACT_FILE))?;
    rotate_file(&current.join(MANIFEST_FILE), &previous_tmp.join(MANIFEST_FILE))?;

    clear_dir(&previous)?;
    std::fs::rename(&previous_tmp, &previous).map_err(|e| {
        anyhow::anyhow!(
            "failed to rotate {} into {}: {e}",
            previous_tmp.display(),
            previous.display()
        )
    })
}

/// Removes `dir` recursively, treating a missing directory as already-removed and propagating every
/// other I/O error.
fn clear_dir(dir: &Path) -> anyhow::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!("failed to remove {}: {e}", dir.display())),
    }
}

/// Moves `from` to `to` if `from` exists; a missing `from` (e.g. a manifest present without its
/// artifact, from a previously interrupted publish) is not an error — `to` simply keeps whatever
/// was already there, or stays absent.
fn rotate_file(from: &Path, to: &Path) -> anyhow::Result<()> {
    if !from.exists() {
        return Ok(());
    }
    std::fs::rename(from, to)
        .map_err(|e| anyhow::anyhow!("failed to rotate {} to {}: {e}", from.display(), to.display()))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| anyhow::anyhow!("failed to write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| anyhow::anyhow!("failed to rename {} to {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain_blocklist::{Category, KeyId as DbKeyId, MergedEntry, SourceId, build};
    use domain_normalize::RuleScope;

    fn key_pair(seed: u8) -> (DbKeyId, ed25519_dalek::SigningKey) {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        (DbKeyId("k1".to_string()), ed25519_dalek::SigningKey::from_bytes(&bytes))
    }

    fn one_entry_artifact(version: u64, seed: u8) -> (FstArtifact, (DbKeyId, VerifyingKey)) {
        let (key_id, signing_key) = key_pair(seed);
        let entries = vec![MergedEntry {
            domain: "example.com".to_string(),
            scope: RuleScope::Apex,
            sources: vec![SourceId::StevenBlack],
            categories: vec![Category::Adult],
        }];
        let artifact = build(
            &entries,
            domain_blocklist::LicenseId("MIT".to_string()),
            vec![],
            version,
            1_700_000_000,
            &[(key_id.clone(), signing_key.clone())],
        )
        .unwrap();
        (artifact, (key_id, signing_key.verifying_key()))
    }

    #[test]
    fn a_missing_base_directory_loads_as_no_previous_build() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("blocklist-output");
        match load_previous(&base, &[]).unwrap() {
            PreviousLoad::NoPreviousBuild => {}
            PreviousLoad::Existing { .. } => panic!("expected no previous build"),
        }
    }

    #[test]
    fn publish_then_load_previous_round_trips_the_domain_set() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let (artifact, trusted) = one_entry_artifact(1, 1);

        publish(&base, &artifact).unwrap();

        match load_previous(&base, &[trusted]).unwrap() {
            PreviousLoad::Existing { manifest, domains } => {
                assert_eq!(manifest.version, 1);
                assert!(domains.contains("example.com"));
            }
            PreviousLoad::NoPreviousBuild => panic!("expected a previous build"),
        }
    }

    #[test]
    fn publishing_twice_rotates_the_first_build_into_previous() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let (artifact1, trusted) = one_entry_artifact(1, 2);
        publish(&base, &artifact1).unwrap();

        let (artifact2, _) = one_entry_artifact(2, 2);
        publish(&base, &artifact2).unwrap();

        assert!(base.join("previous").join(ARTIFACT_FILE).exists());
        assert!(base.join("previous").join(MANIFEST_FILE).exists());
        match load_previous(&base, &[trusted]).unwrap() {
            PreviousLoad::Existing { manifest, .. } => assert_eq!(manifest.version, 2),
            PreviousLoad::NoPreviousBuild => panic!("expected a previous build"),
        }
    }

    #[test]
    fn a_previous_build_that_fails_signature_verification_is_a_hard_error_not_no_previous_build() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let (artifact, _trusted) = one_entry_artifact(1, 3);
        publish(&base, &artifact).unwrap();

        // Trust a key the artifact was never signed with.
        let (_wrong_id, wrong_signing) = key_pair(99);
        let wrong_trusted = (DbKeyId("k1".to_string()), wrong_signing.verifying_key());

        let err = load_previous(&base, &[wrong_trusted]).unwrap_err();
        assert!(err.to_string().contains("does not verify"));
    }

    #[test]
    fn publishing_twice_leaves_an_internally_consistent_pair_in_previous() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let (artifact1, _trusted) = one_entry_artifact(1, 4);
        publish(&base, &artifact1).unwrap();

        let (artifact2, _) = one_entry_artifact(2, 4);
        publish(&base, &artifact2).unwrap();

        let prev_fst = std::fs::read(base.join("previous").join(ARTIFACT_FILE)).unwrap();
        let prev_manifest: Manifest = bincode::deserialize(
            &std::fs::read(base.join("previous").join(MANIFEST_FILE)).unwrap(),
        )
        .unwrap();

        // Mirrors `load_previous`'s own digest check: the artifact and manifest that moved into
        // `previous/` together must match each other, never a torn/mismatched pair.
        let mut hasher = Sha256::new();
        hasher.update(&prev_fst);
        let digest: [u8; 32] = hasher.finalize().into();
        assert_eq!(digest, prev_manifest.fst_digest);
        assert_eq!(prev_manifest.version, 1);
        assert!(!base.join("previous.tmp").exists());
    }

    #[test]
    fn rotation_moves_the_pair_together_and_clears_a_stale_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let (artifact, _trusted) = one_entry_artifact(1, 5);
        publish(&base, &artifact).unwrap();

        // Deterministic stand-in for an interrupted run: `previous/` already absent, a stale
        // `previous.tmp` leftover from a previous crash on disk, and a full `current/` ready to
        // rotate. Running the helper against this state must never merge or partially move — it
        // either fully reflects the rotation or leaves `previous/` cleanly absent.
        clear_dir(&base.join("previous")).unwrap();
        std::fs::create_dir_all(base.join("previous.tmp")).unwrap();
        std::fs::write(base.join("previous.tmp/planted.txt"), b"stale").unwrap();

        rotate_current_into_previous(&base).unwrap();

        let prev_fst = std::fs::read(base.join("previous").join(ARTIFACT_FILE)).unwrap();
        let prev_manifest: Manifest = bincode::deserialize(
            &std::fs::read(base.join("previous").join(MANIFEST_FILE)).unwrap(),
        )
        .unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&prev_fst);
        let digest: [u8; 32] = hasher.finalize().into();
        assert_eq!(digest, prev_manifest.fst_digest);

        assert!(
            !base.join("previous.tmp").exists(),
            "stale temp dir must be cleared, never merged into the rotation"
        );
    }
}
