//! Persistent storage for `liveness`'s [`CacheEntry`] table — the "named, persistent,
//! non-repository location" module 3's plan section requires, owned by `cli` since `liveness/`
//! itself deliberately does no I/O (see that module's doc comment).
//!
//! `CacheEntry`/`Verdict`/`UnknownReason` (in `liveness::cache`/`liveness::lookup`) carry no serde
//! derives — adding them would mean editing an already-shipped pure module's file for a concern
//! that belongs to the caller, so this file instead defines its own bincode-serializable DTOs and
//! pure, exhaustively-matched conversions to and from the real types. A conversion that forgot a
//! variant would be a compile error (every match here is exhaustive, no wildcard arm), which is
//! the property that makes duplicating the enum shape here safe rather than a drift risk.

use std::collections::HashMap;
use std::path::Path;

use domain_blocklist::{CacheEntry, UnknownReason, Verdict};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum UnknownReasonDto {
    NoData,
    ServFail,
    Refused,
    FormErr,
    NotImp,
    Timeout,
    Malformed,
    ChainToDeadTarget,
    FilteredByResolver,
    UncorroboratedDead,
    /// Mirrors `liveness::UnknownReason::Transport`, added when the DNS client gained a
    /// dedicated network-level-failure reason distinct from `Malformed` (an unparseable response
    /// that *arrived*, vs. a connection reset/unreachable network — see that variant's own doc
    /// comment in `liveness/lookup.rs`).
    Transport,
}

impl From<UnknownReason> for UnknownReasonDto {
    fn from(r: UnknownReason) -> Self {
        match r {
            UnknownReason::NoData => Self::NoData,
            UnknownReason::ServFail => Self::ServFail,
            UnknownReason::Refused => Self::Refused,
            UnknownReason::FormErr => Self::FormErr,
            UnknownReason::NotImp => Self::NotImp,
            UnknownReason::Timeout => Self::Timeout,
            UnknownReason::Malformed => Self::Malformed,
            UnknownReason::ChainToDeadTarget => Self::ChainToDeadTarget,
            UnknownReason::FilteredByResolver => Self::FilteredByResolver,
            UnknownReason::UncorroboratedDead => Self::UncorroboratedDead,
            UnknownReason::Transport => Self::Transport,
        }
    }
}

impl From<UnknownReasonDto> for UnknownReason {
    fn from(r: UnknownReasonDto) -> Self {
        match r {
            UnknownReasonDto::NoData => Self::NoData,
            UnknownReasonDto::ServFail => Self::ServFail,
            UnknownReasonDto::Refused => Self::Refused,
            UnknownReasonDto::FormErr => Self::FormErr,
            UnknownReasonDto::NotImp => Self::NotImp,
            UnknownReasonDto::Timeout => Self::Timeout,
            UnknownReasonDto::Malformed => Self::Malformed,
            UnknownReasonDto::ChainToDeadTarget => Self::ChainToDeadTarget,
            UnknownReasonDto::FilteredByResolver => Self::FilteredByResolver,
            UnknownReasonDto::UncorroboratedDead => Self::UncorroboratedDead,
            UnknownReasonDto::Transport => Self::Transport,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum VerdictDto {
    Alive,
    Dead,
    Unknown(UnknownReasonDto),
}

impl From<Verdict> for VerdictDto {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Alive => Self::Alive,
            Verdict::Dead => Self::Dead,
            Verdict::Unknown(r) => Self::Unknown(r.into()),
        }
    }
}

impl From<VerdictDto> for Verdict {
    fn from(v: VerdictDto) -> Self {
        match v {
            VerdictDto::Alive => Self::Alive,
            VerdictDto::Dead => Self::Dead,
            VerdictDto::Unknown(r) => Self::Unknown(r.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CacheEntryDto {
    last_checked: u64,
    verdict: VerdictDto,
    first_dead_at: Option<u64>,
}

impl From<CacheEntry> for CacheEntryDto {
    fn from(e: CacheEntry) -> Self {
        Self {
            last_checked: e.last_checked,
            verdict: e.verdict.into(),
            first_dead_at: e.first_dead_at,
        }
    }
}

impl From<CacheEntryDto> for CacheEntry {
    fn from(e: CacheEntryDto) -> Self {
        Self {
            last_checked: e.last_checked,
            verdict: e.verdict.into(),
            first_dead_at: e.first_dead_at,
        }
    }
}

/// Loads the persistent liveness cache from `path`. Per the plan, "a run that cannot load the
/// cache starts cold" — a missing file is an empty cache, not an error; only a file that exists
/// but fails to deserialize is treated as a real problem (a corrupt cache silently read as empty
/// would quietly throw away months of accumulated liveness state).
pub fn load(path: &Path) -> anyhow::Result<HashMap<String, CacheEntry>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let bytes = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("failed to read liveness cache {}: {e}", path.display()))?;
    let dto: HashMap<String, CacheEntryDto> = bincode::deserialize(&bytes).map_err(|e| {
        anyhow::anyhow!(
            "liveness cache at {} is not valid bincode — refusing to silently start cold: {e}",
            path.display()
        )
    })?;
    Ok(dto.into_iter().map(|(k, v)| (k, v.into())).collect())
}

/// Writes the cache back to `path`, atomically (temp file + rename) so a crash mid-write can never
/// leave a half-written cache file for the next run to trip over. Per the plan, "a run that cannot
/// write it back fails loudly rather than discarding months of accumulated state."
pub fn save(path: &Path, cache: &HashMap<String, CacheEntry>) -> anyhow::Result<()> {
    let dto: HashMap<String, CacheEntryDto> = cache
        .iter()
        .map(|(k, v)| (k.clone(), (*v).into()))
        .collect();
    let bytes = bincode::serialize(&dto)
        .map_err(|e| anyhow::anyhow!("failed to serialize liveness cache: {e}"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| anyhow::anyhow!("failed to rename {} to {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_verdict_and_unknown_reason_variant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.bin");

        let reasons = [
            UnknownReason::NoData,
            UnknownReason::ServFail,
            UnknownReason::Refused,
            UnknownReason::FormErr,
            UnknownReason::NotImp,
            UnknownReason::Timeout,
            UnknownReason::Malformed,
            UnknownReason::ChainToDeadTarget,
            UnknownReason::FilteredByResolver,
            UnknownReason::UncorroboratedDead,
            UnknownReason::Transport,
        ];

        let mut cache = HashMap::new();
        cache.insert(
            "alive.example".to_string(),
            CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Alive,
                first_dead_at: None,
            },
        );
        cache.insert(
            "dead.example".to_string(),
            CacheEntry {
                last_checked: 2_000,
                verdict: Verdict::Dead,
                first_dead_at: Some(1_500),
            },
        );
        for (i, reason) in reasons.into_iter().enumerate() {
            cache.insert(
                format!("unknown{i}.example"),
                CacheEntry {
                    last_checked: 3_000 + i as u64,
                    verdict: Verdict::Unknown(reason),
                    first_dead_at: None,
                },
            );
        }

        save(&path, &cache).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, cache);
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.bin");
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn a_corrupt_file_fails_loudly_rather_than_silently_starting_cold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.bin");
        std::fs::write(&path, b"not bincode at all, just garbage bytes").unwrap();
        assert!(load(&path).is_err());
    }
}
