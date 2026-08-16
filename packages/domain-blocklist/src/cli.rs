//! Command-line surface for the pipeline binary — flags only; the orchestration itself lives in
//! `main.rs`. See `docs/components/domain-blocklist/plan.md`'s `### 7. cli` section for what each
//! flag corresponds to in the plan's own language.

use std::path::PathBuf;

use anyhow::bail;
use clap::Parser;

/// Runs the domain-blocklist pipeline end to end: fetch every pinned source → license gate →
/// merge → canary → liveness-prune → build the signed FST → publish gates → sign → write output.
/// Intended to run on a monthly schedule from CI or a small persistent job runner — never on an
/// end-user device.
#[derive(Debug, Parser)]
#[command(name = "domain-blocklist", version, about)]
pub struct Cli {
    /// Build and run every gate, but do not write anything to `--output` and do not persist
    /// `--cache`. Still reads the previous published build (if any) so a dry run sees exactly the
    /// gate decisions a real run would.
    #[arg(long)]
    pub dry_run: bool,

    /// Read pre-fetched, already-parser-ready source bytes from this directory instead of hitting
    /// the network. Expects `stevenblack.txt`, `hagezi_nsfw.txt`, `ut1_adult.txt`,
    /// `ut1_gambling.txt`, `ut1_dating.txt` — plain bytes in each source's native parse format
    /// (hosts-file for StevenBlack, plain-domain-per-line for the rest), i.e. *already* what
    /// `fetch_source` would have handed the parser, not a raw tarball/API response. This is the
    /// plan's "fixture sources" flag — for deterministic tests, not a network-access workaround
    /// (every real source is live-fetchable, per the module's own assumption audit).
    #[arg(long)]
    pub fixture_dir: Option<PathBuf>,

    /// Base directory for the on-device two-slot artifact layout
    /// (`<output>/current/{artifact.fst,manifest.bin}`, mirrored under `<output>/previous/`).
    #[arg(long, default_value = "blocklist-output")]
    pub output: PathBuf,

    /// Persistent liveness cache file — a redb database (see `cache_store::CacheStore`), not the
    /// legacy single bincode blob. Real (non-dry, non-`--skip-liveness`) runs require this — "a
    /// sweep that cannot persist must not run," since losing verified `Dead` verdicts silently is
    /// how a dead domain re-enters a future signed artifact. There is no automatic converter from
    /// an old bincode `cache.bin` — per `cache_store`'s own doc comment, a deployment switching
    /// to this starts one cold sweep rather than migrating stale state.
    #[arg(long)]
    pub cache: Option<PathBuf>,

    /// Skip the DNS liveness sweep entirely and keep every merged entry as-is. Always implied by
    /// `--fixture-dir`/`--dry-run` unless the caller also has `net`-feature resolvers configured;
    /// exists as its own flag so a real fetch + gates run can still be exercised without a DNS
    /// dependency, e.g. in an environment with no outbound UDP/53.
    #[arg(long)]
    pub skip_liveness: bool,

    /// Plain domain-per-line negative control set (the plan names Tranco) for
    /// `false_positive_gate`. Required for a real, non-dry-run publish.
    #[arg(long)]
    pub control_set: Option<PathBuf>,

    /// Plain domain-per-line file of hits a human has already reviewed and confirmed are
    /// legitimately sensitive despite lacking cross-source corroboration.
    #[arg(long)]
    pub exclusions: Option<PathBuf>,

    /// Plain domain-per-line shared-hosting denylist (`classify_scope`'s backstop).
    #[arg(long)]
    pub denylist: Option<PathBuf>,

    /// SPDX license identifiers this build accepts. Repeatable. The default set below is a
    /// starting point that must be ratified by a human before a real publish, per the module's own
    /// draft notes — it is not this crate's licensing decision to make silently.
    #[arg(long)]
    pub allow_license: Vec<String>,

    /// `key_id:path-to-32-byte-seed-file`, repeatable. Every key here signs the published
    /// manifest — more than one during a rotation's overlap window.
    #[arg(long = "signing-key")]
    pub signing_keys: Vec<String>,

    /// `key_id:path-to-32-byte-verifying-key-file`, repeatable — the keys trusted when verifying
    /// the *previous* published build before comparing against it. Defaults to the verifying keys
    /// derived from `--signing-key` when omitted, so a single-key, non-rotating deployment needs no
    /// extra flags.
    #[arg(long = "trust-key")]
    pub trust_keys: Vec<String>,

    /// Monotonic manifest version for this build. Defaults to one past the previous published
    /// build's version (or `1` for a genuine first build).
    #[arg(long = "build-version")]
    pub build_version: Option<u64>,

