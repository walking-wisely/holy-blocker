# 2. `merge` — normalize, scope, union, provenance, category separation

Part of the [Domain Blocklist plan](../plan.md).

```text
src/merge.rs
```

Responsibilities:

- Applies `domain_normalize::normalize` (module 0) to every `RawEntry`. This crate does **not**
  define its own normalization.
- Resolves each entry's final `RuleScope` via `domain_normalize::classify_scope`, combined with the
  parser's `scope_hint`:
  - `scope_hint = Apex` (a wildcard) plus `classify_scope = Apex` → `Apex`.
  - `scope_hint = Apex` on something that is not eTLD+1 → the *base* is kept as `ExactHost`, and
    the widening is refused. A wildcard cannot conjure apex coverage the PSL says isn't the
    tenant's.
  - `classify_scope = None` (the entry is a public suffix or on the shared-hosting denylist) → the
    entry is **dropped**, counted, and named in the build report. This is the case that would
    otherwise black-hole an entire hosting provider.
- `MergedEntry { domain: String, scope: RuleScope, sources: Vec<SourceId>, categories: Vec<Category> }`
  — the union type. **`categories` is a set, not a single value**, because two sources can
  legitimately disagree about what kind of site a domain is (one flags it `adult`, another flags the
  same domain `gambling`) and both are true statements about it, not a conflict to resolve. Merging
  two `RawEntry`s for the same normalized domain unions their `sources` the same way, rather than
  picking one arbitrarily, so a later "why is this blocked" query can answer with every source that
  flagged it and every category it was flagged under. Merging two entries with different scopes
  takes the **wider** scope (`Apex` wins), since one source legitimately naming the apex is a
  stronger statement than another naming one host under it.
- Categories are never blended into a single value: a source's own category (`adult`, `gambling`,
  `dating`, ...) is added to the entry's `categories` set, never overwrites it, and an entry ships to
  module 3 if **any** of its categories is one this build is configured to ship (an `adult`-only
  build still includes a domain that is also flagged `gambling`, because the `adult` flag alone
  qualifies it). Test a domain classified as both `adult` and `gambling` by two different sources and
  assert both categories survive merge.
- **Personal-name triage** — `flag_personal_name(second_level_label: &str) -> bool`. Pure function:
  returns true when the second-level label decomposes into **2–4 alphabetic tokens** separated by
  `-`, `.`, or `_`, in any order. Optionally raises confidence against an embedded first-name /
  surname frequency list. Matches are written to a review-queue file; **no verdict changes**. This
  is a triage filter to bound the review workload demanded by the decision doc's GDPR section, not
  a completeness guarantee, and its tests should assert that framing (it will flag `red-panda.com`
  and miss `janedoe.com`, and both are acceptable).
