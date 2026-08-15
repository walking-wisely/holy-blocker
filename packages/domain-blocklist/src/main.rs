//! Module 7 of the domain-blocklist plan — the pipeline entry point. One binary that runs the
//! whole pipeline end to end: fetch every pinned source → license gate → merge → canary →
//! liveness-prune → build the signed FST → publish gates → sign → write output.
//!
//! See `docs/components/domain-blocklist/plan.md`'s `### 7. cli` section (including its two
//! assumption-audit tables) for the design this implements, and `docs/decisions/
//! domain-blocklist-sourcing.md` for the pipeline this is the last stage of.

#[cfg_attr(not(feature = "net"), allow(dead_code))]
mod cache_store;
mod cli;
mod publish_policy;
mod slots;
#[cfg(feature = "net")]
mod sweep;

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::Parser;
use ed25519_dalek::{SigningKey, VerifyingKey};

use domain_blocklist::fetchers::github::{GithubSourceFetcher, ReqwestHttp, StaticGithubRepos};
use domain_blocklist::fetchers::ut1::{Adult, Dating, Gambling, Ut1RetryPolicy, Ut1SourceFetcher};
use domain_blocklist::sources::{hagezi, stevenblack, ut1 as ut1_parser};
use domain_blocklist::{
    Category, FalsePositiveHit, FetchError, FetchedSource, GateResult, KeyId, LicenseId,
    MergedEntry, PreviousBuild, RawEntry, SourceConfig, SourceFetcher, SourceId, SourceSnapshot,
    build, fetch_source, flag_personal_name, merge_entries, review_queue,
};

fn main() {
    // A monthly CI run with no RUST_LOG must not silently suppress every fail-open warning this
    // binary emits — so the default level is `info`, not the `error` `EnvFilter` falls back to
    // when RUST_LOG is unset.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = cli::Cli::parse();

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("domain-blocklist: failed to start async runtime: {e}");
            std::process::exit(1);
        }
    };

    match rt.block_on(run(cli)) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("domain-blocklist: {e:#}");
            std::process::exit(1);
        }
    }
}

/// A [`SourceFetcher`] whose answer is already known — used to hand [`fetch_source`] (module 1's
/// pin/license validation, which this pipeline must not reimplement) a result this binary already
/// obtained, whether from a live async fetch (driven outside `fetch_source`'s synchronous trait
/// method, since the real fetchers' sync bridges panic if `block_on`'d from inside this binary's
/// own already-running async context — see `fetchers::github`/`fetchers::ut1`'s own doc comments)
/// or from a fixture file. `.fetch()` performs no I/O of its own, so wrapping a result this way is
/// always safe to call from any context.
struct AlreadyFetched(Result<FetchedSource, FetchError>);

impl SourceFetcher for AlreadyFetched {
    fn fetch(&self, _config: &SourceConfig) -> Result<FetchedSource, FetchError> {
        self.0.clone()
    }
}

fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads a plain domain-per-line file: blank lines and `#`-prefixed comments skipped, everything
/// else trimmed and kept verbatim (normalization happens downstream, once, in `merge`/`gates`).
fn read_lines(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect())
}

/// `key_id:path-to-32-byte-file` — shared parsing for both `--signing-key` and `--trust-key`.
fn parse_key_arg(arg: &str) -> Result<(String, [u8; 32])> {
    let (id, path) = arg
        .split_once(':')
        .with_context(|| format!("expected `key_id:path`, got {arg:?}"))?;
    let bytes = std::fs::read(path).with_context(|| format!("failed to read key file {path}"))?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("key file {path} is {} bytes, expected exactly 32", v.len()))?;
    Ok((id.to_string(), key))
}