    /// Primary resolver for the liveness sweep (`net` feature only). Never a filtering resolver —
    /// see the plan's module 3.
    #[arg(long, default_value = "1.1.1.1:53")]
    pub resolver1: String,

    /// Secondary, independently-operated resolver — corroboration requires the two to disagree
    /// independently, not share an upstream.
    #[arg(long, default_value = "8.8.8.8:53")]
    pub resolver2: String,

    /// Per-query DNS timeout, milliseconds.
    #[arg(long, default_value_t = 3000)]
    pub dns_timeout_ms: u64,

    /// Target domain-checks started per second during the sweep. NOTE: this is roughly half the
    /// plan's raw ~23 queries/sec budget, since each domain start costs ~2 queries per resolver on
    /// average (1 when A resolves and AAAA is skipped, 2 when it doesn't) and both resolvers are
    /// queried concurrently per domain — halving here keeps the *actual* per-resolver query rate
    /// close to the plan's intended budget instead of 2-4x over it.
    #[arg(long, default_value_t = 11.5)]
    pub qps: f64,

    /// Max concurrent in-flight domain checks.
    #[arg(long, default_value_t = 50)]
    pub concurrency: usize,

    /// Re-run the canary every this many domains during the sweep.
    #[arg(long, default_value_t = 2000)]
    pub canary_every: usize,

    /// Persist the liveness cache to `--cache` after roughly this many newly-checked domains, so a
    /// crash mid-sweep loses at most one checkpoint interval instead of the whole run. Rounds up
    /// to the next multiple of `--canary-every`, since a checkpoint only fires once that chunk's
    /// canary re-check has passed. `0` disables checkpointing. No-op under `--dry-run` or without
    /// `--cache`.
    #[arg(long, default_value_t = 10_000)]
    pub checkpoint_every: usize,

    /// Liveness cache TTL, seconds. Plan default: 3x the run cadence (~monthly), so ~90 days.
    #[arg(long, default_value_t = 90 * 24 * 60 * 60)]
    pub ttl_seconds: u64,

    /// Continuous-`Dead` quarantine window before a domain actually prunes, seconds. MUST be at
    /// least twice `ttl_seconds`, and this default is expressed as exactly `2 *` `ttl_seconds`'s
    /// own default so the shipped relationship always satisfies `validate()`'s invariant — an
    /// independently-picked number could drift below the floor and silently defeat the whole
    /// quarantine (see `liveness/cache.rs`'s hysteresis doc comment: the window must outlast the
    /// TTL so a `Dead` verdict is re-checked before it can prune).
    #[arg(long, default_value_t = 2 * 90 * 24 * 60 * 60)]
    pub quarantine_seconds: u64,

    /// Known-always-alive, definitely-non-adult control domains for the canary. Repeatable.
    #[arg(long = "canary-alive")]
    pub canary_alive: Vec<String>,

    /// Known-always-dead control domains for the canary (RFC 2606/6761 reserved names, plus
    /// ideally a fresh per-run nonce under a registrar-controlled zone — see
    /// `liveness::nonce_dead_control`). Repeatable.
    #[arg(long = "canary-dead")]
    pub canary_dead: Vec<String>,

    /// Shrinkage gate: max fraction of the previous build's entry count this build may drop.
    #[arg(long, default_value_t = 0.10)]
    pub max_shrinkage_pct: f64,
    /// Shrinkage gate: absolute floor below which a build fails regardless of percentage.
    #[arg(long, default_value_t = 0)]
    pub shrinkage_floor: u64,
    /// Growth gate: max fraction of the previous build's key set that may be newly added.
    #[arg(long, default_value_t = 0.10)]
    pub max_growth_pct: f64,
    /// False-positive gate: max fraction of the checked control set that may land in the
    /// uncorroborated, unreviewed review queue.
    #[arg(long, default_value_t = 0.005)]
    pub max_fp_rate: f64,
    /// False-positive gate: minimum checked control-set size, below which the gate fails outright.
    /// A `0` default would defeat the floor's whole purpose — the floor exists to catch a
    /// truncated/empty control-set fetch, and at `0` it can never fire. `1000` is a starting
    /// value (not a statistically-derived one), the plan's own placeholder pending a real
    /// judgment about how small a control-set sample can be before its measured false-positive
    /// rate is meaningless.
    #[arg(long, default_value_t = 1000)]
    pub min_control_size: usize,
    /// Size gate ceiling, bytes. Plan default: 32 MiB.
    #[arg(long, default_value_t = 32 * 1024 * 1024)]
    pub size_ceiling_bytes: u64,

