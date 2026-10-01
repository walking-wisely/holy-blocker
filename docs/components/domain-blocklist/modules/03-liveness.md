# 3. `liveness` — DNS-only revalidation with a persistent TTL cache

Part of the [Domain Blocklist plan](../plan.md).

```text
src/liveness.rs
```

Responsibilities:

- `LivenessCache` — a `domain → { last_checked, verdict, sources }` table. **Its storage is a named,
  persistent, non-repository location**: a private object-store bucket, a release asset on the
  pipeline's own distribution channel, or a CI artifact cache with retention configured well beyond
  the TTL. It is fetched at run start and written back at run end. A CI runner's filesystem is
  ephemeral and cannot hold it; "the pipeline's own storage" is not an answer. It is **not**
  committed into the public repository tree, consistent with the project's convention of gitignoring
  corpora and model artifacts. A run that cannot load the cache starts cold; a run that cannot write
  it back **fails loudly** rather than discarding months of accumulated state.
- `check(domain: &str) -> Verdict` — **two lookups per domain, A and AAAA.** Not one: they are
  separate QTYPEs, and `QTYPE=ANY` is not a shortcut (RFC 8482). `Verdict` is **not** a boolean:

  ```rust
  enum Verdict {
      Alive,                 // A or AAAA record, or a CNAME chain resolving to one
      Dead,                  // NXDOMAIN
      Unknown(UnknownReason), // NODATA, SERVFAIL, REFUSED, timeout, or a malformed response
  }
  ```

  Only `Dead` prunes a domain from the next build. `Unknown` is left exactly as it was in the
  previous build — a resolver hiccup or a transient SERVFAIL must never be able to silently shrink
  the blocklist. A domain stuck in `Unknown` across repeated checks is a signal worth surfacing in
  the build's own logs/metrics, not a reason to change its inclusion.
  No HTTP fetch, no TCP connection to the domain's own server, no ICMP — see the decision doc's
  legal boundary for why an application-layer request against a listed domain is never made here.

  **Reducing the A and AAAA lookups to one `Verdict`** — the two queries are independent and can
  disagree (e.g. A returns `NXDOMAIN` while AAAA times out), so `check()` applies this order,
  evaluated top to bottom:

  1. If **either** lookup (or its CNAME chain) resolves to an address, the result is `Alive`. One
     working address family is enough for the domain to be reachable.
  2. Otherwise, if **both** lookups unambiguously return `NXDOMAIN`, the result is `Dead`.
  3. Otherwise — at least one lookup is `Unknown` and neither is `Alive` — the result is `Unknown`.
     This covers a mixed `NXDOMAIN`/`Unknown` pair: one family failing to resolve while the other
     family's answer is merely inconclusive must never be conflated with both families cleanly
     saying the domain doesn't exist.

  A domain checked for the first time that comes back `Unknown` is **included by default**, the
  same as an existing domain whose cached verdict is `Unknown` — a build never treats "we couldn't
  tell" as a reason to omit a domain it has no prior information about. Test the mixed-verdict case
  explicitly, and test first-seen-`Unknown` retention separately from previously-cached-`Unknown`
  retention.
- **An explicitly configured resolver, never the host's system resolver.** The address is
  configuration, defaulting to Cloudflare's unfiltered `1.1.1.1` / `2606:4700:4700::1111` —
  explicitly *not* the `1.1.1.2` or `1.1.1.3` filtering variants. A CI provider's default resolver
  is frequently a filtering one, and that failure is silent and total.
- **`canary_check() -> CanaryResult`, run before every sweep and gating it.** A small fixed control
  set, checked in both directions:
  - Several known-always-alive, definitely-non-adult domains must each return `Alive`.
  - At least one reserved never-resolving name (RFC 2606 / RFC 6761 — `invalid.`, a name under
    `test.`) must return `Dead`, which catches a resolver that NXDOMAIN-rewrites to a wildcard sink.

  **Any canary result other than expected aborts the entire sweep.** Every verdict from that run is
  discarded — not merged, not written back to the cache — and nothing is published. There is no way
  to know at which query a lying resolver started lying, so a partial sweep is not salvaged. This is
  the guard against the single worst failure in the design: a content-filtering resolver returning
  NXDOMAIN for the whole list, pruning it to near-zero, and shipping that signed.