fn load_signing_keys(args: &[String]) -> Result<Vec<(KeyId, SigningKey)>> {
    if args.is_empty() {
        bail!(
            "at least one --signing-key key_id:path is required — a build with no signer must \
             never publish (fst_build::build refuses this too, but failing here names the \
             specific missing flag)"
        );
    }
    args.iter()
        .map(|arg| {
            let (id, bytes) = parse_key_arg(arg)?;
            Ok((KeyId(id), SigningKey::from_bytes(&bytes)))
        })
        .collect()
}

fn load_trust_keys(
    trust_args: &[String],
    signing_keys: &[(KeyId, SigningKey)],
) -> Result<Vec<(KeyId, VerifyingKey)>> {
    if trust_args.is_empty() {
        // Default: trust exactly the keys we're signing with — the common, non-rotating case.
        return Ok(signing_keys
            .iter()
            .map(|(id, key)| (id.clone(), key.verifying_key()))
            .collect());
    }
    trust_args
        .iter()
        .map(|arg| {
            let (id, bytes) = parse_key_arg(arg)?;
            let vk = VerifyingKey::from_bytes(&bytes)
                .map_err(|e| anyhow::anyhow!("invalid verifying key in {arg}: {e}"))?;
            Ok((KeyId(id), vk))
        })
        .collect()
}

/// One thing this pipeline must fetch and parse: which [`SourceId`]/[`Category`] it produces and
/// how its bytes are parsed. Fixture files are named `<fixture_name>.txt`.
struct SourceJob {
    fixture_name: &'static str,
    config: SourceConfig,
    category: Category,
    parse: fn(&[u8], Category) -> domain_blocklist::ParseOutput,
}

/// Every source this build pulls from, per the plan's module 1/7. **Every `pinned_revision` below
/// is a placeholder** (`UNPINNED`) — a real deployment moves these only through the reviewed
/// pin-bump process the plan requires, never by editing this file casually. Left as a placeholder
/// rather than a guessed real SHA/ETag so a real (non-fixture) fetch fails *loudly and closed*
/// (`FetchError::RevisionMismatch` against whatever the source actually serves) instead of quietly
/// trusting an unreviewed pin.
fn default_source_jobs() -> Vec<SourceJob> {
    vec![
        SourceJob {
            fixture_name: "stevenblack",
            config: SourceConfig {
                source: SourceId::StevenBlack,
                url: "https://raw.githubusercontent.com/StevenBlack/hosts/UNPINNED/alternates/porn-only/hosts".to_string(),
                pinned_revision: "UNPINNED".to_string(),
                expected_license: LicenseId("MIT".to_string()),
            },
            category: Category::Adult,
            parse: |bytes, _category| stevenblack::parse(bytes),
        },
        SourceJob {
            fixture_name: "hagezi_nsfw",
            config: SourceConfig {
                source: SourceId::Hagezi,
                url: "https://raw.githubusercontent.com/hagezi/dns-blocklists/UNPINNED/wildcard/nsfw-onlydomains.txt".to_string(),
                pinned_revision: "UNPINNED".to_string(),
                expected_license: LicenseId("GPL-3.0".to_string()),
            },
            category: Category::Adult,
            parse: |bytes, _category| hagezi::parse(bytes),
        },
        SourceJob {
            fixture_name: "ut1_adult",
            config: SourceConfig {
                source: SourceId::Ut1,
                url: "https://dsi.ut-capitole.fr/blacklists/download/adult.tar.gz".to_string(),
                pinned_revision: "UNPINNED".to_string(),
                expected_license: LicenseId("CC-BY-SA-4.0".to_string()),
            },
            category: Category::Adult,
            parse: ut1_parser::parse_domains,
        },
        SourceJob {
            fixture_name: "ut1_gambling",
            config: SourceConfig {
                source: SourceId::Ut1,
                url: "https://dsi.ut-capitole.fr/blacklists/download/gambling.tar.gz".to_string(),
                pinned_revision: "UNPINNED".to_string(),
                expected_license: LicenseId("CC-BY-SA-4.0".to_string()),
            },
            category: Category::Gambling,
            parse: ut1_parser::parse_domains,
        },
        SourceJob {
            fixture_name: "ut1_dating",
            config: SourceConfig {
                source: SourceId::Ut1,
                url: "https://dsi.ut-capitole.fr/blacklists/download/dating.tar.gz".to_string(),
                pinned_revision: "UNPINNED".to_string(),
                expected_license: LicenseId("CC-BY-SA-4.0".to_string()),
            },
            category: Category::Dating,
            parse: ut1_parser::parse_domains,
        },
    ]
}

