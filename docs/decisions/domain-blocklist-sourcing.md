# Decision: Domain Blocklist Sourcing, Merging, and Distribution

## Status

Accepted. No code exists yet — this records the design so `packages/domain-blocklist` (planned,
see [its plan](../components/domain-blocklist/plan.md)) and `net-shield`'s placeholder blocklist
loading path have something concrete to build against.

## Context

Reliable domain blocking depends on a reliable domain list. That list has three separate problems
bundled together — where the domains come from, how multiple sources get combined into one list
without amplifying each other's noise, and how the result gets onto a device (including a mobile
device, where storage and RAM are tighter) — and each one has a wrong-but-tempting shortcut.

## Legal boundary (settled first, because it constrains everything else)

Holding a plain list of domain names that host legal adult content is not restricted in the EU or
US. A domain name is directory metadata, not content — no different in kind from a phone book or a
firewall deny-list. GDPR does not reach it either, because a domain list is not personal data about
an identifiable person; the place GDPR does bite is logging which domains *a specific user*
visited, which this project already avoids by not phoning home at all.

**Hard boundary: this project never builds, holds, or infers a CSAM domain list.** Lists that
identify illegal child-exploitation material are a different legal category, maintained under
chain-of-custody by designated bodies (NCMEC, IWF, INHOPE) and distributed only to vetted members
under contract. Holy Blocker's scope is legal-but-unwanted adult content, never illegal-content
detection, and the pipeline must never attempt to compile or supplement anything resembling that
list itself.

If a source list ever turns out to contain a domain that is actually CSAM rather than legal adult
content, the correct action is the same as for any operator: block it (already the intended
outcome) and report it through NCMEC's CyberTipline or IWF's portal — never fetch, render, cache,
or manually inspect the content to "confirm" it first. This is why the liveness check below is
DNS-only: it must never perform an application-layer request against a listed domain, so the
pipeline can never accidentally retrieve or display a page body for anything on the list.

## Decision

### Sources

Start with three, all actively maintained and with source-tracking rather than blind trust in any
one of them:

- **[StevenBlack/hosts](https://github.com/StevenBlack/hosts)** (`porn` extension) — MIT licensed,
  widely used, itself an aggregation of smaller lists.
- **[hagezi/dns-blocklists](https://github.com/hagezi/dns-blocklists)** (NSFW list) — actively
  maintained, documented methodology, frequent updates.
- **[UT1 blacklists](https://dsi.ut-capitole.fr/blacklists/)** (Université Toulouse Capitole,
  `adult` category) — a 15+ year academic project purpose-built for institutional content
  filtering, the most rigorously documented of the three.

Verify each source's current license text at build time and record it in the provenance metadata
(see below) rather than assuming a license never changes. Some blocklist projects (e.g. oisd.nl)
restrict redistribution/commercial bundling even though the underlying list is legal to hold —
that is a distribution-license question, separate from the legality question above, and it is
checked per source before anything is vendored into a shipped artifact.

### Combining sources

1. **Normalize every entry the same way before comparing**: lowercase, strip `www.`,
   punycode-normalize IDNs, strip trailing dots.
2. **Union with provenance** — track which source(s) flagged each domain, not just the domain
   itself. This matters twice: (a) if a source turns out to be low-quality or disappears, it can
   be backed out cleanly; (b) if a user disputes a block, the pipeline can say why it's blocked
   instead of shrugging.
3. **Periodically revalidate liveness** — dead domains accumulate in every one of these lists
   (sites shut down, nobody prunes). A stale entry is nearly free in a compact filter, but it's
   still useless bulk that should be diffed out over time to keep updates small.
4. **User-level allowlist override sits on top, unconditionally** — false positives are inevitable
   with any aggregated list, and there needs to be a local, no-appeal-required way to unblock a
   domain the household actually trusts.
5. **Don't blend categories silently** — a source's "adult" category and its "gambling" or
   "dating" category are different products; only merge the categories actually shipped as one
   filter.

### Liveness revalidation: DNS-only, centralized, cached with a TTL

Liveness is checked by **DNS resolution only** (does the domain resolve to any A/AAAA record, or
NXDOMAIN) — never a full HTTP fetch, and never ICMP. ICMP is the wrong signal regardless of cost:
most hosting today sits behind a CDN or load balancer that either doesn't route ICMP to the origin
or answers ping from an edge node unrelated to whether the site is actually up. DNS-only also
satisfies the legal boundary above, since it never performs an application-layer request that
could retrieve a page body.

**This runs centrally, in the list-build pipeline, on a monthly cadence — never on an end-user
device.** Client daemons only ever consume an already-pruned, signed list; they do not run their
own liveness checks and this cost is never charged against a user's mobile data.

**Approximate egress for a monthly sweep of ~1,000,000 domains:**

- A DNS query (IPv4 header + UDP header + DNS header + question) for a typical hostname is
  roughly 65–75 bytes on the wire.
- A response (positive A record or NXDOMAIN-with-SOA) is roughly 100–180 bytes.
- Round trip: **~150–250 bytes per domain**, call it ~200 bytes as a working average.
- Full sweep of 1,000,000 domains: **~150–250 MB**, spread over a month that's roughly
  **5–8 MB/day**. Trivial for the pipeline infrastructure it actually runs on.

That figure is the **worst case** — a full sweep of every domain from scratch. Steady-state cost
is lower once the cache exists:

- Each domain's liveness verdict is cached with `last_checked` + `verdict` + provenance.
- A domain cached as **dead** that reappears in a source refresh is only rechecked if its cache
  entry is older than the TTL (starting default: **1 month**) — this is what prevents a source
  that never re-prunes its own stale entries from silently forcing a recheck every cycle, while
  still catching a genuine revival (domain resold/re-registered) eventually.
- A domain cached as **alive**, or brand new to every source, is checked/rechecked normally.
- Batching multiple domains into one DNS query is not practically available — RFC 1035's QDCOUNT
  field permits it in principle, but essentially no real-world resolver answers more than one
  question per message. The available lever is **concurrency** (many in-flight query packets at
  once, rate-limited against the resolver), not batching.

### On-device storage and lookup

The distributed artifact is a **minimal-DFA finite-state transducer (FST) over reversed domain
labels** (Rust's `fst` crate — pure Rust, no native C/C++ cross-compilation burden, unlike e.g.
`marisa-trie`), not a plain text list or a runtime-only in-memory trie:

- **Non-lossy** — an exact set (optionally mapped to a small value carrying a category/provenance
  ID), not a probabilistic structure. No Bloom filter as the primary structure: a Bloom filter's
  false positives fail the non-lossy requirement outright.
- **Compact** — an FST shares both common prefixes *and* common suffixes across entries, unlike a
  plain trie which only shares prefixes. Domain data compresses well under this (`.com`, common
  subdomain patterns, common second-level names all collapse into shared DFA states); expect
  roughly 2–4 bytes/entry versus 15–30 bytes/entry for raw text.
- **Reversed labels turn suffix matching into prefix matching**: `www.example.com` is stored as
  `com.example.www`. A query is answered by exact lookups at each label boundary from shortest to
  longest (`com`, `com.example`, `com.example.www`, ...) — at most ~4–5 lookups per FQDN, first
  hit wins, since a block on a higher-level domain covers everything under it. The general
  prefix-streaming capability of an FST is a bonus for secondary uses (a "why is this blocked"
  debug view, allowlist-manager autocomplete, version-diffing) rather than something the hot path
  needs.
- **mmap-backed, not eagerly loaded into a heap buffer.** At microsecond-scale page-fault cost,
  there is no meaningful lookup-speed penalty versus keeping the whole structure resident, and
  mmap wins on the property that actually matters on a memory-constrained platform: file-backed
  pages are trivially reclaimable by the OS under pressure, where a heap-resident buffer of the
  same bytes cannot be reclaimed without killing the process — a real risk under Android's
  OOM-killer behavior for backgrounded processes.
  - Hold the mapping behind a reference-counted handle (`Arc<Mmap>` via the `memmap2` crate in
    Rust) so an update swaps to a new `Arc` without ever holding two full resident copies at once;
    the old mapping's pages are reclaimed once the last in-flight lookup releases it.
  - Updates are atomic at the file level: write the new signed file to a temp path and `rename()`
    over the old one (atomic on POSIX filesystems, including Android's) — never overwrite the
    mapped file in place.
  - A missing, corrupt, or signature-verification-failed file fails closed to the last-known-good
    mapping, never to an empty (fail-open) structure.

### Distribution

The FST file *is* the signed, distributed artifact — no separate transform happens on the client.
The pipeline that does normalize → merge → dedupe → liveness-TTL-prune also emits the final `.fst`
file and a manifest, signs the bundle with an Ed25519 key (public half shipped in the binary), and
publishes it (e.g. GitHub Releases — no login required, CDN-cached, cheap conditional `ETag` GETs
for clients that are already current). Client updates are opt-in, never silent background polling,
consistent with the project's local-first default.

## Rejected alternatives

- **A single source** — no single list has both good coverage and reliable pruning; the union
  with provenance is what makes a low-quality or disappearing source cheap to back out.
- **ICMP-based liveness checking** — unreliable signal against CDN-fronted origins, and an
  application-layer-adjacent probe is unnecessary risk against the CSAM boundary above; DNS-only
  is both cheaper to reason about and safer.
- **Client-side liveness revalidation** — would charge the recheck cost against every installed
  device's egress and battery for a result that is identical across all of them; centralizing it
  once in the pipeline is strictly better.
- **Bloom filter as the primary on-device structure** — fails the non-lossy requirement.
- **`marisa-trie` instead of `fst`** — comparable compactness, but a native C++ dependency that
  would need cross-compiling across every Android ABI this project targets, repeating the class of
  pain already hit with `ort`/ONNX Runtime on Android (see the `image-sandbox` row in `AGENTS.md`).
- **Eager heap-resident load of the FST bytes at startup** — simpler, and not meaningfully slower
  at realistic list sizes, but gives up OS-level reclaim-under-pressure and forces a
  double-memory window during an atomic update; mmap gets both properties for free at negligible
  cost.
