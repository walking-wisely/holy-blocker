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
//!
//! **[`CacheStore`]** is the module 7 follow-up recorded in
//! `docs/decisions/domain-blocklist-sourcing.md`'s "Measured 2026-08-16: the cache's in-process
//! representation — redb, not a `HashMap` blob" section: at the real ~4.75M-domain corpus, holding
//! the sweep's cache as a live `HashMap<String, CacheEntry>` measured **2.71GB RSS**, almost all of
//! it per-`String`/bucket-array heap overhead the ~25-byte domain keys and ~13-21-byte values don't
//! structurally need. [`CacheStore`] instead reads/writes the cache through
//! [redb](https://docs.rs/redb), a single-file mmap-backed embedded KV store, so the OS pages the
//! table in and out under memory pressure rather than the process holding a fixed multi-GB
//! allocation. The old `HashMap`-based `load`/`save` functions are kept unchanged below — they're
//! still the right shape for `run_sweep`/`run_sweep_with_resolvers` (kept for tests and
//! small-corpus callers, per `sweep.rs`'s own doc comments) and for anything reading/writing a
//! *complete* cache snapshot in one shot.
//!
//! **On-disk format migration, decided rather than discovered mid-migration (per the sourcing
//! doc's own "what implementing this needs to account for" list):** [`CacheStore`] writes a
//! **new** file (conventionally `<name>.redb`, distinct from the legacy `<name>.bin` the
//! `HashMap`-based `save` writes), and there is no automatic converter between the two formats.
//! This project has never checked in or produced a real production `cache.bin` — the doc's own
//! module-7 gap list notes "no on-disk `production.bin` exists yet on this pre-checkpointing
//! binary" — so there is nothing to migrate: a deployment switching to [`CacheStore`] starts one
//! cold sweep (every domain due) rather than carrying forward stale bincode state. A future
//! deployment that *does* have an accumulated `cache.bin` before this lands would need to write a
//! one-time `load` (bincode) → `CacheStore::set` (redb) conversion pass; that pass is not built
//! here because there is no real file to test it against yet, and building it against a synthetic
//! fixture would verify nothing about the actual migration risk (byte-for-byte legacy format
//! quirks a synthetic cache can't reproduce).

use std::collections::HashMap;
use std::path::Path;

