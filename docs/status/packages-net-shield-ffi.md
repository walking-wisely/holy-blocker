# `packages/net-shield-ffi` — status

Language: Rust

UniFFI wrapper over net-shield's DNS path — `DnsGuard` + `inspect`/`wrapResponse`, `DnsDecision` as an enum-with-fields so the Kotlin binding is a sealed class. Built by the same `apps/mobile/scripts/build-ffi.sh`, which now loops over both FFI crates. Ships a **placeholder** rule set (one RFC 2606 reserved name), exactly as text-policy-ffi ships a placeholder dictionary; real lists go through `with_blocked_domains` at runtime

`DnsGuard.withArtifact(artifactDir, trustedKeys)` loads the signed two-slot artifact (`current/` then `previous/`) through `BlocklistArtifact::load` behind the bounded-latency worker (2 ms budget, 4096-entry cache). It returns `ArtifactLoadError` (`Missing`, `BadTrustedKey`, `Rejected`) instead of a guard over an empty or untrusted list; the caller records the failure. Tests build artifacts with the `domain-blocklist` producer's `build` and load them through the FFI constructor. `apps/mobile` calls it from `BlocklistStore`.