/// Fetches and validates one configured source (fixture file if `fixture_dir` is set, otherwise
/// the real GitHub/UT1 fetcher), returning its entries, the category it was fetched as, and the
/// validated snapshot. A failed fetch aborts through `fetch_source`, per the plan's module 1 ("not
/// a warning, not a skip").
///
/// Fixture-mode caveat (accepted test-fixture design, previously undocumented): under `--fixture-dir`
/// the revision and license are synthesized directly from the `SourceConfig` being validated
/// against, so `fetch_source`'s `RevisionMismatch`/`LicenseChanged`/`LicenseNotAllowed` checks can
/// *never* fire. Fixtures are for deterministic parse/merge/gate tests, not a substitute for
/// exercising the pin/license validation path — a real `RevisionMismatch`/`LicenseChanged`
/// regression can only be caught by a real, non-fixture run (or, if a future test wants
/// fixture-mode coverage of the gate itself, by a fixture that deliberately sets
/// `pinned_revision`/`expected_license` to something the fixture bytes DO NOT match — noted as the
/// way to do it later, not built here).
async fn fetch_one_source(
    job: &SourceJob,
    fixture_dir: Option<&Path>,
    allowlist: &[LicenseId],
    github_fetcher: Option<&GithubSourceFetcher<ReqwestHttp, StaticGithubRepos>>,
) -> Result<(Vec<RawEntry>, Category, SourceSnapshot)> {
    let fetched_result: Result<FetchedSource, FetchError> = if let Some(dir) = fixture_dir {
        let path = dir.join(format!("{}.txt", job.fixture_name));
        std::fs::read(&path)
            .map(|bytes| FetchedSource {
                revision: job.config.pinned_revision.clone(),
                license: job.config.expected_license.clone(),
                fetched_at: now_timestamp(),
                bytes,
            })
            .map_err(|e| FetchError::Transport(format!("fixture file {}: {e}", path.display())))
    } else {
        match job.config.source {
            SourceId::StevenBlack | SourceId::Hagezi => github_fetcher
                .expect("github_fetcher built whenever fixture_dir is None")
                .fetch_async(&job.config)
                .await
                .map_err(FetchError::from),
            SourceId::Ut1 => fetch_ut1(job).await,
        }
    };

    let wrapper = AlreadyFetched(fetched_result);
    let (snapshot, bytes) = fetch_source(&wrapper, &job.config, allowlist).map_err(|e| {
        anyhow::anyhow!(
            "fetch/validate failed for {:?} ({}): {e:?}",
            job.config.source,
            job.fixture_name
        )
    })?;

    let parsed = (job.parse)(&bytes, job.category);
    tracing::info!(
        source = job.fixture_name,
        entries = parsed.entries.len(),
        skipped_comment_or_blank = parsed.report.skipped_comment_or_blank,
        dropped_malformed = parsed.report.dropped_malformed,
        dropped_ip_literal = parsed.report.dropped_ip_literal,
        wildcard_count = parsed.report.wildcard_count,
        "parsed source"
    );
    Ok((parsed.entries, job.category, snapshot))
}