use domain_blocklist::{CacheEntry, UnknownReason, Verdict};
use redb::ReadableDatabase;
use redb::ReadableTable;

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
#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
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
    std::fs::rename(&tmp, path).map_err(|e| {
        anyhow::anyhow!(
            "failed to rename {} to {}: {e}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

/// The seam [`sweep::process_batch`](crate::sweep) and the final pruning scan use to read/write
/// one cache entry at a time, without committing to a representation — an in-memory
/// `HashMap<String, CacheEntry>` (unbounded, used by tests and small-corpus callers) or the
/// mmap-backed [`CacheStore`] (bounded, used by the real `run_sweep_streaming` production path).
///
/// `flush`/`for_each` default to the no-op/`HashMap`-shaped behavior that costs an in-memory
/// `HashMap` nothing; [`CacheStore`] overrides both to talk to redb.
pub trait CacheBackend: Send {
    fn get(&self, domain: &str) -> Option<CacheEntry>;
    fn set(&mut self, domain: String, entry: CacheEntry);
    /// Commits any buffered writes. A no-op for `HashMap` (nothing is buffered — every `set` is
    /// already durable in the sense this trait cares about); [`CacheStore`] uses this to batch
    /// many `set`s into one redb write transaction rather than one transaction per domain, per
    /// the sourcing doc's "batched write transactions" requirement.
    fn flush(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    /// Visits every cache entry — flushing first, so a caller never sees a stale view that's
    /// missing recently-`set` entries still sitting in a write buffer.
    fn for_each(&mut self, f: &mut dyn FnMut(&CacheEntry)) -> anyhow::Result<()>;
}

impl CacheBackend for HashMap<String, CacheEntry> {
    fn get(&self, domain: &str) -> Option<CacheEntry> {
        HashMap::get(self, domain).copied()
    }

    fn set(&mut self, domain: String, entry: CacheEntry) {
        self.insert(domain, entry);
    }

    fn for_each(&mut self, f: &mut dyn FnMut(&CacheEntry)) -> anyhow::Result<()> {
        for entry in self.values() {
            f(entry);
        }
        Ok(())
    }
}

/// redb table holding one row per domain: key is the raw domain string (already normalized by the
/// caller, same as the `HashMap` path), value is a bincode-encoded [`CacheEntryDto`]. Reusing the
/// existing DTO/bincode encoding (rather than redb's own typed value support) keeps exactly one
/// serialization format for this data, so `load`/`save`'s DTOs and `CacheStore`'s on-disk rows stay
/// byte-compatible with each other if a future migration pass ever wants that.
const TABLE: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new("liveness_cache");

/// The redb-backed [`CacheBackend`] — see this module's doc comment for the measurement that
/// motivated it and the migration decision.
///
/// `set` only buffers in `pending` (a plain in-memory map, bounded by however many domains are
/// checked between [`flush`](CacheBackend::flush) calls — one `canary_every`-sized sweep chunk in
/// practice, a few thousand entries, not the full multi-million-domain corpus); nothing touches
/// redb until `flush` runs one write transaction over the whole buffer. `get` checks `pending`
/// first (so a domain `set` earlier in the same unflushed batch reads back correctly) and falls
/// through to a redb read transaction otherwise.
pub struct CacheStore {
    db: redb::Database,
    pending: HashMap<String, CacheEntry>,
}

impl CacheStore {
    /// Opens (creating if absent) the redb database at `path` and ensures [`TABLE`] exists —
    /// `redb::Database::create` alone doesn't create tables, only the file/header.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let db = redb::Database::create(path)
            .map_err(|e| anyhow::anyhow!("failed to open redb cache at {}: {e}", path.display()))?;
        let txn = db.begin_write().map_err(|e| {
            anyhow::anyhow!("failed to open a write transaction on a fresh redb cache: {e}")
        })?;
        {
            txn.open_table(TABLE)
                .map_err(|e| anyhow::anyhow!("failed to create the liveness_cache table: {e}"))?;
        }
        txn.commit().map_err(|e| {
            anyhow::anyhow!("failed to commit the initial redb table creation: {e}")
        })?;
        Ok(Self {
            db,
            pending: HashMap::new(),
        })
    }

    /// Compacts the underlying redb file, per the sourcing doc's "periodic compaction" note — the
    /// 515MB measurement is a freshly-compacted, single-writer snapshot, not a steady-state-after-
    /// churn one, so a caller running many sweeps against the same file should call this on some
    /// cadence of its own choosing. **Not wired into any automatic cadence here** — the doc
    /// explicitly flags steady-state fragmentation as unmeasured, an open gap rather than a
    /// silently-assumed cadence.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn compact(&mut self) -> anyhow::Result<bool> {
        self.db
            .compact()
            .map_err(|e| anyhow::anyhow!("redb compaction failed: {e}"))
    }
}

impl CacheBackend for CacheStore {
    fn get(&self, domain: &str) -> Option<CacheEntry> {
        if let Some(entry) = self.pending.get(domain) {
            return Some(*entry);
        }
        let txn = self.db.begin_read().ok()?;
        let table = txn.open_table(TABLE).ok()?;
        let guard = table.get(domain).ok().flatten()?;
        let dto: CacheEntryDto = bincode::deserialize(guard.value()).ok()?;
        Some(dto.into())
    }

    fn set(&mut self, domain: String, entry: CacheEntry) {
        self.pending.insert(domain, entry);
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        // Sorted-by-key insert order — the sourcing doc's "free lever" for B-tree page locality,
        // taken here since a write transaction already touches every buffered key once.
        let mut items: Vec<(String, CacheEntry)> = self.pending.drain().collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));

        let txn = self
            .db
            .begin_write()
            .map_err(|e| anyhow::anyhow!("failed to open a redb write transaction: {e}"))?;
        {
            let mut table = txn
                .open_table(TABLE)
                .map_err(|e| anyhow::anyhow!("failed to open liveness_cache for writing: {e}"))?;
            for (domain, entry) in &items {
                let dto: CacheEntryDto = (*entry).into();
                let bytes = bincode::serialize(&dto).map_err(|e| {
                    anyhow::anyhow!("failed to serialize cache entry for {domain:?}: {e}")
                })?;
                table
                    .insert(domain.as_str(), bytes.as_slice())
                    .map_err(|e| {
                        anyhow::anyhow!("failed to write cache entry for {domain:?}: {e}")
                    })?;
            }
        }
        txn.commit()
            .map_err(|e| anyhow::anyhow!("failed to commit redb write transaction: {e}"))?;
        Ok(())
    }

    fn for_each(&mut self, f: &mut dyn FnMut(&CacheEntry)) -> anyhow::Result<()> {
        self.flush()?;
        let txn = self
            .db
            .begin_read()
            .map_err(|e| anyhow::anyhow!("failed to open a redb read transaction: {e}"))?;
        let table = txn
            .open_table(TABLE)
            .map_err(|e| anyhow::anyhow!("failed to open liveness_cache for reading: {e}"))?;
        let iter = table
            .iter()
            .map_err(|e| anyhow::anyhow!("failed to iterate liveness_cache: {e}"))?;
        for row in iter {
            let (_key, value) =
                row.map_err(|e| anyhow::anyhow!("failed to read a cache row: {e}"))?;
            let dto: CacheEntryDto = bincode::deserialize(value.value())
                .map_err(|e| anyhow::anyhow!("failed to decode a cache row: {e}"))?;
            let entry: CacheEntry = dto.into();
            f(&entry);
        }
        Ok(())
    }
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

    fn sample(last_checked: u64, verdict: Verdict) -> CacheEntry {
        CacheEntry {
            last_checked,
            verdict,
            first_dead_at: None,
        }
    }

    #[test]
    fn a_domain_set_but_not_yet_flushed_still_reads_back_from_the_pending_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CacheStore::open(&dir.path().join("cache.redb")).unwrap();
        store.set("example.com".to_string(), sample(1_000, Verdict::Alive));
        assert_eq!(
            store.get("example.com"),
            Some(sample(1_000, Verdict::Alive))
        );
    }

    #[test]
    fn a_flushed_entry_survives_reopening_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        {
            let mut store = CacheStore::open(&path).unwrap();
            store.set("example.com".to_string(), sample(1_000, Verdict::Dead));
            store.flush().unwrap();
        }
        let store = CacheStore::open(&path).unwrap();
        assert_eq!(store.get("example.com"), Some(sample(1_000, Verdict::Dead)));
    }

    #[test]
    fn an_unflushed_entry_is_not_visible_after_reopening_a_fresh_handle() {
        // `set` only buffers in-process; reopening the same file (standing in for "the process
        // restarted before the next checkpoint" — redb itself refuses two live handles on one
        // file, so the first must be dropped first, exactly as a real restart implies the old
        // process is gone) must not see an unflushed `set` — otherwise a crash between
        // checkpoints would look like it lost no data when it did.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        {
            let mut store = CacheStore::open(&path).unwrap();
            store.set(
                "never-flushed.example".to_string(),
                sample(1_000, Verdict::Alive),
            );
        }

        let reopened = CacheStore::open(&path).unwrap();
        assert_eq!(reopened.get("never-flushed.example"), None);
    }

    #[test]
    fn flushing_an_empty_pending_buffer_is_a_cheap_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CacheStore::open(&dir.path().join("cache.redb")).unwrap();
        store.flush().unwrap();
        store.flush().unwrap();
    }

    #[test]
    fn for_each_visits_every_entry_including_still_pending_ones() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CacheStore::open(&dir.path().join("cache.redb")).unwrap();
        store.set("flushed.example".to_string(), sample(1_000, Verdict::Alive));
        store.flush().unwrap();
        store.set("pending.example".to_string(), sample(2_000, Verdict::Dead));

        let mut seen: Vec<CacheEntry> = Vec::new();
        store.for_each(&mut |e| seen.push(*e)).unwrap();
        seen.sort_by_key(|e| e.last_checked);

        assert_eq!(
            seen,
            vec![sample(1_000, Verdict::Alive), sample(2_000, Verdict::Dead)]
        );
    }

    #[test]
    fn for_each_leaves_the_pending_buffer_flushed_not_duplicated() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CacheStore::open(&dir.path().join("cache.redb")).unwrap();
        store.set("once.example".to_string(), sample(1_000, Verdict::Alive));

        let mut first_pass = Vec::new();
        store.for_each(&mut |e| first_pass.push(*e)).unwrap();
        let mut second_pass = Vec::new();
        store.for_each(&mut |e| second_pass.push(*e)).unwrap();

        assert_eq!(first_pass.len(), 1);
        assert_eq!(second_pass.len(), 1);
    }

    #[test]
    fn overwriting_a_domain_before_flush_keeps_only_the_latest_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CacheStore::open(&dir.path().join("cache.redb")).unwrap();
        store.set("example.com".to_string(), sample(1_000, Verdict::Alive));
        store.set("example.com".to_string(), sample(2_000, Verdict::Dead));
        store.flush().unwrap();
        assert_eq!(store.get("example.com"), Some(sample(2_000, Verdict::Dead)));
    }

    #[test]
    fn compact_does_not_lose_flushed_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CacheStore::open(&dir.path().join("cache.redb")).unwrap();
        store.set("example.com".to_string(), sample(1_000, Verdict::Alive));
        store.flush().unwrap();
        store.compact().unwrap();
        assert_eq!(
            store.get("example.com"),
            Some(sample(1_000, Verdict::Alive))
        );
    }
}
