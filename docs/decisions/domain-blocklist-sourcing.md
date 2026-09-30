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

Two figures used throughout this document are **planning assumptions, not measurements**: the
merged entry count (~1,000,000) and the FST's bytes-per-entry. Both are gated by build-time checks
described below rather than trusted, for the same reason `image-sandbox`'s two v0 constants were
both guesses and both wrong.

## Legal boundary (settled first, because it constrains everything else)

### Jurisdictional scope, and what it does not cover

The operating assumption is **US, EU, and UK law** — UK explicitly, because UK GDPR / DPA 2018 is a
separate regime post-Brexit and because IWF, cited below, is a UK body. No other jurisdiction is
analysed here.

That scope carries a known, documented simplification: **"legal adult content" is itself
jurisdiction-relative.** The legality of drawn and animated depictions in particular varies by
country — and this project's own `machine-learning` work trains on drawn/anime material, so the
boundary is not hypothetical. This document does not claim the legal/illegal line it draws is
universal; it claims only that the line is drawn under US/EU/UK law, and that a deployment
elsewhere needs its own review. Stating this is not a hedge — it is the difference between a
documented scope and an unexamined assumption.

### Users

The specific, defensible privacy claim is narrower than "GDPR doesn't apply": **this system does
not collect, log, or transmit which domains a specific user visited.** That is the category of
processing that matters here, and no part of this design does it — verdicts are computed on-device
against a locally-held artifact, and nothing about a lookup leaves the device.

