# 4. `fst_build` — the on-device artifact

Part of the [Domain Blocklist plan](../plan.md).

```text
src/fst_build.rs
```

Responsibilities:

- Reverses each surviving domain's labels (`example.com` → `com.example`) so subdomain matching
  becomes prefix/exact-match at label boundaries on the consumer side. The key is the *normalized*
  form with no host-identity stripping (module 0): a source entry of `www.example.com` produces the
  key `com.example.www`, distinct from the apex key `com.example` — an `ExactHost` rule stored under
  the `www` key matches only a query for `www.example.com`, never the bare domain or any other
  subdomain. A separate `Apex` entry for `example.com`, if one exists, already covers `www` (and
  every other subdomain) through the label-boundary lookup below, without needing the two keys to
  collide.
- Sorts the reversed, deduplicated key set (an `fst::MapBuilder` requirement — the merge/dedupe step
  upstream already produces this property, so this is not new work, just an ordering constraint on
  output).
- Builds the FST, mapping each key to a small **provenance ID** (`u32`). The ID encodes the
  canonicalized combination of `{sources, categories, scope}` — **an enumerated set, not a
  per-domain value.** `categories` is the same set `MergedEntry` carries (module 2): a domain
  flagged under two categories keeps both in the combination it's assigned. Most domains share one
  of a few dozen combinations, and keeping the value space small is
  what limits the damage a `Map`'s per-key value does to suffix-sharing (states with differing
  outputs cannot be merged). The ID is only meaningful alongside the manifest entry that decodes it,
  so a future "why is this blocked" surface needs no second on-device lookup structure.
- **Report the achieved bytes-per-entry in the build metrics.** The commonly quoted 2–4 bytes/entry
  figure describes a bare set; this builds a `Map`. The number must be measured, and the size budget
  in module 5 — not the estimate — is the gate.
- Writes the `.fst` file plus a manifest:

  ```rust
  struct Manifest {
      version: u64,                          // monotonic; the client's rollback high-water mark
      build_time: Timestamp,
      entry_count: u64,
      output_license: LicenseId,             // least-permissive of the input licenses
      sources: Vec<SourceSnapshot>,          // one per constituent source, from module 1
      provenance_table: Vec<ProvenanceEntry>, // provenance_id -> { sources, categories, scope }
      fst_digest: [u8; 32],                  // SHA-256 of the .fst file, binding manifest to artifact
      signatures: Vec<Signature>,            // { key_id: KeyId, bytes: [u8; 64] } — see below
  }
  ```

  `version` and `signatures` are load-bearing, not decoration: the decision doc's rollback defense
  rejects a bundle whose `version` is not strictly greater than the client's high-water mark, and
  each `signatures` entry's `key_id` is how a client picks which of its trusted keys to try instead
  of guessing. An earlier draft of this struct had a single `key_id`/signature pair, which is exactly
  what key rotation (decision doc, [Distribution](../../../decisions/domain-blocklist-sourcing.md#distribution))
  cannot be built on — a single signer means the pipeline can only ever satisfy one generation of
  client trust set at a time. `signatures` is a list so a build can carry both an old-key and a
  new-key signature during a rotation's overlap window.

- **Signs `manifest_bytes` with every key currently active for signing** — one entry in `signatures`
  per key, each an Ed25519 signature over the manifest bytes with the `signatures` field itself
  excluded from what's signed (it's appended after signing each entry, not signed over — otherwise
  adding the second signature during rotation would invalidate the first). Outside a rotation window
  there is exactly one entry. Not `fst_digest || manifest_bytes` — `fst_digest` is already a field
  inside the manifest, so signing the manifest already binds the artifact transitively. The
  concatenation added a framing detail two implementations could disagree about and no security
  property.
- Round-trip tests cover building a small provenance table, encoding it into the manifest, decoding
  it back to the same `{sources, categories, scope}` per ID — including a domain assigned two
  categories — verifying that a manifest with two signature entries (simulating a rotation) validates
  against a client trusting only the old key, only the new key, or both, and that altering
  `fst_digest` after signing invalidates every signature entry.

(See [Reference documents](../plan.md#reference-documents) below for the FST/mmap references this module builds on.)
