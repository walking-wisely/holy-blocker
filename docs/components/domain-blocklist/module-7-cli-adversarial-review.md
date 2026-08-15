# Module 7 (`cli`) — adversarial review findings

**Pre-fix snapshot.** Recorded 2026-08-15, against the uncommitted working tree of
`feat/domain-blocklist-netshield` after module 7's first implementation pass — this file is the
record of what an Opus adversarial review pass found at that moment, scoped per the user's request
to transient error handling, overall error handling/tracing, and performance/parallelization. It is
kept as-written below (findings text unedited); do not read it as the current state of the code. The
two Critical findings (§1 the `Runtime`-dropped-inside-async-context panic, §2 the DNS client's
missing retries) and the unintended `net-shield`-on-full-HTTP-stack dependency were fixed in this
same branch's later commits — see
[the plan's module 7 disposition note](plan.md#7-cli--the-pipeline-entry-point) for what changed and
what a fresh review would need to re-verify. Findings below this point were not individually
re-audited when that disposition note was written; treat any one of them as still open until
checked against current code. See
[the plan's module 7 section](plan.md#7-cli--the-pipeline-entry-point) for the behavioral contract
this code is supposed to satisfy, and
[domain-blocklist-sourcing.md](../../decisions/domain-blocklist-sourcing.md) for the pipeline
design.

**Reproduction summary:** 3 findings reproduced by executing isolated code written for this review
(not branch code); the rest verified by reading the exact functions cited. No live network run was
performed — finding 1 explains why the implementing agent's own verification (all fixture-mode)
could not have caught the top issue.

## Critical

### 1. Every live fetch and every DNS sweep panics on completion (`Runtime` dropped inside an async context)

`GithubSourceFetcher` holds `rt: tokio::runtime::Runtime` (`fetchers/github.rs:512`) and
`HickoryDnsLookup` holds `rt: tokio::runtime::Runtime` (`liveness/net.rs:92`). Both are
**constructed and dropped inside `async fn` bodies driven by `main.rs`'s multi-thread runtime**:

- `main.rs:225-235` — `let github_fetcher = Some(GithubSourceFetcher::new(...))` inside
  `async fn fetch_all_sources`, dropped at `main.rs:286`.
- `sweep.rs:161-162` — `Arc::new(HickoryDnsLookup::new(...))` inside `pub async fn run_sweep`,
  dropped at the end of the function.

Dropping a `tokio::runtime::Runtime` from inside an async context panics. Reproduced with a
minimal crate of identical shape (a struct holding a current-thread `Runtime`, dropped inside a
future run under `Runtime::new().block_on(...)`):

```
thread 'main' panicked at tokio-1.53.1/src/runtime/blocking/shutdown.rs:51:21:
Cannot drop a runtime in a context where blocking is not allowed.
This happens when a runtime is dropped from within an asynchronous context.
```

**Failure scenario:** `domain-blocklist --signing-key k:key --output out` (no `--fixture-dir`)
fetches all five sources successfully over several minutes, then panics on the return path of
`fetch_all_sources` before merge, gates, or publish. Exit is a panic (SIGABRT/101), not the plan's
"exits non-zero with the specific reason." Identically, `--features net` without
`--skip-liveness` panics at the end of `run_sweep`, destroying a completed multi-hour sweep.

This is invisible to the whole test suite and to every verification the implementing agent
reported, because all of them ran with `--fixture-dir` (`github_fetcher` is `None`) and
`--skip-liveness`. **The two network paths this module exists to add have never executed to
completion.**

**Fix:** `spawn_blocking` the drop, or (better) delete the vestigial `rt` field from both types
when the caller drives `fetch_async`/`lookup_async` — it is unused on the async path.

### 2. One dropped UDP packet discards the entire sweep; the DNS client has no retries at all

`liveness/net.rs:198-226` (`query_udp`) sends exactly one datagram and waits one `timeout`. There
is no retry loop anywhere in `net.rs` or `sweep.rs` — every resolver client in existence retries
2-3 times because UDP loss is the normal case, not the exception.

For the *data* sweep this fails safe (`Unknown(Timeout)` never prunes). For the **canary** it is
fatal: `sweep.rs:110-129` (`canary_pass_async`) requires each alive control to be exactly
`Verdict::Alive` and each dead control exactly `Verdict::Dead`; anything else is a failure, and
`sweep.rs:216-222` turns that into `SweepError::MidSweepCanaryFailed`, aborting the whole run.
`main.rs:584-588` then never reaches `cache_store::save`.

**Failure scenario, quantified with the shipped defaults:** `canary_every = 2000` (`cli.rs:110`),
1,000,000 due domains ⇒ 500 canary rounds, each running both resolvers (`canary_pass_both`) over
every control, ~2 queries per dead control. With 2 alive + 2 dead controls that is ≈6,000 canary
queries per sweep. At a 0.1% UDP loss rate the probability of a clean sweep is
`0.999^6000 ≈ 0.25`; at 1% it is effectively zero. So **the expected outcome of a real 12-24 hour
sweep is an abort**, and the abort message tells the operator a resolver is lying ("canary check
failed mid-sweep ... the whole sweep's results are discarded") when the actual cause was one lost
packet.

This is precisely the bug class `liveness/` was rebuilt three times to avoid, reintroduced at the
layer that drives it: a transient failure made indistinguishable from a real negative, with the
highest-cost consequence attached.

**Fix:** retry each query 2-3 times with jitter before returning `Unknown`, and separately require
a canary control to fail on *repeat* observation (or across both resolvers) before condemning the
sweep.

## High

### 3. `net-shield` (on-device, including mobile) now transitively links a full HTTP client

`Cargo.toml` adds `reqwest`, `tokio`, `tar`, `flate2`, `clap`, `serde_json`, `tracing-subscriber`,
`futures` as **non-optional `[dependencies]`** (only `hickory-proto` is feature-gated), and
`lib.rs:8` adds `pub mod fetchers;`. `net-shield/Cargo.toml:17` depends on `domain-blocklist` by
path.

Reproduced:

```
$ cd packages/net-shield && cargo tree -e normal -i reqwest
reqwest v0.12.28
└── domain-blocklist v0.1.0
    └── net-shield v0.1.0
```

Resolving `net-shield` alone added 118 packages / 1,200 lines to its lockfile. `net-shield-ffi` →
`net-shield` is what ships in `apps/mobile`, so the on-device DNS guard now compiles an HTTP
client, a TLS stack, a tar/gzip decoder and an argument parser it can never call. That contradicts
`AGENTS.md`'s "keep code local-first; avoid network access in runtime paths" and materially
enlarges the attack surface of the one component that sits in the packet path.

**Fix:** put `fetchers`/`sweep`/`cache_store`/`slots` behind a `cli` feature (or move the binary's
deps to `[dev-dependencies]`/a `[[bin]]`-only feature), the same treatment `net` already gets.

### 4. With `RUST_LOG` unset, every `info!`/`warn!` in the pipeline is silently dropped

`main.rs:34-36` uses `EnvFilter::from_default_env()`, whose default directive when the variable is
unset is `error`. Reproduced with a minimal crate:

```
--- RUST_LOG unset:   ERROR-LINE-visible          (INFO and WARN absent)
--- RUST_LOG=info:    INFO / WARN / ERROR all present
```

**Failure scenario:** CI runs `domain-blocklist ...` on a monthly cron with no `RUST_LOG`. The
operator sees no merge counts, no per-source parse counts, no gate-pass lines, no artifact size —
and critically none of the four fail-open warnings: "no `--control-set` given; the false-positive
gate is effectively disabled" (`main.rs:482`), "no `--cache` given: ... results are not persisted"
(`main.rs:544`), "`--skip-liveness` set" (`main.rs:434`), and "no `--allow-license` given; using an
unratified default set" (`main.rs:382`). A build that silently disabled two of its publish gates
produces byte-identical output to a fully-gated one.

**Fix:** `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))`, and
promote the four warnings above to a structured, non-suppressible summary.

### 5. The false-positive gate defaults to disabled, and a real publish proceeds anyway

`cli.rs:52-53` documents `--control-set` as "Required for a real, non-dry-run publish," but
nothing enforces it — `main.rs:481-483` only warns. `cli.rs:146-147` defaults
`min_control_size` to **0**, the exact floor added to catch a truncated control-set fetch, and
`cli.rs:136-137` defaults `shrinkage_floor` to **0**.

**Failure scenario:** an operator runs the pipeline without `--control-set` (or with a control file
a network hiccup truncated to zero lines). `false_positive_gate` sees an empty control set,
`min_control_size = 0` passes the floor, the measured FP rate is 0/0, the gate reports `Pass`, and
the build **publishes and signs**. The gate the decision doc calls the last defence against
black-holing a mainstream site is off, and — per finding 4 — the only signal is a warning nobody
sees.

**Fix:** refuse to publish (not dry-run) without `--control-set`, and make `min_control_size`
default to a real number.

### 6. The shipped defaults make the `Dead` quarantine window a delay, not a confirmation

`cli.rs:114-120`: `ttl_seconds` defaults to **90 days**, `quarantine_seconds` to **35 days**.
`sweep.rs:170` only re-checks domains where `due_for_check(..., ttl_seconds)` is true, and
`sweep.rs:226-238` prunes any cached `Dead` whose `first_dead_at` is `quarantine_seconds` old.

**Failure scenario:** a monthly run corroborates NXDOMAIN for `example.com` from both 1.1.1.1 and
8.8.8.8 during a registry-level glitch or a shared stale-cache event; `first_dead_at = T`. The next
month's run finds the entry only 30 days old against a 90-day TTL, so it is **not re-checked** —
but `T + 35d` has passed, so it is pruned. The domain is removed from the published artifact having
been observed dead exactly once, never re-confirmed. Since `quarantine_seconds < ttl_seconds`, the
hysteresis mechanism CLAUDE.md describes as guarding against clientHold/RGP transients cannot fire
even once.

**Fix:** either require `quarantine_seconds >= 2 * ttl_seconds` at startup (fail loudly if not), or
force a re-check of any `Dead`-cached domain regardless of TTL.

## Medium-high

### 7. The default qps produces 2-4x the plan's polite budget, per resolver

`cli.rs:102-103` defaults `qps = 23.0` and describes it as "domain-checks started per second."
`sweep.rs:99-106` fires **both** resolvers concurrently per domain, and `check_async` issues an A
and (unless A resolved) an AAAA query to each. So one "domain start" is 2-4 queries: **23-46 qps
against `1.1.1.1` and simultaneously 23-46 qps against `8.8.8.8`**, 46-92 total.

The plan's ~23 qps figure is a *total query* budget (1M domains × 2 queries ÷ 86,400 s).
`sweep.rs:39-42`'s own doc comment concedes this ("a caller wanting the plan's exact raw-query qps
should divide accordingly") — but then ships 23.0 as the default, i.e. the out-of-box
configuration is the one the doc says to divide.

**Fix:** default to ~11 (or pace raw queries rather than domain starts).

The rate limiter itself is **correct at the boundaries** and no burst was found:
`sweep.rs:178-186` anchors deadlines at `start + interval*idx` per chunk with `start` captured
before the chunk, and the mid-chunk canary (`sweep.rs:216`) runs *after* the chunk, pushing the
next chunk's `start` later. Every deviation is in the slower-than-target direction. The flagged
"untested chunk-boundary design" is sound; the constant it is fed is not.

## Medium

### 8. Per-source fetches are fully serialized

`main.rs:240` is a plain `for job in jobs { ... .await ... }`. Five sources, each independent:
StevenBlack and Hagezi each make three sequential GitHub calls (`fetch_async`: revision → license →
content), and each of the three UT1 categories makes two (tarball + a *re-fetch of the same
site-global license page*, `ut1.rs:307-309`, which the file's own comment notes could be fetched
once). That is ~12 sequential round trips plus three multi-MB downloads, where a
`futures::try_join_all` over `jobs` would collapse it to the slowest single source. `futures` is
already a dependency.

### 9. The GitHub retry backoff blocks a Tokio worker thread, against the module's own written warning

`fetchers/github.rs:531` — `GithubSourceFetcher::new` wires
`delay: Arc::new(std::thread::sleep)`, and `get_with_retry` calls
`(self.delay)(policy.backoff_for(attempt))` (`github.rs:732`) inside an `async fn`. The field's own
doc comment (`github.rs:506-510`) says: *"only safe here because the runtime is a
single-current-thread runtime run under `block_on` — switching the pipeline to await its futures
directly must also switch this to `tokio::time::sleep`."*

`main.rs:256` does exactly that — `.fetch_async(&job.config).await` from the multi-thread
runtime — and does not switch the sleep. A rate-limited or 5xx GitHub response blocks a runtime
worker for up to 8s per retry with no `spawn_blocking`. Impact today is limited only because
finding 8 means nothing else is in flight.

### 10. The DNS client never matches a response to its request, and a comment claims a check that does not exist

`liveness/net.rs:195-197` states the connected socket is a defence "on top of this function's own
request-ID/question echo check." **There is no such check.** `query_udp` calls
`decode_response(&buf[..n])` (`net.rs:225`) and `interpret_response` (`net.rs:281`) reads
`response.answers` / `response.metadata` without ever comparing `response.metadata.id` to the
request's, or `response.queries` to the question asked. `build_query` (`net.rs:180`) uses
`Message::query()` and never sets a random ID.

`UdpSocket::connect` does give kernel-level source-address/port filtering, so this is not
immediately exploitable off-path — but the repo's `references/dns.md` names ID-and-question
validation as a required check and records the unfixed `NetworkGuardService.ask()` defect of the
same shape. More seriously, the comment is a fabricated citation of the code's own behaviour and
will be read by the next implementer as coverage that exists.

### 11. Transport failures are recorded as `Malformed`, destroying the diagnostic the cache is for

`liveness/net.rs:120-123`: `QueryFailure::Transport` → `LookupResult::Unknown(UnknownReason::Malformed)`.
A connection reset, an unreachable network, or a socket-bind failure is therefore persisted in the
liveness cache as a *protocol* error indistinguishable from a resolver emitting garbage.
`UnknownReason` has no transport variant and none was added.

**Failure scenario:** the build host loses egress for two hours mid-sweep; the cache fills with
`Unknown(Malformed)` for tens of thousands of domains, and the next operator debugging "why is 8%
of my list Malformed?" investigates the resolver instead of the network. Pruning is unaffected
(`Unknown` never prunes), so this is diagnostic damage, not correctness damage.

### 12. `collapse_snapshots_by_source` is sound on license but lossy on provenance

`main.rs:324-355`. Assessment, split by axis:

- **License: sound.** It asserts every grouped snapshot agrees via `spdx_matches` and *bails* on
  disagreement (`main.rs:333-340`) rather than silently picking one. This correctly preserves
  `license_gate`'s one-snapshot-per-`SourceId` invariant instead of weakening the gate.
- **Revision: lossy in a way that matters.** `main.rs:341-345` joins the three UT1 revisions with
  `";"`. The resulting `SourceSnapshot.revision` in the signed manifest — e.g.
  `"W/\"abc\";W/\"def\";W/\"ghi\""` — equals no source's `pinned_revision`, so nothing downstream can
  mechanically re-verify the published manifest against the pins, and **which category each
  revision belongs to is gone** (order is implicit `SourceJob` order, undocumented). If a revision
  string ever contains `;` (an ETag legitimately may), the composite is also un-splittable. A
  `Vec<String>` field, or `"adult=<rev>;gambling=<rev>"`, would cost nothing.
- **`fetched_at`: silently wrong.** `main.rs:346` takes `.min()` across the group, so the manifest
  timestamps UT1's data as older than two of its three category fetches actually are. Staleness
  reporting reads too pessimistic — the safe direction, but it is a fabricated value, not a
  measured one.

### 13. A 12-24 hour sweep emits no progress output whatsoever

`sweep.rs` contains zero `tracing` calls. Between `main.rs:575` (`run_sweep` invoked) and
`main.rs:578` ("liveness sweep complete") there is no per-chunk log, no running
Alive/Dead/Unknown tally, no canary-passed line, no ETA. An operator whose run has been silent for
nine hours cannot distinguish "half done and healthy" from "wedged on a socket" from "grinding
through `Unknown`s because the resolver is rate-limiting us." The module-3 audit's own deferred
item — an aggregate `Unknown`-rate health signal — was named as the mechanism that would catch a
rate limit being hit in production, and it is not here. Combined with finding 4, the
default-configured run prints nothing at all until it exits.

## Low-medium / low

### 14. `publish`'s rotation is atomic per file but not across the pair

`slots.rs:118-123` rotates `artifact.fst` then `manifest.bin` as two separate `rename`s. An
interruption between them leaves `previous/` holding the *new-old* artifact beside the *old-old*
manifest (digest mismatch) while `current/` holds only a manifest. `net-shield`'s `load()`
(`blocklist.rs:108-122`) then gets `SlotMissing` on `current/` and `DigestMismatch` on `previous/`
and returns `Err` — **both slots unusable, no artifact on device**. It fails closed and the next
pipeline run self-heals, so severity is bounded, but the plan's "write both files atomically per
slot" is satisfied per-file and not per-slot. A `previous.tmp` directory renamed into place would
close it.

### 15. A mid-body read failure on a UT1 tarball is not retried

`ut1.rs:480-486`: `AttemptFailure::from_reqwest` maps `is_timeout() || is_connect()` to `Timeout`
(retried) and everything else to `Transport` (**not** retried — `ut1.rs:389-395`). A TCP reset
partway through the multi-megabyte `adult.tar.gz` download is a body error, not a connect error, so
it aborts the whole 24-hour build on the first occurrence against a single French university origin
with no CDN. `github.rs:167-171` handles this case correctly (mid-body read errors are classified
retryable, with a comment explaining why); `ut1.rs` diverges.

### 16. Fixture mode makes pin and license validation tautological

`main.rs:244-248` synthesizes `revision: job.config.pinned_revision.clone()` and
`license: job.config.expected_license.clone()` from the config being validated against. So
`fetch_source`'s `RevisionMismatch` / `LicenseChanged` / `LicenseNotAllowed` checks — module 1's
whole reason for existing — **can never fire in fixture mode**, which is the only mode that has
ever been run (see finding 1). Not wrong as a design, but it means the license/pin gate is
unexercised end to end, and the fixture path cannot be used to test it.

### 17. `pick_output_license`'s permissiveness ranking is a guessed constant

`main.rs:362`: `const RANK: &[&str] = &["CC0-1.0", "MIT", "CC-BY-SA-4.0", "GPL-3.0"]`. The
relative ordering of CC-BY-SA-4.0 against GPL-3.0 (two copyleft licenses of different families) is
a legal judgement with no citation, and it decides the `output_license` written into the signed
manifest — i.e. it crosses a contract boundary into `net-shield`. The `UNKNOWN` fallback is handled
well; the ordering is not sourced.

## Structural claims independently verified

**(a) `lib.rs` and `liveness/mod.rs` are additive-only — CONFIRMED, with one caveat.** `git diff`
shows `lib.rs` adds `pub mod fetchers;` and appends `check_corroborated, corroborate,
is_special_use_domain` to the existing `liveness` re-export list; `liveness/mod.rs` adds a
`#[cfg(feature = "net")] pub mod net;` and its matching re-export. No signature, no behaviour, no
existing export changed, and `types.rs`/`sources/`/`merge.rs`/`gates.rs`/`fst_build.rs`/
`liveness/{lookup,cache,canary,corroboration}.rs` are untouched per `git status`. **The caveat:**
`Cargo.toml` is *not* additive-only in effect — the new non-optional dependencies change what every
downstream consumer compiles (finding 3).

**(b) `slots.rs` matches `BlocklistArtifact::load` byte-for-byte — CONFIRMED.** Verified field by
field against `packages/net-shield/src/blocklist.rs`:

| | `slots.rs` | `net-shield/blocklist.rs` |
|---|---|---|
| artifact file | `"artifact.fst"` (:22) | `"artifact.fst"` (:38) |
| manifest file | `"manifest.bin"` (:24) | `"manifest.bin"` (:40) |
| slot dirs | `"current"` / `"previous"` (:25-26) | `"current"` / `"previous"` (:41-42) |
| manifest codec | `bincode::serialize` (:127) | `bincode::deserialize` (:143) |
| digest | `Sha256` over raw fst bytes (:82-85) | `Sha256` over the mmap (:157-159) |
| order | writes `current/`, rotates old → `previous/` | reads `Current` then `Previous` (:110) |

Rotation direction is correct: the build being replaced becomes the fallback, which is what
`load()`'s fallback expects. The only gap is the cross-file atomicity in finding 14 — the *names
and order* match exactly; the *interruption window* does not produce a state `load()` can recover
from without a rerun.

## Coverage note

`docs/engineering/coverage.md` is not touched by this change, and the change moves a row:
`net-shield`'s module-6 note that `load()` "has only ever loaded the two-slot layout a test helper
wrote, never one module 7 `cli` produced" is **still true** after this work, despite module 7
existing — `slots.rs`'s tests write and read their own output, and no test in either crate takes a
`slots::publish` output and feeds it to `BlocklistArtifact::load`. The two ends have not met. That
cross-crate test is cheap (`net-shield` already depends on `domain-blocklist`) and is the single
check that would convert claim (b) from read-verified to observed.