- `due_for_check(cache_entry, now, ttl) -> bool` — pure function deciding whether a cached verdict is
  still trusted or needs a fresh lookup. **TTL is decoupled from the run cadence and is a multiple of
  it: cadence monthly, TTL 3 cadences (~3 months).** With TTL equal to the cadence, every cached
  entry is due on every run and the cache saves nothing while being exquisitely sensitive to clock
  jitter — a run a minute early or late swings the check volume wildly. At 3× only about one third of
  the previously-dead cohort is due on any run, which is where the saving actually comes from, and a
  genuine revival is still caught within one TTL. Both numbers are configuration, and the tests
  should cover cadence ≠ TTL explicitly.
- **Pacing: a steady low rate across a bounded ~24-hour sweep window**, not a burst and not smeared
  across the whole month. A cold sweep of ~1,000,000 domains is ~2,000,000 queries, which over 24
  hours is **~23 qps**. Concurrency (many in-flight queries, rate-limited to the target qps) is the
  lever; batching is not available, since a single DNS message answering multiple questions is not
  something real-world resolvers implement despite RFC 1035's QDCOUNT nominally allowing it.
- A `Dead` verdict removes the entry from the next build's output; it does not delete it from the
  cache, so a later revival is still detected once its TTL expires rather than being permanently
  invisible.
- Note the accepted limitation, stated in the decision doc and worth restating where it is
  implemented: **DNS liveness measures registration, not content.** A parked domain still resolves.
  Erring toward keeping an entry is the intended direction — false negatives are the budget, false
  positives are the price — which is why only an unambiguous NXDOMAIN prunes.

**Done.** Built as `src/liveness/{mod,lookup,cache,canary,corroboration}.rs`, then hardened across
three review rounds. The load-bearing correction from the third round: an earlier version of this
module required DNSSEC authentication (the AD bit) on both address families before trusting an
NXDOMAIN as `Dead`, and that requirement was measured live to be **the wrong design**, not a
stricter-but-safer one — most large TLDs sign with NSEC3 opt-out (RFC 5155 §6), under which a
validating resolver cannot construct authenticated denial-of-existence for an unsigned delegation
at all, so the gate made `Dead` almost unreachable rather than safer to reach, and the module's own
canary didn't catch it (`invalid.`/`test.` sit under the root zone, which uses plain NSEC rather
than NSEC3 opt-out, so they authenticated cleanly while the real sweep quietly pruned nothing). The
fix
is `liveness::corroboration::corroborate`/`check_corroborated`: pruning now requires **two
independently-configured resolvers** to each independently produce `Dead`, replacing the
DNSSEC-authentication requirement as the actual defence against a lying or hijacking resolver;
`authenticated` is kept as loggable evidence but no longer gates anything. `should_prune`/
`should_prune_with_hysteresis` (`cache.rs`) now require the corroborated verdict, not a single
resolver's own `check()` result. Also fixed in the same round: `is_filtering_ede` broadened to
cover RFC 8914 codes 13 (Cached Error) and 18 (Prohibited) alongside 19 (Stale NXDOMAIN Answer);
`nonce_dead_control`'s worked example and this file's own tests, which used the Cloudflare-hosted
`example.com` — measured to answer a nonexistent subdomain with NODATA ("compact denial of
existence") rather than NXDOMAIN, which broke the dead control and aborted every sweep
permanently — renamed away from it, with the trap now documented explicitly; the apparent tension
between `should_prune` (a `Dead` verdict on `.invalid`/`.test` proves nothing about registration)
and `canary_check` (the same verdict on the same names proves the resolver is honest) resolved with
cross-referencing doc comments explaining these are different questions about the same verdict, not
a contradiction; and the `DnsLookup` trait's contract doc now states explicitly that an
implementation must set the DNSSEC OK bit and read the AD/EDE evidence off the wire, ruling out a
`getaddrinfo`-style high-level resolver API for module 7.

(See [Reference documents](../plan.md#reference-documents) below for the DNS response codes this matrix is
built from.)
