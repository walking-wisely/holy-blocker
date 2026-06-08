# Changelog

## Unreleased

### Added

- Public GitHub Actions CI workflow at `.github/workflows/ci.yml` for package-scoped checks.
- Manual `workflow_dispatch` support so CI can be run on demand even when path filters would skip jobs.

### Changed

- Tier 1 CI is now treated as complete for the current simple coverage: desktop typecheck/build plus `cargo test` for `text-policy`, `mitm-proxy`, and `net-shield`, each gated by path-based change detection on PRs and pushes to `main`.
