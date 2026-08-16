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
//! [redb](https://docs.rs/redb) 4.1.0, a single-file embedded KV store with its own **in-process,
//! bounded page cache** — redb is not mmap-backed (an earlier draft of this doc comment claimed it
//! was; that was never true of this crate's version and is corrected here). The cache is
//! process-owned anonymous heap, not a clean file-backed mapping the kernel can drop under memory
//! pressure the way it can with real mmap pages, so leaving `redb::Builder`'s 1 GiB default in
//! place would trade one unbounded allocator (a resident `HashMap`) for another. [`open`](CacheStore::open)
//! therefore sets an explicit, bounded cache size — see [`DEFAULT_CACHE_SIZE_BYTES`] for the
//! measurement that picked its value. The old `HashMap`-based `load`/`save` functions are kept unchanged below — they're
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

/// Reads a snapshot of an existing redb cache file at `path` into a plain `HashMap`, **without
/// ever creating the file if it's missing or writing anything to it** — the seam that lets a dry
/// run (`main.rs`'s `run_liveness`) preview real gate decisions against real prior cache state
/// again, while still keeping its own "a dry run never persists `--cache`" guarantee, which this
/// function cannot violate structurally: it never opens a write transaction and never touches a
/// path that doesn't already exist.
///
/// [`CacheStore::open`] was the wrong tool for this — it always creates the file (and one write
/// transaction to ensure the table exists) if absent, which is correct for a real run but not
/// something a dry run should ever trigger, however harmless the resulting empty file would be.
///
/// A missing file returns an empty snapshot — "a run that cannot load the cache starts cold," the
/// same rule [`load`] documents for the legacy bincode path — rather than an error, since a dry
/// run against a `--cache` path that hasn't been swept into yet is exactly as valid as a genuine
/// first sweep.
pub fn read_snapshot(path: &Path) -> anyhow::Result<HashMap<String, CacheEntry>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let db = redb::Builder::new()
        .set_cache_size(DEFAULT_CACHE_SIZE_BYTES)
        .open(path)
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to open redb cache at {} for a read-only dry-run snapshot: {e} — if this \
                 path previously held the legacy bincode `cache.bin` format, that's the likely \
                 cause (see this module's own doc comment)",
                path.display()
            )
        })?;
    let txn = db
        .begin_read()
        .map_err(|e| anyhow::anyhow!("failed to open a read transaction for a snapshot: {e}"))?;
    let table = match txn.open_table(TABLE) {
        Ok(table) => table,
        // A redb file this crate created always has the table (`CacheStore::open` creates it
        // eagerly) — this only guards against some other, unexpected redb file at the path.
        Err(redb::TableError::TableDoesNotExist(_)) => return Ok(HashMap::new()),
        Err(e) => {
            return Err(anyhow::anyhow!(
                "failed to open liveness_cache for a read-only snapshot: {e}"
            ));
        }
    };
    let iter = table
        .iter()
        .map_err(|e| anyhow::anyhow!("failed to iterate liveness_cache for a snapshot: {e}"))?;
    let mut out = HashMap::new();
    for row in iter {
        let (key, value) = row.map_err(|e| anyhow::anyhow!("failed to read a cache row: {e}"))?;
        let dto: CacheEntryDto = bincode::deserialize(value.value())
            .map_err(|e| anyhow::anyhow!("failed to decode a cache row: {e}"))?;
        out.insert(key.value().to_string(), dto.into());
    }
    Ok(out)
}

/// Bounded redb in-process cache budget [`CacheStore::open`] uses instead of `redb::Builder`'s 1
/// GiB default. Measured 2026-08-16 against the same 4.8M-synthetic-domain stress harness this
/// module's own doc comment cites (`stress_test_streaming_sweep_with_redb_cache_bounds_rss`),
/// changing only `redb::Database::create(path)` → `Builder::new().set_cache_size(...).create(path)`:
///
/// | variant | final RSS |
/// |---|---|
/// | `HashMap` baseline | 1353.8 MB |
/// | redb, 1 GiB default cache | 1126.7 MB |
/// | redb, 128 MiB cache budget | 613.9 MB (one measured run) / 942.8 MB (a second, separately
/// reproduced run — see below) |
///
/// No measurable CPU cost difference and identical on-disk file size (514.0 MB) between the two
/// redb variants in every run. 128 MiB is small enough to bound RSS hard at multi-million-domain
/// scale while still being large enough that the hot pages touched during one `canary_every`-sized
/// sweep chunk mostly hit cache rather than a disk read.
///
/// **Reported honestly rather than reconciled**: two separately reproduced runs of the same
/// before/after comparison did not agree on the exact final-RSS number for the bounded-cache
/// variant (613.9 MB vs. 942.8 MB), though both agree the fix is real and correctly targeted
/// (both land well under the unbounded-default 1126.7 MB). The most likely cause, per
/// `docs/decisions/domain-blocklist-sourcing.md`'s corrected redb section, is that this stress
/// test's own ~516 MB entry-corpus build-then-drop overhead is not reliably released back to the
/// OS by macOS's allocator between runs, so it doesn't cancel out cleanly. Neither number is
/// confirmed against the real Linux VPS deployment target.
pub const DEFAULT_CACHE_SIZE_BYTES: usize = 128 * 1024 * 1024;

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
    /// Counts real `get` failures (a transaction that couldn't open, a table that couldn't open, a
    /// row that couldn't decode) distinctly from an ordinary cache miss — see [`CacheBackend::get`]
    /// on [`CacheStore`]'s own doc comment for why this distinction matters. `AtomicU64` because
    /// `get` takes `&self` (this crate's `CacheBackend::get` is not `&mut self`, matching the
    /// `HashMap` impl, which needs no mutability either), so a plain counter field can't be bumped
    /// without interior mutability. Mirrors the counter pattern `merge.rs`'s `MergeReport` uses for
    /// its own soft-failure drop counts.
    error_count: std::sync::atomic::AtomicU64,
}

