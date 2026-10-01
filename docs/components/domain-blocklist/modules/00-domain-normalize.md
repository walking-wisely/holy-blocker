# Module 0 — `packages/domain-normalize`, a shared crate, built first

Part of the [Domain Blocklist plan](../plan.md).

**`normalize()` cannot live only in this crate.** The pipeline normalizes at build time and
`net-shield` must normalize identically at query time; any divergence between the two silently
breaks matching in a way no test on either side catches, because each side is internally consistent.
This is the same failure the mac-daemon avoided by consuming `packages/text-policy-ffi` instead of
writing a third implementation of the mode→action mapping — **one implementation, no drift**. A
second copy of `normalize()` is the wrong call for exactly that reason.

So the first thing built is a tiny, pure, dependency-light crate:

```text
packages/domain-normalize/src/lib.rs
```

- `normalize(domain: &str) -> Result<String, NormalizeError>` — the comparison key.
- `RuleScope { Apex, ExactHost }` and `classify_scope(normalized: &str, psl: &PublicSuffixList,
  shared_hosting_denylist: &[&str]) -> Option<RuleScope>` — the apex-eligibility decision.
- Nothing else. No I/O, no HTTP, no FST, no DNS.

Consumed by `domain-blocklist` at build time and by `net-shield` at query time. `net-shield` picks
up one small pure dependency, which is a far cheaper cost than a normalization mismatch.

The normalization contract, in order (the ordering is not optional — mapping and case-folding must
happen **before** punycode encoding, or two spellings of the same name encode differently):

1. Strip a trailing dot.
2. Apply **UTS #46** mapping and case folding (this subsumes plain ASCII lowercasing).
3. Convert U-labels to A-labels — punycode encoding per **RFC 3492**, with code-point eligibility
   per **RFC 5892**. Storage and comparison are always in A-label form.
4. **Validate lengths after conversion** — a label that exceeds 63 bytes or a full name that
   exceeds 253 bytes *after* A-label expansion is rejected as malformed. Pre-conversion length is
   not a valid check; punycode expands.

**There is no `www.`-stripping step, deliberately.** An earlier draft of this design stripped a
leading `www.` label before comparison, on the theory that it was "just a comparison convenience."
It wasn't: `classify_scope` runs on the *same* string `normalize()` produces, so a stripped
`www.example.com` and a bare `example.com` collapsed onto the identical comparison key and could
not be told apart downstream — a source entry meant to name only the `www` host silently became
indistinguishable from one naming the apex, and a query for the bare domain incorrectly matched a
rule that was only ever supposed to cover its `www` subdomain. `www.example.com` is therefore
normalized, scoped, and stored exactly like any other three-label hostname — it produces its own
`ExactHost` entry that matches `www.example.com` and nothing else. If a source separately lists the
bare `example.com` as `Apex`, that Apex entry already covers `www.example.com` (and every other
subdomain) at lookup time — see [module 4](04-fst-build.md) — so no aliasing
step is needed to get the common case right, and the uncommon case (only the `www` host is listed)
no longer silently over-blocks.

`classify_scope` returns `Apex` only when the normalized entry is exactly the registrable domain
(eTLD+1) per the Public Suffix List; `ExactHost` otherwise; and `None` (refuse the entry as an apex
rule) when the entry *is* a public suffix or appears on the shared-hosting denylist. Test this
first and hard: `com`, `co.uk`, `blogspot.com`, `s3.amazonaws.com` must never produce `Apex`, while
`someone.blogspot.com` must. Also test the `www.` case directly: an `ExactHost` entry for
`www.example.com` must match a query for `www.example.com`, and must **not** match `example.com` or
`cdn.example.com`.
