//! Module 6 of the domain-blocklist plan — the signed-artifact loading path for `net-shield`.
//!
//! This is a *separate* handle from [`DomainFilter`]: `DomainFilter` (`radix.rs`) stays a pure,
//! no-I/O trie over a flat rule slice, while this module owns the mmap-backed `.fst` artifact, its
//! signature/digest verification, its two-slot (`current`/`previous`) on-device layout, and the
//! bounded-latency worker that answers lookups off the packet-handling thread.
//!
//! The precedence table (device-local allowlist → explicit `DomainFilter` rules → FST blocklist →
//! default) is implemented in [`resolve_action`]. The query side normalizes with the *same*
//! `domain_normalize::normalize` the build side uses — neither side reimplements it, so a build-time
//! and query-time normalization mismatch is impossible by construction.
//!
//! See `docs/components/domain-blocklist/plan.md` module 6 and the decision doc's
//! [On-device storage and lookup](../../docs/decisions/domain-blocklist-sourcing.md) section for the
//! contract this implements.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use domain_blocklist::{KeyId, Manifest, ProvenanceEntry, reverse_key, verify_manifest};
use domain_normalize::RuleScope;
use ed25519_dalek::VerifyingKey;
use memmap2::{Mmap, MmapOptions};
use sha2::{Digest, Sha256};

use crate::radix::{DomainFilter, FilterAction};

// ---------------------------------------------------------------------------
// On-device slot layout
// ---------------------------------------------------------------------------

/// File holding the artifact's FST bytes inside a slot directory (`current/` or `previous/`).
pub const ARTIFACT_FILE: &str = "artifact.fst";
/// File holding the bincode-serialized [`Manifest`] inside a slot directory.
pub const MANIFEST_FILE: &str = "manifest.bin";
const CURRENT_SLOT: &str = "current";
const PREVIOUS_SLOT: &str = "previous";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotDir {
    Current,
    Previous,
}

impl SlotDir {
    fn name(self) -> &'static str {
        match self {
            SlotDir::Current => CURRENT_SLOT,
            SlotDir::Previous => PREVIOUS_SLOT,
        }
    }
}

// ---------------------------------------------------------------------------
// BlocklistArtifact — the verified, mmap-backed FST
// ---------------------------------------------------------------------------

/// A verified, mmap-backed blocklist artifact. Owns the mapping directly through an
/// [`fst::Map<Mmap>`] (which keeps the `Mmap` alive as its backing store), so there is no
/// self-referential `Mmap`/`Map` pair to keep in sync and the whole artifact is `Send + Sync` —
/// reference-counted through `Arc` for swap-safe sharing, exactly the plan's `map: Arc<Mmap>`
/// intent without the lifetime juggling the two-field layout would force.
pub struct BlocklistArtifact {
    map: fst::Map<Mmap>,
    provenance: Vec<ProvenanceEntry>,
    version: u64,
}