/// Fetches every configured source concurrently, validating each through `fetch_source` — a
/// failed fetch aborts the whole build, per the plan's module 1 ("not a warning, not a skip").
/// Five independent sources, each doing 1-3 sequential HTTP round trips, run as parallel futures
/// instead of a serial ~12-round-trip chain. Each snapshot is returned tagged with the category it
/// was fetched as, so `collapse_snapshots_by_source` can keep per-category revision provenance.
async fn fetch_all_sources(
    jobs: &[SourceJob],
    fixture_dir: Option<&Path>,
    allowlist: &[LicenseId],
) -> Result<(Vec<RawEntry>, Vec<(Category, SourceSnapshot)>)> {
    let github_fetcher = if fixture_dir.is_none() {
        Some(
            GithubSourceFetcher::new(
                ReqwestHttp::new().context("failed to build the GitHub HTTP client")?,
                StaticGithubRepos::the_two_github_sources(),
            )
            .context("failed to start the GitHub fetcher's runtime")?,
        )
    } else {
        None
    };

    // All the sources are independent — concurrency is a latency win, not a correctness change:
    // a failed future still aborts the whole `try_join_all` (and hence the build), same as a
    // serial loop would.
    let results = futures::future::try_join_all(
        jobs.iter()
            .map(|job| fetch_one_source(job, fixture_dir, allowlist, github_fetcher.as_ref())),
    )
    .await?;

    let mut raw_entries = Vec::new();
    let mut snapshots = Vec::new();
    for (entries, category, snapshot) in results {
        raw_entries.extend(entries);
        snapshots.push((category, snapshot));
    }
    Ok((raw_entries, snapshots))
}

async fn fetch_ut1(job: &SourceJob) -> Result<FetchedSource, FetchError> {
    let retry = Ut1RetryPolicy::default();
    let connect = std::time::Duration::from_secs(15);
    let request = std::time::Duration::from_secs(120);
    match job.category {
        Category::Adult => {
            Ut1SourceFetcher::<Adult>::new(retry, connect, request)
                .fetch_files(&job.config)
                .await
                .map(|f| f.fetched)
        }
        Category::Gambling => {
            Ut1SourceFetcher::<Gambling>::new(retry, connect, request)
                .fetch_files(&job.config)
                .await
                .map(|f| f.fetched)
        }
        Category::Dating => {
            Ut1SourceFetcher::<Dating>::new(retry, connect, request)
                .fetch_files(&job.config)
                .await
                .map(|f| f.fetched)
        }
    }
}