impl CacheStore {
    /// Opens (creating if absent) the redb database at `path`, bounding its in-process page cache
    /// to [`DEFAULT_CACHE_SIZE_BYTES`] rather than `redb::Builder`'s 1 GiB default — see this
    /// module's doc comment and [`DEFAULT_CACHE_SIZE_BYTES`]'s own doc comment for the measurement.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Self::open_with_cache_size(path, DEFAULT_CACHE_SIZE_BYTES)
    }

    /// Same as [`open`](Self::open), but with an explicit redb in-process cache budget in bytes —
    /// the seam a future `--cache-size-mb` CLI flag (or any other caller wanting a different
    /// budget than the shipped default) would call into.
    pub fn open_with_cache_size(path: &Path, cache_size_bytes: usize) -> anyhow::Result<Self> {
        let db = redb::Builder::new()
            .set_cache_size(cache_size_bytes)
            .create(path)
            .map_err(|e| {
                // redb's own error here is a generic "invalid data"/header-mismatch failure —
                // the same failure a stale legacy bincode `cache.bin` (or any other non-redb
                // file) at this path produces. Name the likely cause and the remedy this module's
                // own doc comment already describes rather than leaving the operator to guess.
                anyhow::anyhow!(
                    "failed to open redb cache at {}: {e} — if this path previously held the \
                     legacy bincode `cache.bin` format (or any other non-redb file), that's the \
                     likely cause: there is no automatic migrator, so start one cold sweep \
                     against a fresh --cache path instead (see this module's own doc comment)",
                    path.display()
                )
            })?;
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
            error_count: std::sync::atomic::AtomicU64::new(0),
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

    /// Number of `get` calls that hit a real error (a transaction/table that couldn't open, or a
    /// row that failed to decode) rather than an ordinary cache miss, since this store was opened.
    /// See [`CacheBackend::get`]'s impl on [`CacheStore`] for why the two are logged and counted
    /// separately instead of both silently collapsing to `None`.
    pub fn error_count(&self) -> u64 {
        self.error_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl CacheBackend for CacheStore {
    /// A real read error (transaction/table open failure, an undecodable row) is distinguished
    /// from "this domain has no cache entry" — both used to collapse to `None` via four chained
    /// `.ok()?` calls, indistinguishable to every caller. That mattered concretely in
    /// `sweep::process_batch`: a transient read error on a domain previously marked `Dead` made
    /// `entry_before.first_dead_at` read as `None`, so that batch's fresh `Dead` verdict reset
    /// `first_dead_at = now` — silently restarting the domain's whole quarantine clock on a read
    /// error, with nothing to distinguish it from a domain that genuinely just died. A real error
    /// now logs (naming the domain and the failure) and increments [`CacheStore::error_count`]
    /// instead of silently reading as a miss; this is the minimum fix rather than propagating
    /// `Result` through [`CacheBackend::get`] and every caller, since the two failure classes
    /// still need the same fallback behavior (treat as due/unknown) and only need to be
    /// *observable* as distinct, not handled differently by every call site.
    fn get(&self, domain: &str) -> Option<CacheEntry> {
        if let Some(entry) = self.pending.get(domain) {
            return Some(*entry);
        }
        let txn = match self.db.begin_read() {
            Ok(txn) => txn,
            Err(e) => {
                self.error_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(domain, error = %e, "redb read transaction failed on get — treating as a cache miss, not a confirmed absence");
                return None;
            }
        };
        let table = match txn.open_table(TABLE) {
            Ok(table) => table,
            Err(e) => {
                self.error_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(domain, error = %e, "failed to open liveness_cache table on get — treating as a cache miss, not a confirmed absence");
                return None;
            }
        };
        let guard = match table.get(domain) {
            Ok(Some(guard)) => guard,
            Ok(None) => return None, // a genuine miss — no row for this domain
            Err(e) => {
                self.error_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(domain, error = %e, "redb row lookup failed on get — treating as a cache miss, not a confirmed absence");
                return None;
            }
        };
        match bincode::deserialize::<CacheEntryDto>(guard.value()) {
            Ok(dto) => Some(dto.into()),
            Err(e) => {
                self.error_count
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!(domain, error = %e, "cache row failed to decode on get — treating as a cache miss, not a confirmed absence");
                None
            }
        }
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
        items.sort_unstable_by(|a, b| a.0.cmp(&b.0));

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

    /// **Invariant this unconditional `flush()` relies on**: nothing here checks whether it's
    /// safe to write `--cache` back to disk, because that's not this function's job to check — a
    /// dry run never gets far enough to call this at all. `main.rs`'s `run_liveness` only ever
    /// constructs a real [`CacheStore`] on the non-dry-run branch (a dry run builds
    /// `LivenessCache::Memory` instead, via [`read_snapshot`], which never touches redb); as long
    /// as that split holds, a live `CacheStore::for_each` call is proof enough that persisting is
    /// already allowed. If a future refactor ever lets a dry run construct a real `CacheStore`
    /// directly, this flush becomes the "a dry run must never persist `--cache`" guarantee's
    /// weakest point — worth an explicit "may I flush?" signal at that point, not before.
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

    #[test]
    fn open_with_cache_size_bounds_the_redb_in_process_cache() {
        let dir = tempfile::tempdir().unwrap();
        // Just asserts the constructor accepts and uses a caller-chosen budget rather than always
        // falling back to `open`'s default — the real payoff (bounded RSS at multi-million-domain
        // scale) is what the `stress_test_streaming_sweep_with_redb_cache_bounds_rss` `#[ignore]`d
        // stress test in `sweep.rs` measures, not something a fast unit test can observe directly.
        let mut store =
            CacheStore::open_with_cache_size(&dir.path().join("cache.redb"), 4 * 1024 * 1024)
                .unwrap();
        store.set("example.com".to_string(), sample(1_000, Verdict::Alive));
        store.flush().unwrap();
        assert_eq!(
            store.get("example.com"),
            Some(sample(1_000, Verdict::Alive))
        );
    }

    /// Regression test for the bug where `get` chained four `.ok()?` calls and collapsed a real
    /// error (here: an undecodable row) into the same `None` an ordinary cache miss returns. A
    /// row is written directly through redb (bypassing `set`/`flush`, which only ever write valid
    /// `CacheEntryDto` bytes) with garbage bincode, standing in for on-disk corruption or a future
    /// encoding bug — `get` must not panic, must still return `None` (the caller-visible fallback
    /// behavior is unchanged), but must count the failure as distinct from a genuine miss.
    #[test]
    fn get_counts_a_real_decode_failure_separately_from_an_ordinary_miss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        let store = CacheStore::open(&path).unwrap();
        assert_eq!(store.error_count(), 0);

        {
            let txn = store.db.begin_write().unwrap();
            {
                let mut table = txn.open_table(TABLE).unwrap();
                table
                    .insert("corrupt.example", b"not a valid CacheEntryDto".as_slice())
                    .unwrap();
            }
            txn.commit().unwrap();
        }

        // A genuine miss must still be a plain `None` with no error counted.
        assert_eq!(store.get("never-set.example"), None);
        assert_eq!(store.error_count(), 0);

        // The corrupt row is also `None` to the caller (unchanged fallback behavior) but is now
        // distinguishable via the counter.
        assert_eq!(store.get("corrupt.example"), None);
        assert_eq!(store.error_count(), 1);

        // Repeated reads of the same corrupt row keep counting — this isn't a one-shot latch.
        assert_eq!(store.get("corrupt.example"), None);
        assert_eq!(store.error_count(), 2);
    }

    #[test]
    fn read_snapshot_of_a_missing_file_is_empty_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.redb");
        let snapshot = read_snapshot(&path).unwrap();
        assert!(snapshot.is_empty());
        // The whole point: unlike `CacheStore::open`, this must never bring the file into
        // existence — a dry run with `--cache` pointing at a path that hasn't been swept into
        // yet must not itself create that path.
        assert!(!path.exists());
    }

    #[test]
    fn read_snapshot_sees_real_prior_state_without_ever_writing_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.redb");
        {
            let mut store = CacheStore::open(&path).unwrap();
            store.set("alive.example".to_string(), sample(1_000, Verdict::Alive));
            store.set("dead.example".to_string(), sample(2_000, Verdict::Dead));
            store.flush().unwrap();
        }
        let before = std::fs::metadata(&path).unwrap().len();

        let snapshot = read_snapshot(&path).unwrap();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot.get("alive.example"), Some(&sample(1_000, Verdict::Alive)));
        assert_eq!(snapshot.get("dead.example"), Some(&sample(2_000, Verdict::Dead)));

        // A read-only snapshot must not change the file on disk.
        let after = std::fs::metadata(&path).unwrap().len();
        assert_eq!(before, after);
    }
}