The claim is *not* "this system never phones home." It does, in exactly one place: the
[distribution](#distribution) fetch. Downloading a blocklist bundle from a public host is an
outbound request that reveals to that host (and to anyone on the path) that a given IP address runs
adult-content-filtering software, on a recurring cadence. That is a real, if much smaller, privacy
signal than a browsing log — it carries no per-domain information, only the fact of the software's
presence. It is worth stating plainly rather than designing around: updates are opt-in, so the
signal is user-consented, and a user who does not want it can decline updates and keep a stale
list.

### Domain operators

The larger exposure is not users, it is **domain operators**. A domain string can itself identify a
natural person — a creator site branded with a personal name is the ordinary case, not the edge
case — and classifying such a domain as hosting adult content is processing of special-category
data under GDPR Art. 9 (data concerning sex life or sexual orientation). This is not solved by
"we don't store user data."

**The Art. 6/Art. 9/Art. 14 analysis below is a documented working assumption, not a legal
opinion.** It records the reasoning this design was built against so an implementer isn't starting
from nothing, but none of it has been reviewed by qualified counsel, and shipping the CSAM-adjacent
personal-name triage or the general notice described here needs that review first — **the project
owner is the decision owner for that sign-off**, tracked the same way any other pre-ship gate is
tracked, not assumed satisfied by this document existing.

**Lawful basis (working assumption).** Art. 6(1)(f), legitimate interest: child and household
content safety. The Art. 9 condition is **Art. 9(2)(e), data manifestly made public by the data
subject** — an operator publicly running an adult site under their own domain and branding has
manifestly made that fact public themselves. This is the only Art. 9(2) condition that fits;
consent is unobtainable at this scale and none of the other exemptions apply. This is the pipeline
author's reading, offered as the starting point for counsel review, not a substitute for it.

**Notice (working assumption).** Individual notice to every operator in a multi-million-entry
aggregated list is disproportionate, so **Art. 14(5)(b)** applies. That exemption is not silence —
the obligation it substitutes is a *public, general notice*: a published statement describing what
the pipeline classifies, which sources it draws from, and how to dispute an entry. Publishing that
statement is a shipping requirement, not a nicety, and its wording is part of what counsel review
above needs to cover before the pipeline ships to real users.

**Automated triage, not manual review.** Manually reviewing a multi-million-entry list is not a
plan. What is implementable is a narrow heuristic in the merge step: flag a domain whose
second-level label decomposes into **2–4 alphabetic tokens separated by `-`, `.`, or `_`**, in any
order (covering `first-last`, `last-first`, `first.middle.last`, and the underscore variants),
optionally raising confidence against a first-name/surname frequency list. Only matches enter a
lighter review queue. This is explicitly a **triage filter to bound workload, not a completeness
guarantee** — it will miss single-token personal brands and will flag plenty of ordinary
two-word domains. Its value is that it turns "review everything" (impossible) into "review a
tractable slice" (routine).

**Dispute and rectification.** The device-local allowlist is not a remedy for an operator: it does
not reach them, and they need a channel *into* the pipeline, not one out of it. A public
operator-facing channel (a form or a published contact address, named in the general notice) is
therefore required, and disputes split into two tracks with different correct outcomes:

- **Rectification** — "this is misclassified / not adult content / not my domain." Verify and
  correct. This unblocks: the entry is removed from the next build and recorded as a source-level
  correction so a later refresh of the same source does not silently reinstate it.
- **Objection to accurate processing (Art. 21)** — "the classification is accurate, I object
  anyway." The controller may refuse and continue processing where there are compelling legitimate
  grounds, and child-safety is one. But the refusal must be a **documented balancing judgment made
  each time**, not silence and not a template. What makes refusing defensible is minimization: the
  pipeline stores nothing about an operator beyond the domain string and its provenance — no
  cross-referencing to an off-domain identity, no enrichment, no contact scraping. That
  minimization is a design constraint, not a policy preference, precisely because it is what the
  refusal rests on.

### The CSAM boundary

**This project never builds, holds, or infers a CSAM domain list.** Lists that identify illegal
child-exploitation material are a different legal category, maintained under chain-of-custody by
designated bodies (NCMEC, IWF, INHOPE) and distributed only to vetted members under contract. Holy
Blocker's scope is legal-but-unwanted adult content, never illegal-content detection, and the
pipeline must never attempt to compile or supplement anything resembling that list itself.

A blocklist match is **not**, by itself, a reportable incident — the sources here classify legal
adult content, so the overwhelming majority of matches are exactly that and require no action
beyond blocking.

The exceptional case is a credible, specific suspicion that a particular entry is CSAM rather than
legal content. **That suspicion may only originate externally.** The pipeline forbids inspecting
content, so it cannot itself produce such a signal, and a trigger that requires a contributor to
have looked is circular. The only admissible triggers are:

- a user report,
- a partner or hotline organisation's public takedown notice,
- a law-enforcement notice.

Handling, precisely:

- Reports arrive through a **private channel** — a published security-contact address or a private
  issue tracker. Never a public GitHub issue, and never anything that lands in a public CI log. An
  open-source project cannot promise "restricted access" to a public build artifact, so the control
  is that the material never enters the public surface in the first place.
- **Retention is minimal**: the domain string and the external report's own description. Nothing is
  captured independently — the site is never fetched, rendered, cached, or inspected to "confirm"
  it. The record is deleted once it has been reported onward and acted on.
- The entry is quarantined (held out of the published artifact) and reported through NCMEC's
  CyberTipline or IWF's portal.
- **Reporting obligations vary by jurisdiction.** A contributor outside the US/UK may have
  different, and in some countries mandatory, duties. This document does not enumerate them; it
  requires that a contributor receiving such a report route it through the private channel rather
  than acting alone.

This is also why the liveness check below is DNS-only: it must never perform an application-layer
request against a listed domain, so the pipeline can never accidentally retrieve or display a page
body for anything on the list.

## Decision

### Sources

Start with three, all actively maintained and with source-tracking rather than blind trust in any
one of them:

- **[StevenBlack/hosts](https://github.com/StevenBlack/hosts)** (`porn` extension) — widely used,
  itself an aggregation of smaller lists.
- **[hagezi/dns-blocklists](https://github.com/hagezi/dns-blocklists)** (NSFW list) — actively
  maintained, documented methodology, frequent updates.
- **[UT1 blacklists](https://dsi.ut-capitole.fr/blacklists/)** (Université Toulouse Capitole,
  `adult` category) — a 15+ year academic project purpose-built for institutional content
  filtering, the most rigorously documented of the three.

#### Pinned revisions, never floating HEAD

Each source is pinned to a **specific release tag or commit hash**, recorded in the pipeline's
checked-in configuration. Moving to a newer upstream revision is an explicit version bump — a
reviewed diff in a pull request — not something a scheduled run does by itself.

This is the integrity gate on the *upstream* leg, which signing alone does not provide: the
Ed25519 signature described under [Distribution](#distribution) proves the artifact came from this
pipeline, and says nothing about whether a compromised upstream repository or a bad merged PR fed
it garbage. Auto-fetching `HEAD` would launder an upstream compromise into a validly signed
artifact within one cadence. Pinning converts that into something a human sees.

Combined with the diff-size guard below, a version bump that adds an implausible number of entries
also stops before publication rather than after.

#### License gate

**Each source's current license must be verified before implementation, and re-verified at every
build.** This document deliberately does not assert what any of the three licenses currently say —
licenses change, and an assertion in a decision doc is exactly the kind of one-time assumption that
goes stale silently.

What the design needs to know is which license *shapes* would break it, since the artifact is a
redistributed derived binary:

- **Share-alike / copyleft terms** that propagate into a derived combined work would attach to the
  redistributed `.fst` artifact, and potentially to whatever ships alongside it. That is a design
  break, not a paperwork item.
- **Non-commercial-only terms** are compatible with the artifact as published today, but become a
  break the moment the artifact is bundled with anything monetized.
- **No-redistribution terms** (some blocklist projects carry these — oisd.nl is the commonly cited
  example) block vendoring the source into a shipped artifact outright, even though the underlying
  list is perfectly legal to hold. That is a distribution-license question, entirely separate from
  the legality question above.

The mechanism, so this is enforced rather than remembered: each source's `SourceSnapshot` carries a
`license` field, and the build **checks it against a small checked-in allowlist of compatible
license identifiers**. A source whose license identifier is not on the allowlist is refused —
the build fails rather than quietly including it. Adding an identifier to the allowlist is a
reviewed change, which is where the actual legal judgment gets made and recorded.

**The merged artifact's own license is the least-permissive of its inputs**, computed at build time
and written into the manifest, so the published bundle always states terms it can actually satisfy.

Provenance is tracked one level deep, at the source actually being pulled from — StevenBlack's
`porn` extension is itself an aggregation of smaller lists, and this decision does not require
unwinding that further; if StevenBlack's own aggregation is later found to embed something
improperly licensed, the fix is dropping the StevenBlack source entirely, not attributing
individual entries within it.

#### A failed fetch aborts the build

If any configured source fails to fetch — network error, 404 on a pinned tag, a checksum mismatch —
the **entire build aborts and nothing is published**. It does not proceed with two of three sources.

Silently shipping partial coverage is the worst available outcome: it looks like a successful build,
produces a validly signed artifact, and quietly removes a slice of protection that nobody is
watching for. Dropping a source permanently is a deliberate, reviewed configuration change; it is
never something a transient fetch failure decides.

### Combining sources

#### 1. Normalization is for comparison only, and never widens a rule's scope

This is the sharpest correctness rule in the document, because getting it wrong blocks entire
hosting providers. `net-shield`'s own `radix.rs` test suite already demonstrates the failure class:
a rule for `adult` matching `site.adult`.

Two distinct things must not be conflated:

- **`normalize()` — a comparison key.** Lowercase; apply UTS #46 mapping and case folding; convert
  U-labels to A-labels (punycode); strip a trailing dot; validate length limits after conversion.
  Purely so that "is domain X already in the list" is a reliable comparison instead of a
  string-literal accident. **It never changes what a rule covers**, and — unlike an earlier draft of
  this document — it does not strip a `www.` label either, for exactly that reason: stripping it
  would fold `www.example.com` and `example.com` onto the same comparison key before scope is even
  decided, making a rule that named one indistinguishable from a rule that named the other.
- **`RuleScope` — what a rule matches.** Either `Apex` (this domain and everything under it) or
  `ExactHost` (this exact name only).

**A rule is `Apex` only when the source's own listed entry is itself the registrable domain
(eTLD+1).** It is never inferred by stripping a prefix. A source entry of `www.example.com` is
**not** eTLD+1 — `www` is a label below the registrable domain — so it normalizes to its own
comparison key and scopes to `ExactHost`, matching only `www.example.com`. A source entry of
`example.com` *is* eTLD+1 and becomes `Apex`, which — because `Apex` covers everything under the
domain — already matches `www.example.com` too, without the two entries needing to share a
comparison key. Because the query side applies the identical `normalize()`, a lookup of
`www.example.com` matches the `ExactHost` rule (or the `Apex` rule if one exists for the bare
domain), a lookup of `example.com` matches only an `Apex` or `ExactHost` rule actually filed under
`example.com`, and a lookup of `cdn.example.com` correctly matches neither unless something names
it or the apex directly.

Registrability is decided against the **[Public Suffix List](https://publicsuffix.org/)**, checked
at build time:

- An entry that *is* a public suffix (`com`, `co.uk`, `s3.amazonaws.com`, `blogspot.com`) is
  **refused as an apex rule outright**. Nothing about the design should ever be able to emit a rule
  covering an entire multi-tenant provider.
- An entry below a public suffix that is not eTLD+1 (`foo.bar.example.com`) is kept as
  `ExactHost`, not promoted.
- An entry that *is* eTLD+1 under a provider suffix (`someone.blogspot.com`, since the PSL lists
  `blogspot.com`) is legitimately `Apex` — that is the tenant's own registrable space, and the PSL
  is precisely what tells the two cases apart.
- **A small, explicit, checked-in denylist of major shared-hosting apexes** backstops the PSL,
  which is community-maintained and incomplete. An entry on that denylist is never `Apex`, PSL or
  not.

#### 2. Union with provenance

Track which source(s) flagged each domain, not just the domain itself. This matters twice: (a) if a
source turns out to be low-quality or disappears, it can be backed out cleanly; (b) if a user
disputes a block, the pipeline can say why it's blocked instead of shrugging.

Provenance is a forensic tool, not a quality control. It tells you which source was wrong *after* a
false positive has already shipped — which is why the union needs the measured gate below rather
than trusting the sources to be individually clean.

#### 3. A measured false-positive gate on every build

A plain union is, by construction, the **maximum-noise** combining rule: every source's false
positives survive into the output, and no source's cleanliness cancels another's mistakes. That
directly contradicts this design's own goal of combining sources without amplifying each other's
noise. Provenance does not fix it; measurement does.

Every build therefore evaluates the merged list against a **known-good negative control set** and
refuses to publish if it fails:

- The control is the **[Tranco](https://tranco-list.eu/) top-N list**, pinned to a specific daily
  release like any other source. Tranco (and its near-equivalents — Cisco Umbrella's popularity
  list, Majestic, Chrome's CrUX) is a pure traffic ranking with **no content-category metadata at
  all**; that isn't a Tranco-specific gap, it's structural to what a popularity ranking measures.
  Because of that, Tranco's top-N genuinely contains adult sites at real traffic volume — measured
  at roughly **1–2.5% density depending on N** (e.g. ~250 in a top-10k slice), not "a handful," and
  the design below accounts for that density rather than assuming it away.
- **What "legitimately adult" means here is decided by cross-source corroboration, not a
  hand-maintained exclusion file measured against Tranco alone.** Every domain that could ever
  register as a control-set hit is, by construction, already present in at least one source's raw
  fetch (`MergedEntry` only exists for domains some `RawEntry` produced), and each source's
  category tag is assigned at the file/directory level, not per domain — see the plan's module 1
  input-shape notes: StevenBlack's `porn` extension, hagezi's NSFW list, and UT1's `adult`
  directory are each fetched as a single trusted unit, so one source's tag can be that source's own
  curation mistake (a domain the file shouldn't have included). Two *independently maintained*
  sources agreeing on the same domain is a much stronger signal than either one alone, without
  needing any classification beyond what the pipeline already fetches:
  - A control-set hit corroborated by **two or more** sources is treated as correctly blocked and
    never counts toward the rate — no review needed.
  - A hit backed by only **one** source lands in a bounded **review queue** — this is the actual
    manual-triage surface, and it is small by construction: the extremely well-known adult sites
    that dominate Tranco's top ranks (the ones any curated list independently includes) clear the
    2-of-3 bar automatically, leaving only the domains a single project happened to flag alone,
    which is both the smaller set and the more plausible place for an actual misclassification to
    live. A small, reviewed, checked-in exclusion file still exists for domains a human has
    confirmed are legitimately sensitive despite lacking corroboration — its expected size shrinks
    to that residual set, not "every adult site Tranco ranks."
  - **Accepted limitation, stated plainly rather than assumed away:** corroboration only helps if
    the sources are genuinely independent. If two of the three sources share upstream provenance
    for a given entry (blocklist projects do sometimes cross-pollinate from each other or from
    shared community submissions), a shared mistake would not be caught this way. This has not been
    verified either way for StevenBlack/hagezi/UT1's adult lists specifically, and is worth checking
    before leaning on the mechanism harder than "reduces the review burden," not "eliminates it."
  - A genuinely independent third-party check remains available as a future strengthening layer,
    specifically because the set needing it is now small: for just the single-source review-queue
    domains (not the full million-entry control set — the earlier obstacle to using a rate-limited
    API at all), a real third-party categorization service could cross-check before a human does.
    Not required for the mechanism above to work; not built.
- The gate is the fraction of the checked control set landing in the review queue (uncorroborated
  and not already excluded) that the merged list blocks. **Starting threshold: 0.5%** — deliberately
  a starting value, to be re-derived from the first real measurement rather than defended as
  correct. A build exceeding it does not publish.
- **A control set that comes back empty or suspiciously small fails outright**, rather than
  measuring a 0% rate against nothing — a truncated download, a parse bug, or a maintenance page
  standing in for Tranco's real list must never look identical to "measured, and clean."
- The build also reports *which* control entries were hit and under which sources, so a regression
  names the source that caused it.

This mirrors `machine-learning`'s `gate.py` release guardrail: the project already refuses to ship a
classifier with no measured quality signal, and a blocklist with no measured false-positive rate is
the same failure in a different package.

#### 4. Personal-name triage

The heuristic described under [Domain operators](#domain-operators) runs here, over the merged set,
routing matches into a review queue. It changes no verdict on its own.

#### 5. User-level allowlist override sits on top, unconditionally

False positives are inevitable with any aggregated list, and there needs to be a local,
no-appeal-required way to unblock a domain the household actually trusts. This decision fixes the
*precedence* (allowlist checked before the shipped list, always wins) and the *ownership* (local to
the device, never uploaded or reconciled against anything this pipeline produces); it deliberately
leaves the exact matching semantics and persistence mechanics to `net-shield`'s own plan, since
that's the crate that actually evaluates a query against both. The full precedence model, including
where FST-sourced rules sit relative to `net-shield`'s existing explicit rules, is specified in
[the plan](../components/domain-blocklist/plan.md) module 6.

This is a *user's* remedy. It is not an operator's remedy — see the dispute channel above.

#### 6. Don't blend categories silently

A source's "adult" category and its "gambling" or "dating" category are different products, and a
domain legitimately can be both — an aggregator listing under "adult" and a different one under
"gambling" are two true statements about the same domain, not a conflict. `MergedEntry` therefore
carries a **set** of categories, never a single value that one merge step could silently overwrite
with the other. A build ships a domain if any one of its categories is in that build's configured
set; it does not require all of a domain's categories to match. The exact type
(`MergedEntry.categories: Vec<Category>`) and the provenance ID's matching field are in
[the plan](../components/domain-blocklist/plan.md) module 2 and module 4.

### Publish gates

Signing proves an artifact came from this pipeline. It says nothing about whether the pipeline
produced a *sane* artifact. Six checks stand between a completed build and a published bundle, and
**each one requires explicit human sign-off to override — never an automatic publish**:

| Gate | Refuses to publish when | Why |
|---|---|---|
| Liveness canary | any canary domain returns anything but its expected verdict | a filtering or hijacking resolver would otherwise prune the list to nothing |
| Shrinkage | entry count drops **>10%** below the previous published build, or below an absolute floor | catches a bad sweep, a source that silently emptied, a parser regression |
| Growth | *added* entries exceed **10%** of the previous published build | catches a compromised or mis-bumped upstream injecting bulk entries |
| False-positive rate | the uncorroborated, unreviewed review-queue rate against the control set exceeds its threshold, or the control set itself is suspiciously small | catches noise amplification across the union, and a broken control-set fetch masquerading as a clean measurement |
| Artifact size | the built `.fst` exceeds its size budget | catches unbounded growth before a device pays for it |
| License | any source that contributed entries has no license snapshot, an ambiguous (duplicated) one, or one off the allowlist | re-checks the [fetch-time license gate](#license-gate) at publish time so it can't be bypassed or weakened in between |

The percentages are starting values chosen to be tight enough to catch a category error and loose
enough not to fire on ordinary churn; like every other constant here they are to be re-derived once
several real builds exist.

#### Artifact size budget

**Ceiling: 32 MB for the `.fst` file.** A build that exceeds it does not publish without a
signed-off exception.

The precedent is `image-sandbox`'s 15 MB model budget — this project states download/footprint
ceilings and enforces them rather than discovering them on a user's device. 32 MB is chosen against
a mobile download and a mmap-backed on-disk footprint, with headroom over the ~4 MB the planning
assumption implies, precisely because the planning assumption is untrusted: if the real merged
count turns out to be several million (UT1's `adult` category alone is plausibly a multiple of the
assumed figure), the budget is what surfaces that as a decision rather than a surprise.

### Liveness revalidation: DNS-only, centralized, cached with a TTL

Liveness is checked by **DNS resolution only** — never a full HTTP fetch, and never ICMP. ICMP is
the wrong signal regardless of cost: most hosting today sits behind a CDN or load balancer that
either doesn't route ICMP to the origin or answers ping from an edge node unrelated to whether the
site is actually up. DNS-only also satisfies the legal boundary above, since it never performs an
application-layer request that could retrieve a page body.

The verdict is a three-way match, not a boolean, because DNS has more failure modes than "up" or
"down":

| Response | Verdict | Effect on next build |
|---|---|---|
| A/AAAA record, or a CNAME chain resolving to one | Alive | kept |
| NXDOMAIN | Dead | pruned |
| NODATA, SERVFAIL, REFUSED, timeout, or a malformed/unparseable response | Unknown | **kept, unchanged** |

Only `NXDOMAIN` prunes an entry. Everything else that isn't a clean positive answer is `Unknown`,
not `Dead` — a resolver having a bad moment, a transient SERVFAIL, or a timeout must never be able
to silently shrink the blocklist. An entry that stays `Unknown` across repeated sweeps is worth
surfacing in the pipeline's own build metrics as a thing to look at, not a reason to change its
inclusion.

The table above describes one query's result; a liveness check runs **two** (A and AAAA), which can
disagree — one family returning `NXDOMAIN` while the other times out is not a rare case, it is the
ordinary shape of a dual-stack failure. The two combine in this order: **`Alive` if either lookup
resolves** (one working address family is enough); otherwise **`Dead` only if both unambiguously
return `NXDOMAIN`**; otherwise **`Unknown`**. A mixed `NXDOMAIN`/`Unknown` pair is therefore
`Unknown`, never `Dead` — a resolver failing to give a straight answer on one address family must
not let the other family's clean `NXDOMAIN` carry the domain to pruning. A domain checked for the
first time that comes back `Unknown` is **included by default**, same as an already-cached
`Unknown` entry: "we couldn't tell yet" is never itself a reason to omit a domain.

**This runs centrally, in the list-build pipeline — never on an end-user device.** Client daemons
only ever consume an already-pruned, signed list; they do not run their own liveness checks and this
cost is never charged against a user's mobile data.

#### The resolver, and the canary check that guards it

**The pipeline must run against a documented, verified non-filtering resolver.** This is a hard
requirement, not a configuration preference, and it is the single most dangerous failure mode in the
whole design: a resolver that filters adult content — Cloudflare's `1.1.1.3` family filter,
OpenDNS FamilyShield, and a great many corporate and CI-provider networks — returns NXDOMAIN or a
sink address for essentially every domain on this list. Every entry reads as `Dead`. The pipeline
prunes the list to near-zero, signs the result, and ships it. Nothing in the process looks broken.

Three mechanisms, all required:

**A named, non-filtering resolver.** The pipeline is configured with an explicit resolver address
and does not use the host's system resolver, which on a CI runner is whatever the provider
happens to supply. The default is Cloudflare's unfiltered `1.1.1.1` / `2606:4700:4700::1111`,
explicitly *not* the `1.1.1.2` (malware) or `1.1.1.3` (family) variants. Any substitute must be
documented and canary-verified the same way.

**Two-resolver corroboration before pruning.** A `Dead` verdict from a single resolver is never
sufficient on its own — a domain is only actually pruned once a *second*, independently-configured
resolver (a different operator, a different anycast network) also produces `Dead` for it. An
earlier design tried to establish that same trust from DNSSEC authentication (the AD bit) on a
single resolver's own answer instead, and that was measured, live, to be the wrong mechanism: most
large TLDs — `.com`/`.net`/`.org`/`.xxx` included — sign with NSEC3 opt-out, under which a
validating resolver cannot construct an authenticated proof of non-existence for an unsigned
delegation at all, so requiring it made `Dead` nearly unreachable rather than safer to reach. See
[`packages/domain-blocklist`'s `liveness::corroboration`
module](../../packages/domain-blocklist/src/liveness/corroboration.rs) for the implementation and
the full reasoning.

**A canary check before every sweep**, over a small fixed set of control domains that has nothing to
do with adult content, checked in both directions:

- Several **known-always-alive, definitely-non-adult** domains (e.g. `iana.org`,
  `root-servers.net`, a major-CDN hostname) must each return `Alive`.
- At least one **reserved name guaranteed not to resolve** (RFC 2606 / RFC 6761 — `invalid.` and a
  name under `test.`) must return `Dead`. This second direction catches the resolver that
  NXDOMAIN-rewrites to a wildcard sink, which would produce the *opposite* failure: nothing ever
  prunes, and the list quietly stops being maintained. A real deployment should also mix in a
  **per-run random-nonce control** under a real, currently-registered zone (`nonce_dead_control` in
  the same module) — but never a zone hosted behind a provider that answers a nonexistent name with
  NODATA rather than NXDOMAIN ("compact denial of existence"; Cloudflare-hosted zones, including
  `example.com`, do this and were measured to break this exact control). See
  `nonce_dead_control`'s doc comment for the full trap and how to verify a candidate zone before
  using it.

**If any canary returns anything other than its expected verdict, the entire sweep aborts.** Every
verdict from that run is treated as untrusted and discarded — not merged, not cached, not written
back. Nothing is published. A partially-completed sweep is not salvaged, because there is no way to
know at which query the resolver started lying.

#### Open gap: the canary cannot see a resolver that filters only this category

The control set above is deliberately drawn from **outside** the category this pipeline sweeps —
`alive_controls` must be "definitely-non-adult", `dead_controls` are reserved names or a per-run
nonce (see [`packages/domain-blocklist`'s `liveness::canary`
module](../../packages/domain-blocklist/src/liveness/canary.rs) for the implementation). That
shape catches an indiscriminate sink and an indiscriminate NXDOMAIN rewrite — a resolver that lies
about *everything*. It cannot catch a resolver that lies about *only this category*.

A resolver that filters adult content specifically, and nothing else, answers every alive control
honestly (they're all non-adult, by construction), answers every dead control honestly (reserved
names and a nonce aren't in the category either), and then silently NXDOMAINs the entire real
sweep. The canary passes. The sweep prunes the list to near-zero. This is not a hypothetical
edge case — it is the *ordinary* shape of the exact failure this mechanism exists to catch, and it
is documented as the single most dangerous failure mode two paragraphs above this one. Concrete,
real deployments with exactly this shape: UK ISP-level content filtering, Italy's AGCOM blocking
regime, a CI provider's "family-safe" network default, and Cloudflare's own `1.1.1.3` family-filter
resolver variant (already named above as the resolver this pipeline must *not* use — the canary as
currently shaped would not detect accidentally ending up on it anyway).

**The fix is a known, standard technique the canary does not yet apply:** an *in-category* alive
control — a domain a category-targeting filter would block, but which is not itself sensitive
content and is safe to check into a public repository. Filtering vendors publish exactly this kind
of test hostname for other categories (Cisco/OpenDNS's malware/phishing test domains are the
well-known pattern); an equivalent for the adult-content category, sourced from a filtering
vendor's *current* published documentation at deployment time, would close the gap the same way.

**Why this repository cannot supply that value itself, and does not attempt to:**

1. This project's own conventions (`CLAUDE.md`) forbid checking adult-content domains — real or
   placeholder — into this public repository, even as a test fixture, and even for a security
   control whose job is to detect adult-content filtering. That rule is not relaxed for this case.
2. A vendor-published test domain must be taken from that vendor's *current* documentation, not
   from a model's training-data memory or an engineer's recollection — a stale or misremembered
   hostname is worse than an honest gap, since it would silently stop testing anything the moment
   the vendor retires or repurposes it.

Both constraints point the same way: **sourcing an in-category control is a deployment-time
operator responsibility, not something this codebase can discharge.** No code change is needed to
*support* it — `CanaryConfig::new`'s existing `alive_controls: Vec<String>` parameter already
accepts any number of domains from any category; there is nothing about its shape that privileges
"non-adult" domains over any other kind of alive control. The gap is entirely in the data supplied
at construction time, which belongs to `cli` (module 7, unbuilt). When that module is built, its
operator should source one or more current, vendor-published, category-relevant test domains from
the filtering vendors' own current documentation and inject them via `CanaryConfig::new`'s
`alive_controls` parameter alongside the existing non-adult controls — never by inventing a domain
name, and never by drawing one from any adult-content list this repository would ever check in.

Until that sourcing happens, this canary catches the crude failure (indiscriminate sink,
indiscriminate NXDOMAIN rewrite) and **not** the targeted one — record this as a known, accepted
limitation of every sweep run before that gap is closed, not a solved problem.

#### Cadence, TTL, and pacing — three separate numbers

These were previously one number, which made the promised steady-state saving imaginary: if the
recheck TTL equals the run cadence, every cached entry is due on every run, and a run a minute early
or late swings the check volume wildly.

- **Run cadence: monthly.** How often the pipeline builds and publishes.
- **Dead-entry recheck TTL: 3 cadences (~3 months).** A domain cached as `Dead` that still appears
  in a source refresh is only rechecked once its cache entry is older than the TTL. With TTL at
  three times the cadence, roughly **one third** of the previously-dead cohort is due on any given
  run — which is where the steady-state saving actually comes from. A genuine revival (domain
  resold or re-registered) is still caught, within one TTL.
- **Sweep pacing: spread over a ~24-hour window**, not the full month and not one burst. The doc
  previously implied both 0.4 qps and ~280 qps without choosing. The chosen model is a steady low
  rate across a bounded window: a cold sweep of ~1,000,000 domains is ~2,000,000 queries (see
  below), which over 24 hours is **~23 qps** — low enough to be a polite, unremarkable load against
  a public resolver, bounded enough that a sweep's results all describe roughly the same moment in
  time. Steady-state sweeps are a fraction of that and finish sooner.

A domain cached as **alive**, or brand new to every source, is checked on every sweep.

#### Measured 2026-08-15/16: the sizing and pacing numbers above are stale, and the fix is not a bare qps bump

The `~23 qps` / `~1,000,000 domains` / `~24 hours` figures above were a sizing estimate made before
`sources` (module 1) or `cli` (module 7) existed. With both now built, three things were measured
directly against real sources and real resolvers rather than re-derived on paper.

**The corpus is ~4.75x bigger than assumed, using less than the full source list.** Fetching and
merging just StevenBlack's porn-only hosts file, Hagezi's NSFW wildcard list, and UT1's adult and
gambling categories (4 of the plan's 6 planned source/category lists — missing UT1 dating and
Hagezi's other tiers) produced **4,752,920 merged unique domains**, not ~1,000,000. UT1 adult alone
is 4,599,280 raw lines before merge. The `~23 qps` figure was sized to sweep ~1,000,000 domains in
24 hours; the real number, even undercounted, needs roughly **4.75x that dispatch rate** to hit the
same 24-hour window — this is the honest reason a qps increase is on the table at all, not a desire
for a faster sweep for its own sake.

**The per-chunk `.collect()` barrier in `sweep.rs`'s dispatch loop was investigated and cleared at
the shipped default.** A real dead/lame domain can cost close to the client's ~15-second worst-case
query budget (`liveness/net.rs`'s `TOTAL_QUERY_BUDGET`), and because each `canary_every`-sized chunk
must fully complete (`.collect()`) before the next chunk's canary re-check and dispatch can begin,
one straggler taxes its whole chunk. At `--canary-every 50` (a value chosen only to get more canary
observations in a quick local test, never a real setting) this cost 601 real domains a 1.83x wall-
clock overhead (95.7s actual vs. 52.3s naive-ideal) against real dead/lame entries from the corpus
above. At the CLI's actual shipped default, `--canary-every 2000`, the identical sample and domain
mix measured **1.03x** — the straggler cost is real but amortizes to near-nothing once a chunk is
production-sized. No code change was needed here; the concern was specific to an artificially small
test value, not the shipped default.

**A naive 5x qps bump (matching the 4.75x corpus-size gap, landing at `--qps 57.5` / ≈115 raw
queries/sec) was tested against a real, canary_every-sized chunk of the merged corpus and showed
real degradation, not just a wall-clock cost.** At the current default (`--qps 11.5`,
`--concurrency 50`), a 2001-domain real sample resolved with **0.67–0.7% `Unknown`** verdicts. At
5x qps with concurrency scaled to match (`--concurrency 200`, keeping headroom per Little's Law),
the identical corpus sample produced **10.6% `Unknown`** — over 15x worse, not proportional — broken
down as `Timeout: 116 (5.8%)`, `UncorroboratedDead: 88 (4.4%, the two resolvers disagreeing on an
NXDOMAIN)`, `ServFail: 4`, `NoData: 4`. Local resource exhaustion was checked and ruled out as the
cause: the test machine's `ulimit -n` was effectively unbounded (1,048,576) and the breakdown
contains zero `Malformed`/`Transport` entries, which is what socket exhaustion or a client bug would
produce instead. What's left — real timeouts and cross-resolver disagreement — is the signature this
document's earlier "watch the canary" reasoning predicted a real ceiling would look like.

**This result does not tell us where the real ceiling is, and must not be read as one.** It was
measured from one development machine's residential/office network and its own, unrelated IP
reputation history at 1.1.1.1/8.8.8.8 — a VPS's datacenter uplink and IP history could show a higher
ceiling, a lower one, or a different failure signature entirely. What it *does* tell us: the
instinct to just multiply the default qps by the corpus-size growth factor and ship that as the new
default is not safe, because the first real test of that exact jump produced a real degradation
signal, not a clean pass. **The correct next step is the incremental, canary-monitored qps ramp this
document already prescribes, run against the actual deployment host** (starting at the current
default, stepping up, watching the `Unknown` breakdown — specifically `Timeout` and
`UncorroboratedDead` rates, not just the aggregate — for degradation before each step), not a single
jump sized to the corpus-growth factor. The CLI's `--qps 11.5` / `--concurrency 50` defaults are
therefore left unchanged pending that ramp; changing them now would be encoding an untested guess
in the exact place this project's own review history keeps finding them (the image-sandbox
threshold, the FST floor, the DNSSEC-authentication requirement — see `CLAUDE.md`'s `image-sandbox`
and `domain-blocklist` rows).

**Batching multiple domains into one DNS query is not practically available** — RFC 1035's QDCOUNT
field permits it in principle, but essentially no real-world resolver answers more than one question
per message. The available lever is **concurrency** (many in-flight query packets at once,
rate-limited to the chosen qps), not batching.

#### Measured 2026-08-16: the incremental qps ramp, run against the real deployment VPS

The section above prescribed running the qps ramp against the real deployment host rather than
guessing from the dev-machine result. That ramp was run on the production VPS (4 vCPU, datacenter
uplink), each step a real `--sample` slice (a new CLI flag added for exactly this purpose — see its
own doc comment in `cli.rs`) of the actual fetched 4.75M-domain corpus, against real 1.1.1.1/8.8.8.8
resolvers, canary passing at every step:

| qps / concurrency | sample | Unknown% | Timeouts | UncorroboratedDead |
|---|---|---|---|---|
| 11.5 / 50 (shipped default) | 2,000 | 2.35% | 0 | 0 |
| 23 / 100 | 4,000 | 1.98% | 5 | 0 |
| 46 / 200 | 8,000 | 1.80% | 10 | 0 |
| 92 / 400 | 16,000 | 1.43% | 14 | 0 |

No degradation signal anywhere in this ramp, unlike the dev-machine measurement above, which broke
down at 57.5 qps (10.6% Unknown, 15x worse). This VPS's datacenter uplink and IP reputation history
tolerate roughly 8x the shipped default cleanly — confirming the section above's own caveat that a
VPS could show a materially different ceiling than a residential/office network, in either
direction. **This does not change the shipped CLI defaults** — the defaults are a safe floor for an
unknown deployment host, and this result is evidence for *this* host's operator to raise `--qps`
explicitly for its own scheduled runs, not a reason to move the shipped default itself. A full
92 qps / 400-concurrency production sweep against the complete real corpus was then launched from
this measurement, detached from the invoking shell (`setsid`/`disown`, reparented to PID 1) so it
survives an SSH disconnect, logging to `dbl-run/logs/production.log` on the host.

Two real defects were found and fixed while getting the real (non-fixture) fetch path to run at
all, neither previously exercised end-to-end against live sources:

- `SourceConfig.pinned_revision` for the two GitHub sources and the three UT1 categories were
  literal `"UNPINNED"` placeholders (`main.rs`'s own doc comment: "left as a placeholder ... so a
  real fetch fails loudly and closed"). Moved to real pins via the reviewed pin-bump process this
  file's "Pinned revisions, never floating HEAD" section requires: StevenBlack `35db0ae9...`,
  hagezi `3975aafc...` (both `git commit` SHAs baked into the raw-content URL path), and the three
  UT1 categories to `last-modified=Sat, 15 Aug 2026 20:50:17 GMT` (UT1 publishes no ETag;
  `fetchers::ut1::pick_revision` falls back to the `Last-Modified` header, prefixed — the prefix
  format was undocumented outside the function body and cost one failed real-fetch attempt to
  discover).
- `fetchers::ut1::MAX_MEMBER_BYTES` was 64 MiB on the doc comment's claim that "UT1's largest
  `domains`/`urls` member measures a few MB" (2026-08-15 assumption audit). A real fetch of
  `adult.tar.gz` hit the ceiling: `adult/domains` is **124,529,768 bytes** (4,599,280 lines), not
  "a few MB" — the assumption audit measured the wrong file, or measured before UT1's adult list
  grew to its current size. Raised to 256 MiB (~2x headroom over the measured real file), documented
  in the constant's own doc comment as measured-wrong rather than silently widened.

#### The TTL cache needs a real home

The cache is a multi-million-row `domain → {last_checked, verdict, sources}` table that must survive
between runs. **A CI runner's filesystem is ephemeral and cannot hold it.** The pipeline therefore
stores the cache as a **versioned object in dedicated persistent storage** — a private object-store
bucket, or a release asset on the pipeline's own distribution channel, or a CI artifact cache with
retention configured well beyond the TTL — fetched at the start of a run and written back at the
end.

It is **not committed into the public repository tree**, consistent with this project's existing
convention of gitignoring corpora and model artifacts. A run that cannot load the cache starts cold
and does a full sweep; a run that cannot write it back fails loudly rather than silently discarding
three months of accumulated state.

#### Measured 2026-08-16: the cache's in-process representation — redb, not a `HashMap` blob

The persistent-storage question above (*where* the cache lives between runs) is separate from *how*
it's held while a run is using it, and the second question turned out to be the one actually driving
memory cost. `cache_store.rs` currently loads the whole cache into a `HashMap<String, CacheEntry>`
for the sweep's duration and reserializes the entire map to one bincode blob on every checkpoint.
Measured live on the production VPS mid-sweep: **2.71GB RSS** for the real ~4.75M-domain corpus —
about 569 bytes per entry, all of it Rust heap (a separately-allocated `String` per key, `HashMap`
bucket-array slack below 100% load factor, struct alignment padding), none of it structurally needed
by the ~25-byte domain strings and ~13–21-byte `CacheEntryDto` values being stored.

The fix is to stop holding the cache as a live Rust collection at all and read/write it through
[redb](https://docs.rs/redb) 4.1.0 — a pure-Rust, single-file, transactional embedded key-value
store ([repository](https://github.com/cberner/redb)), with the domain string as key and a
bincode-encoded `CacheEntryDto` as value, matching the DTO shape already defined in
`cache_store.rs`. **Correction (2026-08-16, same day, after implementation and measurement): redb
is not mmap-backed.** An earlier draft of this section claimed it was, on the strength of the crate
description alone rather than reading its actual storage code; that claim was wrong and is corrected
here rather than left standing. redb 4.1.0 keeps its own in-process page cache
(`redb::Builder::set_cache_size`, defaulting to 1 GiB) over a single file it manages with its own
I/O, not a `mmap()`ed region the kernel pages in the sense the rest of this section originally
assumed. This matters for the "how does this bound memory" argument below, which the mmap framing
got backwards — see the corrected "Net effect" paragraph. This was evaluated rather than assumed,
using real data from the actual deployment host rather than synthetic benchmarks:

- **VPS specs** (`ssh` to the production Contabo box): 7.8GB RAM, of which only ~4.0GB was
  "available" (free + reclaimable buff/cache) while the current sweep held its 2.71GB `HashMap`
  resident alongside ~1GB from unrelated tenants on the same box (n8n, an `openclaw` gateway,
  `dockerd`, `tailscaled`). 4 vCPUs. Disk reports `ROTA=0` but is `QEMU HARDDISK` — a virtio block
  device on a KVM host — which does **not** prove local NVMe; that flag alone cannot distinguish
  local flash from network-attached block storage.
- **Real disk latency**, measured directly rather than inferred from the rotational flag: a 500MB
  scratch file, `posix_fadvise(..., POSIX_FADV_DONTNEED)` to evict it from page cache, then 500
  `O_DIRECT` random 4KB `pread`s to force genuine disk I/O. Result: **p50 405μs, p90 1.37ms, p99
  7.4ms, max 16.9ms, mean 744μs** — an order of magnitude worse than local NVMe's typical tens of
  microseconds, consistent with network-backed virtual storage rather than local flash. This matters
  because it sets the cost of a redb page fault: roughly 4,000–7,000x a `HashMap` hit on average, and
  70,000x+ at the tail, if a lookup actually has to go to disk.
- **Real corpus**, not a synthetic one: the project's actual pinned sources (StevenBlack `porn-only`,
  hagezi `nsfw-onlydomains`, UT1 `adult`/`gambling`/`dating`, at the same pins recorded in the
  "Measured 2026-08-16" ramp section above) were fetched and normalized on the VPS itself, producing
  **4,767,348 distinct domains** — matching the corpus-size figure already measured elsewhere in this
  document. Average domain length: 25.7 bytes.
- **Real redb file size**, built from those real keys plus the real `CacheEntryDto` value shape
  (synthetic verdicts, since no on-disk `production.bin` exists yet on this pre-checkpointing binary —
  see the "Left explicitly undone" list in the plan's module 7 for why): **515MB, compacted**. For
  comparison, the same data as a flat (non-B-tree) bincode `HashMap` blob would run an estimated
  ~228MB (8-byte length prefix + ~26-byte key + ~8-byte timestamp + ~4-byte enum discriminant +
  ~2-byte average `Option<u64>` per entry) — so redb's B-tree page/checksum overhead costs roughly
  **2.25x** the flat format, but the absolute number is what matters: 515MB against 4.0–7GB of
  available RAM is a comfortable fit with room to spare, not the 2.5–3.5GB this document originally,
  and wrongly, guessed before measuring (that guess anchored on the inflated 2.71GB in-memory figure
  as its baseline, which was already the wrong number to add B-tree overhead on top of).

**Net effect of the swap, corrected:** steady-state resident memory for the cache drops for the
boring reason that Rust's live `HashMap<String, T>` carries per-object heap overhead redb's packed
encoding doesn't — but that drop is bounded by **an explicit cache-size budget passed to
`redb::Builder::set_cache_size`, not by OS reclaim of mmap pages**, since no such pages exist. redb's
page cache is process-owned anonymous heap the kernel can, at best, swap under pressure — it cannot
drop it the cheap way it drops a clean file-backed mapping, so leaving the 1 GiB default in place
(as the first implementation of `CacheStore::open` did, before this correction) trades one unbounded
allocator (a resident `HashMap`) for a differently-shaped one. The actual lever, and the one this
section's "bounding cache RSS by a page budget instead of corpus size" goal describes, is choosing
`set_cache_size` explicitly — `cache_store::DEFAULT_CACHE_SIZE_BYTES` (128 MiB) is what
`CacheStore::open` now passes; see that constant's own doc comment for the measurement that picked
128 MiB specifically. This is graceful in a different sense than originally claimed: it is a hard,
predictable ceiling set at open time, not a hope that the kernel reclaims something under pressure.

**What implementing this needs to account for, not treated as free:**

- **Batched write transactions**, replacing the current whole-map reserialize-on-checkpoint. A
  transaction per domain would fsync every write (~1–10ms each per typical SSD fsync latency,
  unmeasured on this specific VPS) and cap sweep throughput far below what's needed at 4.75M domains;
  batch commits every `--checkpoint-every` domains (mirroring the existing checkpoint cadence) rather
  than per-entry.
- **Periodic compaction.** The 515MB figure is a freshly compacted, single-writer build — it does
  **not** measure steady-state size after months of incremental updates (`last_checked` bumps,
  verdict flips) fragmenting the B-tree across repeated commits. This needs its own measurement
  before shipping a maintenance cadence; call it an open gap, not an assumption to build against
  silently.
- **On-disk format migration.** redb's file format is not bincode-`HashMap`-compatible, so an
  existing `cache.bin` written by the current `cache_store::save` cannot be opened directly by a
  redb-based reader. Either a one-time conversion pass or accepting that in-flight caches finish
  their current cycle on the old format and new caches start clean is a decision the implementing
  PR must make explicitly, not one to discover mid-migration.
- **Due-list key ordering** is a free lever worth taking at the same time: today's due-domain
  traversal order has no particular relationship to redb's key-sorted B-tree layout, so a sweep that
  visits domains in sorted-key order gets meaningfully better page locality (and fewer of the
  400μs–17ms cold-fault lookups measured above) than visiting them in whatever order the corpus
  happened to be merged in, at no cost beyond sorting the due list once per sweep.

This section documents the decision and the measurements backing it; the implementation
(`cache_store::CacheStore`, `CacheBackend`, and a `CacheBackend`-generic `sweep::run_sweep_streaming`)
is built — see `docs/components/domain-blocklist/plan.md`'s module 7 entry for what shipped and the
one narrowing it cost (a dry run can no longer cheaply seed itself from an existing cache file's
exact prior state, so it always starts cold now).

**Measured 2026-08-16, same day: the stress-harness comparison, its first (wrong) explanation, and
the correction.** Running the streaming sweep's own 4.8M-synthetic-domain stress test
(`sweep::tests::stress_test_streaming_sweep_avoids_holding_the_full_corpus`, already in this repo)
both ways — a `HashMap`-backed cache and a `CacheStore`-backed one, same process shape, same machine
— gave final RSS 1353.8MB vs. 1126.7MB: a real ~17% reduction, not the ~5x this section's own
production-VPS estimate projected. **That comparison's explanation was wrong**, not just optimistic:
it attributed the shortfall to "dirty/recently-touched mmap pages stay resident until something
evicts them," reasoning the production VPS's shared-tenant memory pressure would let the kernel
reclaim more than a quiet single-process stress test would. There are no mmap pages — see the
correction above — so there is nothing for the kernel to reclaim either way, on a quiet dev machine
or a busy VPS; a 1 GiB redb cache is 1 GiB of anonymous heap regardless of who else is running.
**The real cause was the unbounded 1 GiB `redb::Builder` default itself, left unset by the first
`CacheStore::open`.** Fixed by passing `set_cache_size(cache_store::DEFAULT_CACHE_SIZE_BYTES)`
(128 MiB — see that constant's doc comment) explicitly. Re-measured on the same stress harness,
same machine, after the fix: **rss_final=942.8 MB**, cache file still 514.0MB on disk (unchanged —
this is a resident-memory fix, not a format change). That's a real reduction from the 1 GiB-default
redb run (1126.7MB → 942.8MB, ~16%) and from the `HashMap` baseline (1353.8MB → 942.8MB, ~30%), but
notably **not** as large as `redb::Builder`'s cache-size delta alone would suggest — a separately
reproduced run of the same before/after comparison (see `cache_store::DEFAULT_CACHE_SIZE_BYTES`'s
own doc comment) measured a larger gap (1126.7MB → 613.9MB) under conditions not fully pinned down
here; the two runs agree on direction and rough magnitude but not on the exact number, most likely
because this stress test's own ~516MB entry-corpus build-then-drop overhead (documented in the
streaming-corpus pass above as **not actually released back to the OS by macOS's allocator**) sits
underneath both numbers and doesn't cancel out cleanly between runs. Reported honestly rather than
reconciled: the bounded-cache-size fix is confirmed real and correctly targeted at the right root
cause, but this repo does not yet have a single trusted number for its exact size on this dev
machine, and — per every other caveat this document already carries about macOS vs. the Linux VPS
target — neither number has been confirmed against the real deployment host. Unlike the original
mmap-based reasoning, though, the fixed mechanism does not depend on kernel reclaim behavior varying
by host at all: `set_cache_size` is an explicit, host-independent ceiling, so the VPS should see the
same order-of-magnitude reduction this stress harness does, not a host-specific bonus the way the
original (wrong) mmap story implied. A real multi-hour sweep against the production VPS is still the
only way to confirm the exact number there.

#### Egress: a documented estimate, not a measurement

The previous figures undercounted by roughly 2×, because **a liveness check is not one query**. A
and AAAA are separate QTYPEs requiring separate queries, and `QTYPE=ANY` is not a shortcut —
RFC 8482 documents that major resolvers refuse or minimize it precisely to stop this use.

- A DNS query (IPv4 header + UDP header + DNS header + question) for a typical hostname is roughly
  **65–75 bytes** on the wire.
- A response (positive A/AAAA record, or NXDOMAIN-with-SOA) is roughly **100–180 bytes**.
- Round trip per query: ~150–250 bytes. **Per domain, at two queries: ~300–500 bytes**, call it
  ~400 bytes as a working average.
- Cold sweep of 1,000,000 domains (2,000,000 queries): **~300–500 MB**, spread over the 24-hour
  sweep window.

**These are estimates and should be presented as such.** They exclude retries on timeout, EDNS0
option overhead, and CNAME-chain following where a resolver's answer requires a follow-up query —
each of which adds queries, not bytes-per-query. The real number is above this range, not below it.
It remains trivial for the pipeline infrastructure it runs on, which is why the estimate is good
enough to design against and not worth measuring precisely up front.

#### What DNS liveness does and does not tell you

**DNS liveness measures registration, not content.** A parked, squatted, or repurposed domain still
resolves and still reads as `Alive`; nothing here confirms the domain is still serving adult
content, and confirming it would require exactly the application-layer fetch the legal boundary
forbids. That limitation is accepted, not worked around.

The converse gap is narrower but real: a domain that lapses and is re-registered by the same
operator is pruned as `Dead` on one sweep and only re-enters the list when a source republishes it,
leaving **up to one cadence period unblocked**.

Both are accepted in the same direction, and for the project's stated reason: false negatives are
the budget, false positives are the price. Keeping a stale-but-harmless entry costs a few bytes in
a compact filter; removing a live one costs a bypass. This is exactly why `Unknown` never prunes
and why only an unambiguous NXDOMAIN does.

#### What pruning actually buys

Pruning's original justification — smaller updates — **does not hold under the chosen artifact
format.** The FST ships as one monolithic file with no delta or patch mechanism, so removing 2% of
entries does not shrink any delivered download by a meaningful amount; the client re-downloads the
whole file either way.

The justifications that do hold:

- **Bounding artifact size over years.** These lists accumulate monotonically — sources add and
  essentially never prune. Without a pruning step the artifact grows without limit and eventually
  collides with the size budget above.
- **Reducing stale false-hit surface.** A dead domain that is later re-registered by an unrelated,
  innocent party becomes a false positive that nothing else in the pipeline would catch.

**Delta/patch distribution is a documented future improvement**, not part of this design. If it is
built, pruning's original justification becomes true again.

### On-device storage and lookup

The distributed artifact is a **minimal-DFA finite-state transducer (FST) over reversed domain
labels** (Rust's `fst` crate — pure Rust, no native C/C++ cross-compilation burden, unlike e.g.
`marisa-trie`), not a plain text list or a runtime-only in-memory trie:

- **Non-lossy** — an exact map from key to a small provenance/scope ID, not a probabilistic
  structure. No Bloom filter as the primary structure: a Bloom filter's false positives fail the
  non-lossy requirement outright.
- **Compact** — an FST shares both common prefixes *and* common suffixes across entries, unlike a
  plain trie which only shares prefixes. Domain data compresses well under this (`.com`, common
  subdomain patterns, common second-level names all collapse into shared DFA states).

  **The commonly quoted 2–4 bytes/entry figure describes a bare set, and this design builds a
  `Map`.** States whose output values differ cannot be merged, so a per-key value degrades exactly
  the suffix-sharing the estimate depends on. The mitigation is to make the value space small:
  canonicalize the *combination* of `{sources, category, scope}` into a small enumerated set of
  provenance IDs — most domains will share one of a few dozen combinations — rather than treating
  every domain's provenance as unique. That bounds how much the value hurts compression, but it
  does not restore the bare-set figure. **The bytes-per-entry number must be measured against a
  real build, not assumed**, and the [artifact size budget](#artifact-size-budget) is the actual
  gate, not the estimate.
- **Reversed labels turn suffix matching into prefix matching.** `example.com` is stored as the key
  `com.example`. `www.example.com` from a source is stored as its own key, `com.example.www` —
  `normalize()` does not strip `www.` (see the normalization section above), so a `www` host keeps
  its own identity in the key space and an `ExactHost` rule filed under it matches only that host.

  A query is answered by exact lookups at each label boundary from shortest to longest (`com`,
  `com.example`, `com.example.cdn`, …), first hit wins. A hit whose scope is `Apex` covers
  everything below it; a hit whose scope is `ExactHost` matches only when the query key equals the
  stored key exactly. **Cost is one exact lookup per label — bounded by label count, typically
  single digits and formally bounded by DNS's 253-byte name limit — and each lookup is O(key
  length), not O(1).** That is the honest bound; the previously asserted "~4–5 lookups" was a
  typical case stated as a limit.

  The general prefix-streaming capability of an FST is available for secondary uses (a "why is this
  blocked" debug view, allowlist-manager autocomplete, version-diffing), with one hazard that must
  be handled: **an unanchored prefix query for `com.example` also matches `com.examplezzz`**
  (i.e. `examplezzz.com`), because nothing stops the match at a label boundary. Any prefix query
  used for these purposes must therefore be **anchored to end exactly at a label boundary** — the
  query prefix includes the trailing separator byte — so a prefix can never match past a partial
  label. This hazard is why prefix-streaming is a secondary-use convenience and not something the
  hot path uses.

#### mmap: the tradeoff, stated honestly

The artifact is **mmap-backed, not eagerly loaded into a heap buffer**. The previous framing claimed
both "microsecond-scale, no meaningful penalty" and "reclaimable under memory pressure," which
cannot both hold: eviction is precisely what makes the next access expensive. You do not get a
mapping that is always cheap *and* evictable.

The real trade:

- mmap accepts a **rare, bounded tail latency** — a genuine major fault after a page has been
  evicted is a real disk read, low milliseconds on flash storage — in exchange for the process never
  being OOM-killed over an unreclaimable heap buffer of the same bytes.
- That trade is correct because **losing the whole filter is worse than one occasionally slow
  lookup.** An Android background process killed for holding tens of megabytes of unreclaimable heap
  stops filtering entirely; a process that takes a few milliseconds on a cold page keeps filtering.
- Steady-state lookups still hit resident pages and are cheap. The cost is a tail, not a baseline.

Two things make that tail rarer and bounded:

**Verification warms the cache for free.** Signature verification hashes the whole `.fst` file, so it
reads every byte sequentially at load/update time. That pass naturally populates the page cache, so
the mapping starts **fully resident immediately after an update** and only cools under genuine later
memory pressure — which is exactly the condition under which eviction is the behavior you want.
This also settles an apparent contradiction: the "we never eagerly load it all" property applies to
**steady-state lookup access**, not to the one-time verification pass, which necessarily touches
every byte. Those are different claims and only the first one was ever true.

**A bounded latency budget on the hot path.** `net-shield`'s DNS path answers live queries, and a
stalled lookup delays packet delivery. The FST lookup therefore runs off the packet-handling thread
and the caller waits at most a small fixed budget (starting value: **2 ms**); on expiry it falls
back to the last-known decision for that domain from a small in-memory cache, or, absent one, to the
filter's existing default action — and counts the event. It never blocks the packet path waiting on
a page fault. This is the same shape as the mac-daemon's rule that a tick with work in flight
repeats the last verdict rather than stalling or defaulting to allow.

Mechanics:

- Hold the mapping behind a reference-counted handle (`Arc<Mmap>` via the `memmap2` crate). During
  a swap, the old and new mappings *can* briefly coexist — any lookup already holding a clone of the
  old `Arc` keeps it alive until it finishes — so this is not a claim that only one copy is ever
  resident. The property that holds is narrower and still the important one: neither mapping is ever
  force-retained beyond what's in use, both are file-backed and therefore reclaimable the moment
  nothing references them, and the swap itself needs no bulk copy — just publishing a new `Arc` and
  letting the old one's refcount drain to zero. The `Arc` adds a per-lookup atomic-refcount cost,
  not a memory cost.
- Updates are atomic at the file level: write the new signed file to a temp path and `rename()` over
  the old one (atomic on POSIX filesystems, including Android's) — never overwrite the mapped file
  in place.
- **The last-known-good fallback needs a durable second copy, not just an atomic `rename()`.** A
  single-slot layout (one `.fst` + one manifest, replaced in place) has nothing to fall back *to*
  once the new file has replaced the old one — "fails closed to the last-known-good mapping" is only
  true if a last-known-good file still exists on disk. The layout is therefore **two slots**,
  `current/` and `previous/`, each holding a `.fst` + manifest pair, plus a small separate
  high-water-mark record (the highest `version` ever verified, per the trust contract below):
  1. Write the new `.fst` + manifest into a fresh temp directory.
  2. `fsync` both files, then `fsync` the temp directory's own entry (the directory fsync is what
     makes the file's existence durable, not just its contents).
  3. Verify the new manifest's signature and `fst_digest` **from the temp location**, before it
     becomes anything's `current`.
  4. Move the *existing* `current/` to `previous/` (replacing whatever was there), then `rename()`
     the temp directory to `current/`, then `fsync` the parent directory to persist both renames.
  5. Only after step 4 completes does the loader update the persisted high-water mark, itself written
     via the same temp-write-`fsync`-`rename` pattern, and only that write is what makes the update
     "seen" on a future start — a crash between step 4 and the high-water-mark write is recovered on
     next start by re-deriving the mark from `current/`'s own manifest, since a version that reached
     `current/` intact is trusted at least that far.
  - **Recovery on start:** verify `current/`'s manifest and digest. If that fails (missing, corrupt,
    bad signature, or `fst_digest` mismatch — the file was truncated by a crash mid-write, for
    instance), fall back to verifying `previous/` the same way and load it if it passes, logging the
    fallback as a tamper/corruption signal. If **both** slots fail verification, the loader fails
    closed to **no mapping** rather than fabricating one — `net-shield`'s existing default action for
    an unloadable filter applies, the same as any other missing-artifact case, and this state is
    surfaced the same way a revoked permission is elsewhere in this project, not silently swallowed.
  - This sequence is why an interrupted update can never leave a client with an unverifiable *and*
    unrecoverable state: at every point up to step 4, `current/` is untouched and still the prior
    good version; from step 4 onward, `previous/` holds that same prior good version as a fallback.
    Test the crash points explicitly: a crash before step 4 (current unaffected), a crash during the
    `current`↔`previous` swap (recovery must find one consistent slot), and a crash after step 4 but
    before the high-water-mark write (recovery re-derives the mark from `current/`).

**The latency figures above are unmeasured.** The benchmarking plan that turns them into real
numbers — including how to force genuine kernel-level eviction rather than measuring a warm cache
and calling it a fault — is in [the plan](../components/domain-blocklist/plan.md).

### A fast-cadence overlay tier, so an update need not wait for the next bulk rebuild

Two costs are easy to conflate and need separating. **Rebuilding** the FST when the domain set
changes is cheap — it runs on the pipeline's own build machine, on the existing monthly cadence, with
no device-side resource pressure at all. **Distributing** an update is not free at any size: every
client update, no matter how small the actual change, still costs a full-artifact download and a
full sequential hash-verify (verification reads every byte, by design — see the mmap section above).
Making the bulk cadence tighter to propagate an urgent single-domain fix faster would mean paying
that full cost on every device far more often than the bulk gates (canary, shrinkage, growth,
false-positive) actually need to run, and it would not make the FST itself faster to update: `fst::
Map` has no incremental-edit API, and mutating a live mmap that lookups may be concurrently reading
is a correctness hazard this design already avoids elsewhere (see the mmap section's atomic
`rename()`-only update rule). Rebuilding from the complete sorted key set is the only way the format
supports, whether one domain changed or a million did.

The fix is not to make the bulk artifact patchable. It's a **second, much smaller artifact** — a
capped list of recent additions and removals, reusing the bulk manifest's exact trust contract
(monotonic `version`, signature list, digest-bound payload, current/previous atomic slot swap) at a
fraction of the size, and therefore cheap enough to re-fetch and re-verify in full on a much shorter
cadence. It is folded into the next bulk rebuild and drained, never a second permanent copy of the
list living outside the reviewed bulk pipeline — see [the plan](../components/domain-blocklist/plan.md)
module 8 for the entry shape, size caps, and folding mechanics.

**A removal is not just an additional entry; it changes query-time control flow.** The overlay is
consulted before the bulk FST specifically so it can represent state the bulk artifact doesn't have
yet, and a removal is exactly such a case: an entry the bulk FST still blocks that this artifact
exists to urgently unblock. A matching removal therefore must terminate the lookup at the overlay
tier rather than falling through to the bulk FST — a fall-through would silently re-apply the block
the removal exists to lift, indistinguishable from the removal never having published at all. This
holds regardless of the eventual lookup implementation (in-memory map, a second mmap, or an
overlay-first cache-through structure ahead of the bulk FST, none yet measured); it does not change
the precedence of device-local rules or explicit `net-shield` rules, which still outrank the overlay
the same way they outrank the bulk FST — see the plan's module 6 precedence table.

**This does not weaken the opt-in-only stance above, or reintroduce it by another name.** The overlay
is fetched on the exact same user-consented check as the bulk artifact; nothing here adds a
background poll or a push channel. What changes is what that check costs: today, a user who checks
between monthly bulk releases gets nothing for it — there is no smaller update to fetch. With the
overlay, the same check picks up a few-KB, sub-second update instead. "Faster" here means "cheap
enough that the user's own chosen cadence already delivers it," not a new covert channel — the
opt-in stance is unchanged, and remains a decision revisited only with a stated privacy tradeoff, not
one incidentally weakened by a distribution optimization.

### Distribution

The FST file *is* the signed, distributed artifact — no separate transform happens on the client.
The pipeline that does normalize → merge → gate → liveness-TTL-prune also emits the final `.fst`
file and a manifest, and publishes it (e.g. GitHub Releases — no login required, CDN-cached, cheap
conditional `ETag` GETs for clients that are already current). Client updates are opt-in, never
silent background polling, consistent with the project's local-first default — and see the note
under [Users](#users) about the fetch itself being an outbound signal.

**The trust contract, precisely:**

- The manifest carries a monotonically increasing `version`, one or more `{key_id, signature}` pairs
  (see key rotation below — this is a list, not a single field), the SHA-256 `fst_digest` of the
  `.fst` file, the entry count, the per-source snapshots (version, license, fetch time), the computed
  output license, and the provenance table. The exact struct is in
  [the plan](../components/domain-blocklist/plan.md) module 4 and must match this section field for
  field.
- The `fst_digest` field binds the manifest to the artifact — a client can't be handed a manifest
  that describes one artifact while a different one is loaded.
- **Each Ed25519 signature covers the manifest bytes with that entry's `key_id` excluded** (the
  `key_id`/`signature` list itself is appended after signing, not signed over — otherwise adding a
  second signature during rotation would invalidate the first). `fst_digest` is already a field
  *inside* the signed portion, so signing it already binds the artifact transitively; the earlier
  `fst_digest || manifest_bytes` construction concatenated the digest with bytes that already
  contained it, which added a way to get the framing wrong and no security property.
- A client verifies against **any one** `{key_id, signature}` entry whose `key_id` is in its own
  trusted set — it does not need every entry to verify, only one. This is what makes rotation work
  without needing every client to agree on which key is "current."
- A client rejects and falls back to the last-known-good bundle on: no entry verifying against any
  key in its trusted set, a manifest whose `fst_digest` doesn't match the `.fst` file actually
  present, a manifest that fails to parse, or a `version` that is **not strictly greater** than the
  client's recorded high-water mark. That last check is what stops a compromised distribution point
  from serving a validly signed but older, previously-revoked bundle back to a client (a rollback
  attack).
- **The high-water mark is the highest `version` the client has ever successfully verified**, stored
  alongside the last-known-good bundle — not merely the version currently loaded, so a client that
  has rolled back to a last-known-good bundle for an unrelated reason does not thereby re-accept an
  older one.
- **Accepted limitation:** a fresh install, or a device offline long enough to be reimaged, has no
  high-water mark and will accept any validly signed bundle, including an old one. Anchoring the
  floor in the shipped binary (a minimum acceptable `version` baked in at app build time) narrows
  this but does not close it, since an old app build has an old floor. This residual gap is
  accepted; closing it properly needs an online freshness check, which conflicts with the opt-in
  update stance.
- **Key rotation, both sides.** The binary ships a small list of trusted public keys, not a single
  hardcoded one, but a client's trusted set only helps if the pipeline actually produces something
  that set can verify — keeping an old key trusted in a *new* binary does nothing for a client still
  running the *old* binary, which only knows the old key. So the pipeline, not just the client, has
  an overlap obligation:
  1. Ship an app update that adds the new key to the trusted set. No client yet requires it.
  2. Once that update has had a full release cycle to reach clients, the pipeline switches to
     **dual-signing**: every build carries both an old-key and a new-key `{key_id, signature}` entry
     in the manifest. An unupdated client (old key only) still verifies via the old entry; an
     updated client verifies via either.
  3. Only after dual-signing has run for a full release cycle — long enough that a client which will
     ever update has had the chance to — does the pipeline drop the old signature and sign
     new-key-only. Retiring the old key from clients' trusted sets can follow in a later app update;
     it is not itself security-critical, since an unused trusted key that nothing signs with anymore
     is inert.
  A key is never removed from the trusted set in the same release that introduces its replacement,
  and the pipeline never stops producing a signature an already-shipped client can verify before that
  client has had a real chance to update. Test the dual-signature manifest against both an
  old-key-only and a both-keys-trusted client.

**Verification runs on every process start, not only on update.** The tradeoff is explicit: it costs
one sequential read and hash of the artifact per start (which, per the mmap section, is not wasted —
it warms the mapping), and it is the only thing that detects an on-disk tamper *between* updates.
Verifying only on update would make the check cheaper and leave a modified file undetected until the
next update, which on an opt-in update cadence could be indefinitely. A daemon starts rarely and
runs for a long time, so the cost lands in the right place. If a boot-time measurement ever shows it
matters, the answer is a faster hash or an OS-level integrity mechanism — not dropping the check.

#### Staleness must be visible

Opt-in updates mean a client can sit on a months-old list forever, silently, which is in tension
with the freshness the trust design assumes. The stance does not change — no unconsented background
network call — but the *silence* does: the client **surfaces a visible in-app indicator when the
loaded manifest's `build_time` is older than three cadence periods**, so a user who has ignored
updates knows the list is stale rather than assuming it is current. Staleness a user can see is a
choice; staleness they can't is a defect.

## Rejected alternatives

- **A continuously mutable/patchable on-device FST** — considered as an alternative to the overlay
  tier above, to avoid shipping a second artifact type. Rejected on two independent grounds, either
  one sufficient alone: the `fst` crate provides no incremental-edit API (the format's compression is
  a function of the whole sorted key set, not something a local patch can preserve), and mutating a
  file backing a live mmap that lookups may be concurrently reading is a correctness hazard this
  design otherwise avoids entirely via atomic `rename()`-only updates. A second, small, append-then-
  drain artifact reuses the existing trust contract instead of inventing an unsafe one.
- **A single source** — no single list has both good coverage and reliable pruning; the union with
  provenance is what makes a low-quality or disappearing source cheap to back out.
- **Auto-fetching each source's branch HEAD** — convenient, and it turns any upstream compromise or
  bad merge into a validly signed artifact within one cadence with no human in the loop. Pinned
  revisions plus a reviewed version bump cost one PR per update and are the only thing standing
  between an upstream mistake and a shipped one.
- **ICMP-based liveness checking** — unreliable signal against CDN-fronted origins, and an
  application-layer-adjacent probe is unnecessary risk against the CSAM boundary above; DNS-only is
  both cheaper to reason about and safer.
- **Using the CI host's system resolver** — the default is whatever the provider supplies, which is
  filtered often enough that this is not a hypothetical, and the failure is silent and total. An
  explicitly configured resolver plus a canary check is the only safe configuration.
- **Client-side liveness revalidation** — would charge the recheck cost against every installed
  device's egress and battery for a result that is identical across all of them; centralizing it
  once in the pipeline is strictly better.
- **Proceeding with a partial build when a source fetch fails** — produces a valid-looking, validly
  signed artifact with a slice of coverage silently missing. Failing loudly is the only option that
  cannot ship a lie.
- **Inferring apex coverage by stripping prefixes** — the shortest path from `www.foo.example.com`
  to blocking all of `example.com`, and from a hosting-provider hostname to blocking every tenant on
  it. Scope comes from what the source actually named, checked against the PSL, and nowhere else.
- **Manual review of the full list for personal-name domains** — not implementable at multi-million
  scale, so proposing it is the same as proposing nothing. An automated triage filter over a
  bounded review queue is a real mechanism.
- **Bloom filter as the primary on-device structure** — fails the non-lossy requirement.
- **`marisa-trie` instead of `fst`** — comparable compactness, but a native C++ dependency that
  would need cross-compiling across every Android ABI this project targets, repeating the class of
  pain already hit with `ort`/ONNX Runtime on Android (see the `image-sandbox` row in `AGENTS.md`).
- **Eager heap-resident load of the FST bytes at startup** — simpler, and it removes the major-fault
  tail entirely, which is a genuine advantage. It is rejected because it makes the bytes
  unreclaimable: under Android's OOM-killer behavior for backgrounded processes, that trades a rare
  slow lookup for an occasional total loss of filtering. It also forces a double-memory window
  during an update, where mmap needs no bulk copy.
- **Signing `fst_digest || manifest_bytes`** — the digest is already a manifest field, so the
  concatenation binds nothing the manifest signature doesn't already bind, while adding a framing
  detail two implementations could disagree about.
