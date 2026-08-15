//! Real, network-touching `SourceFetcher` implementations for `cli` (module 7).
//!
//! `sources::mod.rs` (module 1) defines the `SourceFetcher` trait and the pin/license validation
//! every fetch goes through (`fetch_source`); the parsers there are network-free by design and
//! tested against fixture bytes. This module supplies the two real transports module 1 always
//! deferred to `cli`: GitHub (StevenBlack, Hagezi — see [`github`]) and UT1's tarball-plus-HTML-
//! scrape origin (see [`ut1`]). Neither module changes `SourceConfig`/`FetchError`/`SourceFetcher`;
//! both are thin adapters that produce a [`crate::sources::FetchedSource`] for `fetch_source` to
//! validate, per each file's own module doc comment.
//!
//! See `docs/components/domain-blocklist/plan.md`'s `### 7. cli` assumption audit for the live
//! findings (Hagezi's real path, UT1's tar.gz transport shape, GitHub's license/revision API
//! shape) these fetchers implement against.

pub mod github;
pub mod ut1;