async fn run(cli: cli::Cli) -> Result<()> {
    // Cross-field validation first, before any I/O, so every function below can assume the flags
    // are consistent with each other (not just individually well-formed).
    cli.validate()?;

    let signing_keys = load_signing_keys(&cli.signing_keys)?;
    let trust_keys = load_trust_keys(&cli.trust_keys, &signing_keys)?;

    let allowlist: Vec<LicenseId> = if cli.allow_license.is_empty() {
        tracing::warn!(
            "no --allow-license given; using an unratified default set (MIT, CC0-1.0, GPL-3.0, \
             CC-BY-SA-4.0) — a real deployment must ratify this list explicitly, per the module's \
             own draft notes"
        );
        ["MIT", "CC0-1.0", "GPL-3.0", "CC-BY-SA-4.0"]
            .iter()
            .map(|s| LicenseId(s.to_string()))
            .collect()
    } else {
        cli.allow_license.iter().map(|s| LicenseId(s.clone())).collect()
    };

    let denylist_owned = cli.denylist.as_deref().map(read_lines).transpose()?.unwrap_or_default();
    let denylist_refs: Vec<&str> = denylist_owned.iter().map(String::as_str).collect();

    let jobs = default_source_jobs();
    let (raw_entries, snapshots) =
        fetch_all_sources(&jobs, cli.fixture_dir.as_deref(), &allowlist).await?;
    let snapshots = publish_policy::collapse_snapshots_by_source(snapshots)?;

    let merge_output = merge_entries(&raw_entries, &denylist_refs);
    tracing::info!(
        merged = merge_output.entries.len(),
        dropped_normalization_failed = merge_output.report.dropped_normalization_failed,
        dropped_public_suffix_or_denylisted = merge_output.report.dropped_public_suffix_or_denylisted,
        dropped_ip_literal = merge_output.report.dropped_ip_literal,
        "merged sources"
    );

    // Personal-name review triage (module 2) — reported, never gated on.
    let personal_name_candidates: Vec<String> = merge_output
        .entries
        .iter()
        .filter_map(|e| {
            let label = e.domain.split('.').next()?;
            flag_personal_name(label).then(|| e.domain.clone())
        })
        .collect();
    if !personal_name_candidates.is_empty() {
        tracing::info!(
            count = personal_name_candidates.len(),
            "personal-name review-queue candidates flagged (see output review-queue file)"
        );
    }

    let mut entries = merge_output.entries;

    // --- Liveness sweep -----------------------------------------------------------------------
    // `None` when liveness was skipped entirely — there is no sweep data to report in that case,
    // and `write_negative_outcome_report` writes nothing rather than an empty (and misleadingly
    // "all clean") file.
    let mut negative_report: Option<domain_blocklist::NegativeOutcomeReport> = None;
    if !cli.effective_skip_liveness() {
        let (kept, report) = run_liveness(&cli, entries).await?;
        entries = kept;
        negative_report = Some(report);
    } else {
        tracing::warn!("liveness skipped (--skip-liveness or --fixture-dir): every merged entry is kept without a DNS check");
    }

    let new_keys: BTreeSet<String> = entries.iter().map(|e| e.domain.clone()).collect();

    // --- Load and verify the previous published build ------------------------------------------
    let previous = slots::load_previous(&cli.output, &trust_keys)?;
    let (prev_count, prev_keys, next_version) = match &previous {
        slots::PreviousLoad::NoPreviousBuild => (PreviousBuild::None, PreviousBuild::None, 1u64),
        slots::PreviousLoad::Existing { manifest, domains } => (
            PreviousBuild::Existing(manifest.entry_count),
            PreviousBuild::Existing(domains),
            manifest.version + 1,
        ),
    };
    let version = cli.build_version.unwrap_or(next_version);
    if let slots::PreviousLoad::Existing { manifest, .. } = &previous
        && version <= manifest.version
    {
        bail!(
            "requested version {version} does not exceed the previous published version {} — \
             refusing to publish a rollback",
            manifest.version
        );
    }

    // --- Gates that don't need the built artifact ----------------------------------------------
    let mut gate_report: Vec<(&'static str, GateResult)> = Vec::new();
    gate_report.push((
        "shrinkage",
        domain_blocklist::shrinkage_gate(prev_count, entries.len() as u64, cli.max_shrinkage_pct, cli.shrinkage_floor),
    ));
    gate_report.push((
        "growth",
        domain_blocklist::growth_gate(prev_keys, &new_keys, cli.max_growth_pct),
    ));

    let output_license = publish_policy::pick_output_license(&snapshots);
    if output_license.spdx_matches(&LicenseId("CC-BY-SA-4.0".to_string()))
        || output_license.spdx_matches(&LicenseId("GPL-3.0".to_string()))
    {
        tracing::warn!(
            license = ?output_license,
            "pick_output_license ranked `{output_license:?}` as the build's output license; its \
             relative ranking (juxtaposed against the other copyleft license) is unreviewed and \
             MUST be ratified by a qualified reviewer before this manifest field drives \
             redistribution decisions"
        );
    }
    gate_report.push((
        "license",
        domain_blocklist::license_gate(&entries, &snapshots, &allowlist),
    ));

    let control_set_owned = cli.control_set.as_deref().map(read_lines).transpose()?.unwrap_or_default();
    let control_set_refs: Vec<&str> = control_set_owned.iter().map(String::as_str).collect();
    let exclusions_owned = cli.exclusions.as_deref().map(read_lines).transpose()?.unwrap_or_default();
    let exclusions_refs: Vec<&str> = exclusions_owned.iter().map(String::as_str).collect();
    if control_set_owned.is_empty() {
        tracing::warn!("no --control-set given; the false-positive gate is effectively disabled");
        if !cli.dry_run {
            bail!(
                "refusing a real (non-dry-run) publish with no --control-set: the false-positive gate \
                 cannot check anything against an empty control set, which is the last defence against \
                 black-holing a mainstream site — pass --control-set, or add --dry-run if this run is \
                 intentionally exploratory"
            );
        }
    }
    gate_report.push((
        "false_positive",
        domain_blocklist::false_positive_gate(
            &entries,
            &control_set_refs,
            &exclusions_refs,
            cli.max_fp_rate,
            cli.min_control_size,
        ),
    ));

    let review: Vec<FalsePositiveHit> = review_queue(&entries, &control_set_refs, &exclusions_refs);

    // --- Build the artifact (needed for the size gate and, if everything passes, publish) ------
    let artifact = build(&entries, output_license, snapshots, version, now_timestamp(), &signing_keys)
        .context("failed to build the FST artifact")?;
    tracing::info!(
        entry_count = artifact.report.entry_count,
        fst_bytes = artifact.report.fst_bytes,
        bytes_per_entry = artifact.report.bytes_per_entry,
        provenance_combos = artifact.report.provenance_combos,
        "built artifact"
    );
    gate_report.push((
        "size",
        domain_blocklist::size_gate(artifact.report.fst_bytes, cli.size_ceiling_bytes),
    ));

    let mut failed = false;
    for (name, result) in &gate_report {
        match result {
            GateResult::Pass => tracing::info!(gate = name, "gate passed"),
            GateResult::Fail(reason) => {
                failed = true;
                tracing::error!(gate = name, reason, "gate FAILED");
            }
        }
    }

    if failed {
        // A gate failure still writes the review queue and the negative-outcome report: both are
        // useful triage input even when the build itself did not pass, and `--dry-run` (not gate
        // failure) is what actually gates whether anything is written to `--output`.
        write_review_queue(&cli.output, &personal_name_candidates, &review)?;
        write_negative_outcome_report(&cli.output, &negative_report)?;
        bail!("one or more publish gates failed — see the gate log above; nothing was published");
    }

    if cli.dry_run {
        tracing::info!(
            review_queue_entries = review.len(),
            personal_name_candidates = personal_name_candidates.len(),
            "--dry-run set: all gates passed, nothing was written to --output"
        );
        return Ok(());
    }

    write_review_queue(&cli.output, &personal_name_candidates, &review)?;
    write_negative_outcome_report(&cli.output, &negative_report)?;
    slots::publish(&cli.output, &artifact).context("failed to publish the artifact")?;
    tracing::info!(version, base = %cli.output.display(), "published");
    Ok(())
}

#[cfg(feature = "net")]
async fn run_liveness(
    cli: &cli::Cli,
    entries: Vec<MergedEntry>,
) -> Result<(Vec<MergedEntry>, domain_blocklist::NegativeOutcomeReport)> {
    let cache = match &cli.cache {
        Some(path) => cache_store::load(path)?,
        None => {
            if !cli.dry_run {
                bail!(
                    "refusing a real (non-dry-run) liveness sweep with no --cache: a sweep that \
                     cannot persist loses every verified Dead verdict, so the quarantine window \
                     can never elapse and no domain is ever pruned — pass --cache, or add \
                     --dry-run/--skip-liveness"
                );
            }
            tracing::warn!("no --cache given: the liveness sweep runs with no prior state and its results are not persisted");
            Default::default()
        }
    };

    if cli.canary_alive.is_empty() || cli.canary_dead.is_empty() {
        bail!(
            "the liveness sweep needs at least one --canary-alive and one --canary-dead control \
             domain — a canary with an empty control list can never guard the sweep, per liveness::CanaryConfig::new"
        );
    }
    let canary = domain_blocklist::CanaryConfig::new(cli.canary_alive.clone(), cli.canary_dead.clone())
        .map_err(|e| anyhow::anyhow!("invalid canary config: {e:?}"))?;

    let sweep_config = sweep::SweepConfig {
        primary: domain_blocklist::liveness::ResolverConfig {
            addr: cli.resolver1.parse().context("invalid --resolver1")?,
            timeout: std::time::Duration::from_millis(cli.dns_timeout_ms),
        },
        secondary: domain_blocklist::liveness::ResolverConfig {
            addr: cli.resolver2.parse().context("invalid --resolver2")?,
            timeout: std::time::Duration::from_millis(cli.dns_timeout_ms),
        },
        qps: cli.qps,
        concurrency: cli.concurrency,
        canary_every: cli.canary_every,
        ttl_seconds: cli.ttl_seconds,
        quarantine_seconds: cli.quarantine_seconds,
    };

    let now = now_timestamp();
    let outcome = sweep::run_sweep(&entries, cache, &canary, &sweep_config, now)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let negative_report = domain_blocklist::negative_outcome_report(&outcome.cache);
    tracing::info!(
        checked = outcome.checked_count,
        pruned = outcome.pruned_domains.len(),
        dead = negative_report.dead.len(),
        unknown = negative_report.total() - negative_report.dead.len(),
        "liveness sweep complete"
    );

    if let Some(path) = &cli.cache
        && !cli.dry_run
    {
        cache_store::save(path, &outcome.cache)?;
    }

    let kept = entries
        .into_iter()
        .filter(|e| !outcome.pruned_domains.contains(&e.domain))
        .collect();
    Ok((kept, negative_report))
}

#[cfg(not(feature = "net"))]
async fn run_liveness(
    _cli: &cli::Cli,
    _entries: Vec<MergedEntry>,
) -> Result<(Vec<MergedEntry>, domain_blocklist::NegativeOutcomeReport)> {
    bail!(
        "a liveness sweep was requested but this binary was built without the `net` feature — \
         rebuild with `--features net`, or pass --skip-liveness to keep every merged entry unchecked"
    );
}

fn write_review_queue(
    output: &Path,
    personal_names: &[String],
    false_positives: &[FalsePositiveHit],
) -> Result<()> {
    std::fs::create_dir_all(output).with_context(|| format!("failed to create {}", output.display()))?;

    let mut personal = String::new();
    for domain in personal_names {
        personal.push_str(domain);
        personal.push('\n');
    }
    std::fs::write(output.join("review-queue-personal-names.txt"), personal)?;

    let mut fp = String::new();
    for hit in false_positives {
        fp.push_str(&format!("{}\t{:?}\t{:?}\n", hit.domain, hit.scope, hit.sources));
    }
    std::fs::write(output.join("review-queue-false-positives.txt"), fp)?;
    Ok(())
}

/// Writes every domain a liveness sweep placed in a negative category — `dead\t<domain>` for
/// `Dead`, `unknown:<reason>\t<domain>` for each distinct `Unknown` reason — to
/// `liveness-negative-outcomes.txt`, one line per domain, sorted for reproducible diffs across
/// runs. Writes nothing when `report` is `None` (liveness was skipped: `--skip-liveness` or
/// `--fixture-dir`), since an absent file correctly reads as "no sweep data", where an empty file
/// would misleadingly read as "swept clean, zero negatives".
fn write_negative_outcome_report(
    output: &Path,
    report: &Option<domain_blocklist::NegativeOutcomeReport>,
) -> Result<()> {
    let Some(report) = report else {
        return Ok(());
    };
    std::fs::create_dir_all(output).with_context(|| format!("failed to create {}", output.display()))?;

    let mut lines = String::new();
    for domain in &report.dead {
        lines.push_str(&format!("dead\t{domain}\n"));
    }
    for (reason, domains) in &report.unknown_by_reason {
        for domain in domains {
            lines.push_str(&format!("unknown:{reason}\t{domain}\n"));
        }
    }
    std::fs::write(output.join("liveness-negative-outcomes.txt"), lines)?;
    Ok(())
}
