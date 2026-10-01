# AGENTS.md

Holy Blocker is an on-device content blocking project: local-first, no cloud calls, no telemetry.

**Read [`CLAUDE.md`](CLAUDE.md) first.** It is the canonical guidance file and holds everything
this one used to duplicate: the router that decides which loop a request enters, package status,
conventions, verification checks, and the worktree and PR rhythm. This file carries no project
state of its own, so it cannot go stale.

## Which loop

- New capability or no plan yet → the `plan-inception` skill.
- An existing plan step → the `step-loop` skill.
- Anything else → classify it with the tier table in `CLAUDE.md`, stricter tier on a tie.

## Commands

- JavaScript workspace: `pnpm install`, `pnpm build`, `pnpm typecheck`, `pnpm dev:desktop`
- Rust: `cargo test` from the crate directory
- macOS daemon: `./scripts/test.sh` from `native-modules/mac-daemon`, never bare `swift test`
- Plan tooling: `python -m tools.plan.ledger next docs/components/<component>`
- Docs: `python -m tools.plan.doclint`
- Tooling tests: `python -m unittest discover -s tools/plan/tests -t .`

Anything not listed here is in `CLAUDE.md`.