/// Why an artifact could not be loaded. `Err` always means "no usable artifact" — never a
/// partially-trusted one. This is the cold-start loader, so a `BadSignature`/`DigestMismatch` in
/// `current/` falls back to `previous/` before giving up (the decision doc's last-known-good rule).
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("no usable artifact slot under {0}")]
    NoUsableSlot(String),
    #[error("slot directory does not exist")]
    SlotMissing,
    #[error("could not read {0}: {1}")]
    Io(String, #[source] std::io::Error),
    #[error("could not deserialize manifest from {0}: {1}")]
    Manifest(String, #[source] bincode::Error),
    #[error("manifest signature did not verify")]
    BadSignature,
    #[error("manifest fst_digest did not match the artifact file")]
    DigestMismatch,
    #[error("artifact is not a valid fst map: {0}")]
    BadFst(String),
    #[error("artifact verification could not run: {0}")]
    Verify(String),
}

impl BlocklistArtifact {
    /// The `version` this artifact carries — the decision doc's rollback high-water mark source.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Cold-start loader. Verifies `current/` (signature against `trusted_keys`, then the
    /// `fst_digest` over the artifact bytes); if that slot is missing or fails verification, tries
    /// `previous/` the same way; if both fail, returns `Err` (no artifact — `net-shield`'s existing
    /// default action applies). The `version > high-water-mark` rollback check is the update/accept
    /// path's job, not this cold-start loader's.
    pub fn load(path: &Path, trusted_keys: &[(KeyId, VerifyingKey)]) -> Result<Self, ArtifactError> {
        let mut first_failure: Option<ArtifactError> = None;
        for slot in [SlotDir::Current, SlotDir::Previous] {
            match Self::load_slot(path, slot, trusted_keys) {
                Ok(artifact) => return Ok(artifact),
                Err(ArtifactError::SlotMissing) => continue,
                Err(e) => {
                    if first_failure.is_none() {
                        first_failure = Some(e);
                    }
                }
            }
        }
        Err(first_failure.unwrap_or(ArtifactError::SlotMissing))
    }

    /// `load` from a slot directory, verifying signature then digest before trusting anything.
    fn load_slot(
        path: &Path,
        slot: SlotDir,
        trusted_keys: &[(KeyId, VerifyingKey)],
    ) -> Result<Self, ArtifactError> {
        let dir = path.join(slot.name());
        let fst_path = dir.join(ARTIFACT_FILE);
        let manifest_path = dir.join(MANIFEST_FILE);

        let file = match File::open(&fst_path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ArtifactError::SlotMissing)
            }
            Err(e) => return Err(ArtifactError::Io(fst_path.display().to_string(), e)),
        };
        let manifest_bytes = std::fs::read(&manifest_path)
            .map_err(|e| ArtifactError::Io(manifest_path.display().to_string(), e))?;
        let manifest: Manifest = bincode::deserialize(&manifest_bytes)
            .map_err(|e| ArtifactError::Manifest(manifest_path.display().to_string(), e))?;

        // Verify the signature first — nothing from an unverified manifest may be trusted.
        let verified = verify_manifest(&manifest, trusted_keys)
            .map_err(|e| ArtifactError::Verify(e.to_string()))?;
        if !verified {
            return Err(ArtifactError::BadSignature);
        }

        let mmap = unsafe { MmapOptions::new().map_copy_read_only(&file) }
            .map_err(|e| ArtifactError::Io(fst_path.display().to_string(), e))?;

        // Bind the manifest to the artifact: the digest must cover exactly the bytes we map.
        let mut hasher = Sha256::new();
        hasher.update(mmap.as_ref());
        let digest: [u8; 32] = hasher.finalize().into();
        if digest != manifest.fst_digest {
            return Err(ArtifactError::DigestMismatch);
        }

        let map = fst::Map::new(mmap).map_err(|e| ArtifactError::BadFst(e.to_string()))?;
        Ok(BlocklistArtifact {
            map,
            provenance: manifest.provenance_table,
            version: manifest.version,
        })
    }

    /// Exact-key provenance lookup — the plan's `provenance_for(&reversed_key)`.
    pub fn provenance_for(&self, reversed_key: &str) -> Option<&ProvenanceEntry> {
        let id = self.map.get(reversed_key)?;
        self.provenance.get(id as usize)
    }

    /// Scope-aware, label-boundary lookup of a *reversed* normalized query key (e.g. `com.example`
    /// for `example.com`). The decision doc's rule: one exact `get` per label boundary from shortest
    /// to longest; an `Apex` entry covers everything below it, an `ExactHost` entry matches only at
    /// the full key. Returns `Some(Block)` if any stored rule covers the query, else `None`. The
    /// artifact only ever stores block rules, so every hit is `Block` — the action is fixed at this
    /// boundary.
    ///
    /// This is intentionally the `get`-at-label-boundary approach, not prefix streaming: a raw
    /// prefix query for `com.example` would also match `com.examplezzz` (i.e. `examplezzz.com`), the
    /// exact hazard the decision doc's "anchored to end exactly at a label boundary" rule exists to
    /// avoid (decision doc l.674).
    pub fn lookup(&self, reversed_query: &str) -> Option<FilterAction> {
        let bytes = reversed_query.as_bytes();
        let mut boundaries: Vec<usize> = Vec::new();
        for (idx, &b) in bytes.iter().enumerate() {
            if b == b'.' {
                boundaries.push(idx);
            }
        }
        boundaries.push(bytes.len());

        for &i in &boundaries {
            let prefix = &bytes[..i];
            let Some(id) = self.map.get(prefix) else {
                continue;
            };
            let Some(prov) = self.provenance.get(id as usize) else {
                continue;
            };
            if prov.scope == RuleScope::Apex {
                return Some(FilterAction::Block);
            }
            if i == bytes.len() && prov.scope == RuleScope::ExactHost {
                return Some(FilterAction::Block);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Allowlist — level 1 of the precedence table
// ---------------------------------------------------------------------------

/// The device-local user allowlist (level 1, always wins). Matches a normalized domain and every
/// label-suffix (subdomain) of it: allowing `example.com` also allows `www.example.com`. The exact
/// persistence/mechanics of the allowlist belong to `net-shield`'s own plan; this is just the
/// decision structure the precedence table needs.
#[derive(Debug, Default)]
pub struct Allowlist {
    entries: HashMap<String, ()>,
}

impl Allowlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_entries(entries: &[&str]) -> Self {
        let mut this = Self::new();
        for e in entries {
            this.insert(e);
        }
        this
    }

    pub fn insert(&mut self, normalized: &str) {
        // Only keep entries that actually normalized; an un-normalizable allowlist entry can never
        // match a normalized query, so storing it is dead weight.
        if let Ok(n) = domain_normalize::normalize(normalized) {
            self.entries.insert(n, ());
        }
    }

    /// Whether `normalized` is covered: an exact entry, or one of its label-suffixes.
    pub fn contains(&self, normalized: &str) -> bool {
        if self.entries.contains_key(normalized) {
            return true;
        }
        let bytes = normalized.as_bytes();
        for (idx, &b) in bytes.iter().enumerate() {
            if b == b'.' && self.entries.contains_key(&normalized[idx + 1..]) {
                return true;
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Precedence resolution — the top-level filter-query path
// ---------------------------------------------------------------------------

/// Resolves a normalized domain against the precedence table the plan's module 6 fixes:
/// device-local allowlist (1) → explicit `DomainFilter` rules (2) → FST blocklist (4) → default (5).
/// Level 3 (overlay, module 8) is unbuilt; the caller passes `None` for the artifact to leave it out.
///
/// Returns `None` only when the domain fell through levels 1–4 with no decision — the caller then
/// applies the level-5 default (`Proxy`). This split keeps levels 1–2 (cheap, no mmap) on the calling
/// thread and hands only level 4 to the bounded-latency worker.
pub fn resolve_action(
    normalized: &str,
    allowlist: &Allowlist,
    explicit: &DomainFilter,
    artifact: Option<&BlocklistArtifact>,
) -> Option<FilterAction> {
    // Level 1: device-local allowlist, always wins.
    if allowlist.contains(normalized) {
        return Some(FilterAction::Allow);
    }
    // Level 2: explicit DomainFilter rules. `rule_for` distinguishes an explicit rule (returns
    // Some) from the trie's miss default (None), so an explicit `Proxy` beats the FST exactly the
    // way the plan says an explicit `Allow`/`Block` does, and a miss falls through to level 4.
    match explicit.rule_for(normalized) {
        Some(action @ (FilterAction::Allow | FilterAction::Block | FilterAction::Proxy)) => {
            return Some(action);
        }
        None => {}
    }
    // Level 4: the FST blocklist (through the worker in the DnsShield path).
    if let Some(artifact) = artifact {
        let reversed = reverse_key(normalized);
        if let Some(action) = artifact.lookup(&reversed) {
            return Some(action);
        }
    }
    // Level 3 (overlay) is unbuilt; level 5 default is the caller's, not ours.
    None
}

// ---------------------------------------------------------------------------
// Lookup-budget worker — bounded latency on the hot path
// ---------------------------------------------------------------------------

type Responder = Sender<Option<FilterAction>>;

enum WorkerMsg {
    Lookup(String),
}

// The bound on the worker's request queue. Arbitrary and deliberately not load-bearing: a full
// queue is handled (the caller falls back to the LRU/default and counts the event) rather than
// blocking the packet thread, so the exact value only trades "how many in-flight before we give up
// this round" against memory. No measurement is claimed for it.
const CHANNEL_CAPACITY: usize = 1024;

struct Worker {
    artifact: Arc<BlocklistArtifact>,
    rx: Receiver<WorkerMsg>,
    inflight: Arc<Mutex<HashMap<String, Vec<Responder>>>>,
    cache: Arc<Mutex<LruCache>>,
    worker_requests: Arc<AtomicU64>,
    panics: Arc<AtomicU64>,
    delay: Duration,
}

impl Worker {
    fn run(self) {
        while let Ok(msg) = self.rx.recv() {
            match msg {
                WorkerMsg::Lookup(domain) => self.handle_lookup(domain),
            }
        }
    }

    /// Answers one lookup request. Two things here are load-bearing, not incidental:
    ///
    /// - `self.artifact.lookup` runs inside `catch_unwind`. Without this, a panic here (e.g. a
    ///   future FST-decoding bug) would kill the whole worker thread: every domain not already
    ///   cached would silently degrade to the level-5 default forever (the channel send in
    ///   `BlocklistLookup::lookup` starts failing once `rx` drops, which *is* caught and falls
    ///   back — but every domain, not just the one that panicked), and the domain that was
    ///   in-flight at the moment of the panic would never have its `inflight` entry removed,
    ///   leaking one `Responder` per subsequent `lookup()` call for that domain for the rest of
    ///   the process's life. A caught panic instead answers this one request as "no rule" (fail
    ///   open on the panic itself, never on a half-computed result) and the worker keeps running.
    /// - The cache is populated *before* `inflight` is drained, not after. Populating first closes
    ///   the window where a `lookup()` call arriving between the two steps would see both an empty
    ///   `inflight` entry and a cache miss, and issue a duplicate worker request for a domain that
    ///   was just resolved.
    /// - `worker_requests` is incremented *before* any waiter is sent its answer, not after.
    ///   Sending an answer can wake a caller's `recv_timeout` immediately, on another thread, and a
    ///   caller is allowed to read `worker_requests_processed()` right after `lookup()` returns —
    ///   incrementing afterward raced that read against this thread's own next instruction, which
    ///   is exactly the flake CI caught: the counter could still read the pre-increment value at
    ///   the moment a freshly-woken caller checked it.
    fn handle_lookup(&self, domain: String) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let reversed = reverse_key(&domain);
            self.artifact.lookup(&reversed)
        }));
        let decision = match outcome {
            Ok(decision) => decision,
            Err(_) => {
                self.panics.fetch_add(1, Ordering::SeqCst);
                None
            }
        };

        // Test seam: a configured delay lets a slow-worker test force the caller's 2 ms budget to
        // expire deterministically.
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }

        self.cache
            .lock()
            .expect("cache mutex poisoned")
            .put(domain.clone(), decision.clone());
        self.worker_requests.fetch_add(1, Ordering::SeqCst);

        let waiters = self
            .inflight
            .lock()
            .expect("inflight mutex poisoned")
            .remove(&domain)
            .unwrap_or_default();
        for w in waiters {
            let _ = w.send(decision.clone());
        }
    }
}

/// The bounded-latency handle `DnsShield` (and the top-level filter path) uses to consult the
/// artifact without ever blocking packet delivery on an mmap major fault.
///
/// A single worker thread owns the `Arc<BlocklistArtifact>`. Requests travel over a bounded channel
/// keyed by the **normalized** domain; the caller waits at most `budget` for a response, falling
/// back to a small in-memory LRU (then to the default action) on expiry, and counting the fallback.
/// A request already in flight for a domain is not duplicated — later callers attach to the same
/// in-flight future.
pub struct BlocklistLookup {
    request_tx: SyncSender<WorkerMsg>,
    inflight: Arc<Mutex<HashMap<String, Vec<Responder>>>>,
    cache: Arc<Mutex<LruCache>>,
    fallback_count: Arc<AtomicU64>,
    worker_requests: Arc<AtomicU64>,
    panics: Arc<AtomicU64>,
    budget: Duration,
}

impl BlocklistLookup {
    /// Spawn a worker over `artifact`. `budget` is the caller's wait (plan's starting value: 2 ms);
    /// `cache_capacity` bounds the in-memory LRU of past decisions.
    pub fn new(artifact: Arc<BlocklistArtifact>, budget: Duration, cache_capacity: usize) -> Self {
        Self::spawn(artifact, budget, cache_capacity, Duration::ZERO)
    }

    /// Number of fallbacks (budget expiries) that have occurred — a metric, not a silently absorbed
    /// path: a fallback that fires constantly means the budget or cache size needs revisiting.
    pub fn fallback_count(&self) -> u64 {
        self.fallback_count.load(Ordering::SeqCst)
    }

    /// Number of worker requests actually processed — a diagnostic for verifying in-flight
    /// deduplication (a burst of packets to one domain must cost one worker round trip).
    pub fn worker_requests_processed(&self) -> u64 {
        self.worker_requests.load(Ordering::SeqCst)
    }

    /// Number of lookups that panicked inside the worker and were caught rather than killing the
    /// worker thread — a metric, not a silently absorbed path: a nonzero count means some FST
    /// lookup is failing in a way `BlocklistArtifact::lookup` doesn't already handle as `None`.
    pub fn panic_count(&self) -> u64 {
        self.panics.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn with_worker_delay(
        artifact: Arc<BlocklistArtifact>,
        budget: Duration,
        cache_capacity: usize,
        worker_delay: Duration,
    ) -> Self {
        Self::spawn(artifact, budget, cache_capacity, worker_delay)
    }

    fn spawn(
        artifact: Arc<BlocklistArtifact>,
        budget: Duration,
        cache_capacity: usize,
        delay: Duration,
    ) -> Self {
        let (tx, rx) = sync_channel(CHANNEL_CAPACITY);
        let inflight = Arc::new(Mutex::new(HashMap::new()));
        let cache = Arc::new(Mutex::new(LruCache::new(cache_capacity)));
        let worker_requests = Arc::new(AtomicU64::new(0));
        let fallback_count = Arc::new(AtomicU64::new(0));
        let panics = Arc::new(AtomicU64::new(0));

        let worker = Worker {
            artifact,
            rx,
            inflight: inflight.clone(),
            cache: cache.clone(),
            worker_requests: worker_requests.clone(),
            panics: panics.clone(),
            delay,
        };
        std::thread::spawn(move || worker.run());

        BlocklistLookup {
            request_tx: tx,
            inflight,
            cache,
            fallback_count,
            worker_requests,
            panics,
            budget,
        }
    }

    /// Look up `normalized` against the artifact under the bounded budget. `None` (no rule) and a
    /// cache miss that times out both resolve to the caller's default action.
    pub fn lookup(&self, normalized: &str) -> FilterAction {
        // Fast path: a cached decision avoids the channel and worker round trip entirely.
        {
            let mut cache = self.cache.lock().expect("cache mutex poisoned");
            if let Some(decision) = cache.get(normalized) {
                return decision.unwrap_or(FilterAction::Proxy);
            }
        }

        let (tx, rx) = std::sync::mpsc::channel();
        {
            let mut inflight = self.inflight.lock().expect("inflight mutex poisoned");
            match inflight.get_mut(normalized) {
                Some(waiters) => waiters.push(tx),
                None => {
                    inflight.insert(normalized.to_string(), vec![tx]);
                    drop(inflight);
                    // Enqueue the worker request without blocking the packet thread. A full queue
                    // or a dead worker means "no decision this round" — undo our in-flight entry and
                    // fall back, rather than leaving a waiter that can never be answered.
                    if self
                        .request_tx
                        .try_send(WorkerMsg::Lookup(normalized.to_string()))
                        .is_err()
                    {
                        self.inflight
                            .lock()
                            .expect("inflight mutex poisoned")
                            .remove(normalized);
                        self.note_fallback();
                        return FilterAction::Proxy;
                    }
                }
            }
        }

        match rx.recv_timeout(self.budget) {
            Ok(Some(action)) => action,
            Ok(None) => FilterAction::Proxy,
            Err(_) => {
                // Budget expired (or the worker was torn down): fall back to the last cached answer
                // for this domain if one exists, else the default — and count the event.
                self.note_fallback();
                self.cache
                    .lock()
                    .expect("cache mutex poisoned")
                    .get(normalized)
                    .map(|v| v.unwrap_or(FilterAction::Proxy))
                    .unwrap_or(FilterAction::Proxy)
            }
        }
    }

    fn note_fallback(&self) {
        self.fallback_count.fetch_add(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// A small fixed-capacity LRU
// ---------------------------------------------------------------------------

/// A minimal LRU over `domain → Option<FilterAction>` (the `None` value means "checked, no FST
/// rule"). Fixed capacity; `get`/`put` both move the key to most-recently-used. The eviction scan
/// is O(capacity) but the cache is deliberately tiny (hundreds of entries), so that cost is
/// acceptable and this stays dependency-free.
struct LruCache {
    cap: usize,
    map: HashMap<String, Option<FilterAction>>,
    order: VecDeque<String>,
}

impl LruCache {
    fn new(cap: usize) -> Self {
        LruCache {
            cap: cap.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns `Some(decision)` if the key is cached (the decision may itself be `None`), or `None`
    /// if the key is absent.
    fn get(&mut self, key: &str) -> Option<Option<FilterAction>> {
        let value = self.map.get(key).cloned()?;
        self.touch(key);
        Some(value)
    }

    fn put(&mut self, key: String, value: Option<FilterAction>) {
        if !self.map.contains_key(&key) {
            if self.map.len() == self.cap {
                if let Some(oldest) = self.order.pop_front() {
                    self.map.remove(&oldest);
                }
            }
            self.order.push_back(key.clone());
        }
        self.map.insert(key.clone(), value);
        self.touch(&key);
    }

    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos).unwrap();
            self.order.push_back(k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain_blocklist::{Category, LicenseId, MergedEntry, SourceId, SourceSnapshot};
    use domain_normalize::RuleScope;
    use ed25519_dalek::SigningKey;
    use std::path::Path;
    use tempfile::tempdir;

    fn merged(domain: &str, scope: RuleScope) -> MergedEntry {
        MergedEntry {
            domain: domain.to_string(),
            scope,
            sources: vec![SourceId::StevenBlack],
            categories: vec![Category::Adult],
        }
    }

    fn snapshots() -> Vec<SourceSnapshot> {
        Vec::new()
    }

    fn key(id: &str, seed: u8) -> (KeyId, SigningKey) {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        (KeyId(id.to_string()), SigningKey::from_bytes(&bytes))
    }

    fn verifying(key: &SigningKey) -> VerifyingKey {
        key.verifying_key()
    }

    /// Build an artifact from `entries` and write it into `dir`'s `slot` subdirectory in the
    /// two-slot layout. Returns the (signed) manifest for tampering tests.
    fn write_artifact(dir: &Path, slot: &str, entries: &[MergedEntry], version: u64) -> Manifest {
        let (kid, key) = key("k1", 1);
        let artifact = domain_blocklist::build(
            entries,
            LicenseId("MIT".to_string()),
            snapshots(),
            version,
            1_700_000_000,
            &[(kid, key)],
        )
        .unwrap();
        let d = dir.join(slot);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(ARTIFACT_FILE), &artifact.fst_bytes).unwrap();
        std::fs::write(d.join(MANIFEST_FILE), bincode::serialize(&artifact.manifest).unwrap())
            .unwrap();
        artifact.manifest
    }

    fn load(dir: &Path, id: &str, seed: u8) -> Result<BlocklistArtifact, ArtifactError> {
        let (kid, key) = key(id, seed);
        BlocklistArtifact::load(dir, &[(kid, verifying(&key))])
    }

    // --- BlocklistArtifact lookup: scope at label boundaries ---

    #[test]
    fn apex_covers_its_subdomains_exact_host_does_not() {
        let dir = tempdir().unwrap();
        // Only an ExactHost entry for www.example.com — no apex for example.com.
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("www.example.com", RuleScope::ExactHost)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();

        // www.example.com → com.example.www: the ExactHost rule matches the full key.
        assert_eq!(artifact.lookup(&reverse_key("www.example.com")), Some(FilterAction::Block));
        // The bare domain and a sibling subdomain are NOT covered by an ExactHost rule.
        assert_eq!(artifact.lookup(&reverse_key("example.com")), None);
        assert_eq!(artifact.lookup(&reverse_key("cdn.example.com")), None);
    }

    #[test]
    fn apex_rule_covers_everything_below_it() {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();

        for query in ["example.com", "www.example.com", "deep.sub.example.com"] {
            assert_eq!(
                artifact.lookup(&reverse_key(query)),
                Some(FilterAction::Block),
                "apex should cover {query}"
            );
        }
        assert_eq!(artifact.lookup(&reverse_key("example.org")), None);
    }

    #[test]
    fn lookup_never_matches_across_a_partial_label() {
        let dir = tempdir().unwrap();
        // example.com apex must never match examplezzz.com (which reverses to com.examplezzz).
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();
        assert_eq!(artifact.lookup(&reverse_key("examplezzz.com")), None);
    }

    #[test]
    fn provenance_for_returns_the_combination_for_an_exact_key() {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();
        let prov = artifact.provenance_for("com.example").expect("present");
        assert_eq!(prov.scope, RuleScope::Apex);
        assert_eq!(artifact.provenance_for("com.absent"), None);
    }

    // --- load: verification and slot fallback ---

    #[test]
    fn load_verifies_and_loads_current() {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 3);
        let artifact = load(dir.path(), "k1", 1).unwrap();
        assert_eq!(artifact.version(), 3);
        assert_eq!(artifact.lookup(&reverse_key("www.example.com")), Some(FilterAction::Block));
    }

    #[test]
    fn load_rejects_an_unknown_signer() {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        // Trust a different key than the one that signed.
        match load(dir.path(), "attacker", 99) {
            Err(ArtifactError::BadSignature) => {}
            Err(e) => panic!("expected BadSignature, got {e:?}"),
            Ok(_) => panic!("expected load to fail"),
        }
    }

    #[test]
    fn load_rejects_a_tampered_fst_digest() {
        let dir = tempdir().unwrap();
        // Keep the manifest (and its valid signature) intact, but corrupt the artifact bytes the
        // digest is supposed to cover. The signature still verifies; the digest comparison is what
        // catches the tampering.
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        let fst_path = dir.path().join(CURRENT_SLOT).join(ARTIFACT_FILE);
        let mut bytes = std::fs::read(&fst_path).unwrap();
        bytes[0] ^= 0xFF;
        std::fs::write(&fst_path, bytes).unwrap();
        match load(dir.path(), "k1", 1) {
            Err(ArtifactError::DigestMismatch) => {}
            Err(e) => panic!("expected DigestMismatch, got {e:?}"),
            Ok(_) => panic!("expected load to fail"),
        }
    }

    #[test]
    fn load_falls_back_to_previous_when_current_is_corrupt() {
        let dir = tempdir().unwrap();
        // Corrupt current: a manifest whose digest doesn't match the (kept) artifact bytes.
        let mut corrupt = write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 2);
        corrupt.fst_digest[0] ^= 0xFF;
        std::fs::write(
            dir.path().join(CURRENT_SLOT).join(MANIFEST_FILE),
            bincode::serialize(&corrupt).unwrap(),
        )
        .unwrap();
        // Valid previous.
        write_artifact(dir.path(), PREVIOUS_SLOT, &[merged("other.net", RuleScope::Apex)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();
        // The fallback slot's own version is what we loaded.
        assert_eq!(artifact.version(), 1);
        assert_eq!(artifact.lookup(&reverse_key("other.net")), Some(FilterAction::Block));
    }

    #[test]
    fn load_fails_closed_when_both_slots_are_unusable() {
        let dir = tempdir().unwrap();
        match load(dir.path(), "k1", 1) {
            Err(ArtifactError::SlotMissing) => {}
            Err(e) => panic!("expected SlotMissing, got {e:?}"),
            Ok(_) => panic!("expected load to fail"),
        }
    }

    // --- Allowlist ---

    #[test]
    fn allowlist_matches_a_domain_and_its_subdomains() {
        let allow = Allowlist::from_entries(&["example.com"]);
        assert!(allow.contains("example.com"));
        assert!(allow.contains("www.example.com"));
        assert!(allow.contains("deep.sub.example.com"));
        assert!(!allow.contains("example.org"));
        assert!(!allow.contains("badexample.com"));
    }

    // --- precedence resolution ---

    #[test]
    fn precedence_allowlist_beats_an_explicit_block() {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();
        let allow = Allowlist::from_entries(&["example.com"]);
        let explicit = DomainFilter::from_rules(&[("example.com", FilterAction::Block)]);
        assert_eq!(
            resolve_action("example.com", &allow, &explicit, Some(&artifact)),
            Some(FilterAction::Allow)
        );
    }

    #[test]
    fn precedence_explicit_proxy_beats_the_fst() {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();
        let explicit = DomainFilter::from_rules(&[("example.com", FilterAction::Proxy)]);
        assert_eq!(
            resolve_action("example.com", &Allowlist::new(), &explicit, Some(&artifact)),
            Some(FilterAction::Proxy)
        );
    }

    #[test]
    fn precedence_fst_blocks_when_no_explicit_rule() {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();
        assert_eq!(
            resolve_action("example.com", &Allowlist::new(), &DomainFilter::from_rules(&[]), Some(&artifact)),
            Some(FilterAction::Block)
        );
    }

    #[test]
    fn precedence_miss_returns_none_for_the_caller_default() {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        let artifact = load(dir.path(), "k1", 1).unwrap();
        assert_eq!(
            resolve_action("unlisted.org", &Allowlist::new(), &DomainFilter::from_rules(&[]), Some(&artifact)),
            None
        );
    }

    // --- BlocklistLookup: the lookup budget ---

    fn blocking_artifact() -> Arc<BlocklistArtifact> {
        let dir = tempdir().unwrap();
        write_artifact(dir.path(), CURRENT_SLOT, &[merged("example.com", RuleScope::Apex)], 1);
        Arc::new(load(dir.path(), "k1", 1).unwrap())
    }

    #[test]
    fn fast_worker_answers_within_budget() {
        let lookup = BlocklistLookup::new(blocking_artifact(), Duration::from_millis(50), 64);
        assert_eq!(lookup.lookup("example.com"), FilterAction::Block);
        assert_eq!(lookup.worker_requests_processed(), 1);
        assert_eq!(lookup.fallback_count(), 0);
    }

    #[test]
    fn fast_worker_caches_the_answer_for_the_next_query() {
        let lookup = BlocklistLookup::new(blocking_artifact(), Duration::from_millis(50), 64);
        assert_eq!(lookup.lookup("example.com"), FilterAction::Block);
        assert_eq!(lookup.worker_requests_processed(), 1);
        // Second query hits the LRU fast path — no new worker request.
        assert_eq!(lookup.lookup("example.com"), FilterAction::Block);
        assert_eq!(lookup.worker_requests_processed(), 1);
    }

    #[test]
    fn slow_worker_forces_the_timeout_fallback() {
        let lookup = BlocklistLookup::with_worker_delay(
            blocking_artifact(),
            Duration::from_millis(10),
            64,
            Duration::from_millis(200),
        );
        // Cold cache + a worker slower than the budget: default action, fallback counted.
        assert_eq!(lookup.lookup("example.com"), FilterAction::Proxy);
        assert_eq!(lookup.fallback_count(), 1);
        // The in-flight request is not cancelled: once the worker finishes it populates the cache,
        // so the next query (fast path) returns the real answer with no new worker request.
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(lookup.lookup("example.com"), FilterAction::Block);
        assert_eq!(lookup.worker_requests_processed(), 1);
    }

    #[test]
    fn two_concurrent_lookups_collapse_to_one_worker_request() {
        let lookup = Arc::new(BlocklistLookup::with_worker_delay(
            blocking_artifact(),
            Duration::from_millis(10),
            64,
            Duration::from_millis(200),
        ));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let lookup = lookup.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                lookup.lookup("example.com")
            }));
        }
        for h in handles {
            let _ = h.join().unwrap();
        }
        // Both attached to the same in-flight request — one worker round trip, not two.
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(lookup.worker_requests_processed(), 1);
        // And the eventual answer is now cached.
        assert_eq!(lookup.lookup("example.com"), FilterAction::Block);
    }

    // --- end-to-end: DnsShield through the artifact ---

    fn dns_query(name: &str) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.extend_from_slice(&0xbeefu16.to_be_bytes());
        msg.extend_from_slice(&0x0100u16.to_be_bytes());
        msg.extend_from_slice(&1u16.to_be_bytes());
        msg.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        for label in name.split('.') {
            msg.push(label.len() as u8);
            msg.extend_from_slice(label.as_bytes());
        }
        msg.push(0);
        msg.extend_from_slice(&1u16.to_be_bytes());
        msg.extend_from_slice(&1u16.to_be_bytes());
        msg
    }

    fn query_packet(name: &str) -> Vec<u8> {
        crate::udp::build_ipv4_udp(
            "10.111.0.2".parse().unwrap(),
            "10.111.0.1".parse().unwrap(),
            41234,
            crate::dns::PORT_DNS,
            &dns_query(name),
        )
        .unwrap()
    }

    fn artifact_shield() -> crate::dns_shield::DnsShield {
        let lookup = BlocklistLookup::new(blocking_artifact(), Duration::from_millis(50), 64);
        crate::dns_shield::DnsShield::with_policy(
            DomainFilter::from_rules(&[]),
            Allowlist::new(),
            Some(lookup),
        )
    }

    #[test]
    fn dns_shield_blocks_a_domain_in_the_artifact() {
        let shield = artifact_shield();
        assert!(matches!(
            shield.inspect(&query_packet("example.com")),
            crate::dns_shield::DnsVerdict::Blocked { .. }
        ));
        // A subdomain of an apex rule is blocked too.
        assert!(matches!(
            shield.inspect(&query_packet("www.example.com")),
            crate::dns_shield::DnsVerdict::Blocked { .. }
        ));
        // A domain with no rule is forwarded, not guessed.
        assert!(matches!(
            shield.inspect(&query_packet("unlisted.org")),
            crate::dns_shield::DnsVerdict::Forward { .. }
        ));
    }

    #[test]
    fn dns_shield_allowlist_wins_over_the_artifact() {
        let lookup = BlocklistLookup::new(blocking_artifact(), Duration::from_millis(50), 64);
        let allowlist = Allowlist::from_entries(&["example.com"]);
        let shield = crate::dns_shield::DnsShield::with_policy(
            DomainFilter::from_rules(&[]),
            allowlist,
            Some(lookup),
        );
        assert!(matches!(
            shield.inspect(&query_packet("example.com")),
            crate::dns_shield::DnsVerdict::Forward { .. }
        ));
    }

    #[test]
    fn dns_shield_explicit_rule_wins_over_the_artifact() {
        let lookup = BlocklistLookup::new(blocking_artifact(), Duration::from_millis(50), 64);
        let explicit = DomainFilter::from_rules(&[("example.com", FilterAction::Allow)]);
        let shield = crate::dns_shield::DnsShield::with_policy(
            explicit,
            Allowlist::new(),
            Some(lookup),
        );
        assert!(matches!(
            shield.inspect(&query_packet("example.com")),
            crate::dns_shield::DnsVerdict::Forward { .. }
        ));
    }
}