    /// Operational testing only, never for a real publish: truncates the merged entry set to the
    /// first N (post-merge, pre-liveness) domains before the liveness sweep and every gate below
    /// it. Exists so a qps ramp or a canary check can be exercised against a small slice of a
    /// *real* fetched corpus without waiting out a full-corpus sweep — `--fixture-dir` cannot
    /// serve this purpose, since it deliberately forces `--skip-liveness` (see
    /// `effective_skip_liveness`'s doc comment) and this flag's whole point is to reach the real
    /// DNS sweep. Every gate still runs and can still fail (most obviously `shrinkage_gate`
    /// against a real previous build) — that is intentional signal, not a bug to suppress.
    #[arg(long)]
    pub sample: Option<usize>,
}

impl Cli {
    /// Cross-field validation that `clap`'s per-flag constraints cannot express. Called from
    /// `main.rs`'s `run()` before any I/O so every later function can assume validated input.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.quarantine_seconds < 2 * self.ttl_seconds {
            bail!(
                "--quarantine-seconds ({}) must be at least twice --ttl-seconds ({}) — otherwise a \
                 domain's Dead verdict can be pruned before the next sweep ever gets a chance to \
                 re-check and confirm it, defeating the quarantine window's whole purpose (see \
                 liveness/cache.rs's hysteresis doc comment)",
                self.quarantine_seconds,
                self.ttl_seconds
            );
        }
        Ok(())
    }

    /// The value `main.rs` must use in place of the raw `skip_liveness` flag: liveness is skipped
    /// whenever the caller asked for it explicitly, or whenever `--fixture-dir` is set — a fixture
    /// run is documented as deterministic and offline, so it must never open UDP/53 to
    /// `--resolver1`/`--resolver2` regardless of whether the caller remembered `--skip-liveness`.
    pub fn effective_skip_liveness(&self) -> bool {
        self.skip_liveness || self.fixture_dir.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `Cli` with the two flags `validate()` cares about, everything else at clap's
    /// defaults. Parsing through `clap` (rather than a hand-constructed struct literal) keeps the
    /// test independent of the struct's 27 fields — any added flag with a default still parses.
    fn cli_with(ttl_seconds: u64, quarantine_seconds: u64) -> Cli {
        Cli::try_parse_from([
            "domain-blocklist",
            "--ttl-seconds",
            &ttl_seconds.to_string(),
            "--quarantine-seconds",
            &quarantine_seconds.to_string(),
        ])
        .expect("clap parse of a known-good flag set cannot fail")
    }

    /// A `Cli` with every flag at its shipped default.
    fn default_cli() -> Cli {
        Cli::try_parse_from(["domain-blocklist"]).expect("the default flag set must parse")
    }

    #[test]
    fn old_shipped_defaults_90d_ttl_35d_quarantine_now_fail_validate() {
        // The pre-fix shipped relationship (90-day TTL, 35-day quarantine) is exactly the
        // misconfiguration `validate` exists to catch: the quarantine window elapses before the
        // TTL ever schedules a re-check, so a transient `Dead` could prune without confirmation.
        let cli = cli_with(90 * 24 * 60 * 60, 35 * 24 * 60 * 60);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn quarantine_exactly_twice_ttl_is_valid() {
        // Boundary: `quarantine < 2 * ttl` is the failure line; equality must pass.
        let cli = cli_with(30 * 24 * 60 * 60, 60 * 24 * 60 * 60);
        assert!(cli.validate().is_ok());
    }

    #[test]
    fn quarantine_below_twice_ttl_is_invalid() {
        let cli = cli_with(30 * 24 * 60 * 60, 30 * 24 * 60 * 60);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn final_shipped_defaults_satisfy_validate() {
        // Post-fix shipped defaults: ttl 90d, quarantine 180d (== 2 * ttl), so a fresh,
        // un-overridden run passes its own new invariant.
        default_cli()
            .validate()
            .expect("shipped defaults must satisfy validate");
    }

    #[test]
    fn effective_skip_liveness_false_by_default() {
        assert!(!default_cli().effective_skip_liveness());
    }

    #[test]
    fn effective_skip_liveness_true_when_flag_set() {
        let cli = Cli::try_parse_from(["domain-blocklist", "--skip-liveness"])
            .expect("--skip-liveness must parse");
        assert!(cli.effective_skip_liveness());
    }

    #[test]
    fn effective_skip_liveness_true_when_fixture_dir_set_without_the_flag() {
        // The documented implication: `--fixture-dir` alone must imply skip, since a fixture run
        // is supposed to be deterministic and offline regardless of whether the caller also
        // remembered `--skip-liveness`.
        let cli = Cli::try_parse_from(["domain-blocklist", "--fixture-dir", "/tmp/fixtures"])
            .expect("--fixture-dir must parse");
        assert!(cli.effective_skip_liveness());
    }
}
