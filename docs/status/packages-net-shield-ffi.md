# `packages/net-shield-ffi` — status

Language: Rust

UniFFI wrapper over net-shield's DNS path — `DnsGuard` + `inspect`/`wrapResponse`, `DnsDecision` as an enum-with-fields so the Kotlin binding is a sealed class. Built by the same `apps/mobile/scripts/build-ffi.sh`, which now loops over both FFI crates. Ships a **placeholder** rule set (one RFC 2606 reserved name), exactly as text-policy-ffi ships a placeholder dictionary; real lists go through `with_blocked_domains` at runtime
