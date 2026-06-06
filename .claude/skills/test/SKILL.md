---
name: test
description: >
  Write and run tests for any part of the Holy Blocker project. Use this skill whenever the user wants
  to write a new test, run existing tests, add test coverage, follow TDD for a new feature, check if
  tests pass, or debug a failing test. Triggers on phrases like "write a test for", "add tests", "run
  tests", "cargo test", "does this pass tests", "test this function", "TDD", "unit test", "integration
  test", or any request involving verifying code behavior through tests.
---

# Test Skill

This skill covers two things: **writing** good tests and **running** the right test command. Figure out which the user needs (or both) and act accordingly.

## Running Tests

Pick the command based on what the user is working on:

| Area | Command | Run from |
|---|---|---|
| Rust policy engine | `cargo test` | `packages/text-policy/` |
| Rust MITM proxy | `cargo test` | `packages/mitm-proxy/` |
| Desktop TypeScript | `pnpm --filter @holy-blocker/desktop typecheck` | project root |
| Desktop build check | `pnpm --filter @holy-blocker/desktop build` | project root |
| Python ML | `pytest` | `machine-learning/` |
| C++ daemon | CMake build + any added unit tests | `native-modules/win-daemon/` |

If it's not clear which area, ask. If the user says "run tests" while discussing a Rust file, run `cargo test` from the right package. Always show the output and flag failures clearly.

Useful `cargo test` flags:
- `cargo test <name>` — run only tests matching that substring
- `cargo test --lib` — library tests only
- `cargo test --test <file>` — one integration test file
- `cargo test -- --nocapture` — show `println!` output even on pass
- `cargo test -- --test-threads=1` — run serially (useful when tests share state)

## Writing Tests

### The core approach: TDD

When adding new logic, write the test first, then implement. This is the project rule. The cycle is:
1. Write a failing test that describes the behavior you want
2. Run it to confirm it fails (Red)
3. Implement the minimum code to make it pass (Green)
4. Clean up without breaking the test (Refactor)

### Structure every test with AAA

```
// Arrange — set up inputs and state
// Act — call the thing under test (one call)
// Assert — verify the outcome
```

Keep each test focused on one behavior. If a test needs a long arrange section, consider a helper function.

### Naming tests

Names should read as a sentence describing the scenario and expected result:

```rust
// Good
fn returns_err_when_url_scheme_is_missing()
fn scores_zero_for_empty_input()
fn blocks_known_adult_domain()

// Avoid
fn test_url()
fn it_works()
fn edge_case()
```

### Rust-specific patterns

**Unit tests** — put them in the same file as the code, inside a `#[cfg(test)]` module. This lets you test private functions directly:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_is_block_when_score_exceeds_threshold() {
        // Arrange
        let scorer = Scorer::new(threshold: 0.7);
        let text = "explicit content example";

        // Act
        let verdict = scorer.evaluate(text);

        // Assert
        assert_eq!(verdict, Verdict::Block);
    }
}
```

**Integration tests** — put them in `tests/` at the crate root. These can only call public API, which is good: they verify the contract the rest of the system depends on.

**Testing `Result` and `Option`:**

```rust
// For errors
assert!(result.is_err());
assert!(matches!(result, Err(PolicyError::InvalidInput(_))));

// Unwrap with a message so failures are readable
let val = result.expect("should parse valid input");
```

**Testing errors with `should_panic`:**

```rust
#[test]
#[should_panic(expected = "score out of range")]
fn panics_on_negative_score() {
    Scorer::new(-1.0);
}
```

**Doc tests** — examples in `///` comments are compiled and run as tests. Use them for simple, illustrative cases. Hide setup boilerplate with `#`:

```rust
/// Normalizes text before scoring.
///
/// ```
/// # use text_policy::normalize;
/// assert_eq!(normalize("Hello, World!"), "hello world");
/// ```
```

### What to test

Test the behaviors that matter for the project's safety guarantees:
- Classification thresholds and policy decisions
- Text normalization, scoring, allow/block outcomes
- Error propagation (does the right error surface?)
- Boundary conditions (empty input, max-length input, known edge cases)
- Public API contracts (what callers can rely on)

Don't test:
- Rust stdlib behavior
- Simple getters with no logic
- Implementation details that could change without breaking behavior

### Isolation

Tests should not share mutable state. If two tests write to the same file or global, they'll flake when run in parallel. Use:
- Local variables and structs created fresh in each test
- Temporary directories (`tempfile` crate) for file system tests
- Trait objects + `mockall` mocks when testing code that calls external services

### Property-based testing (when useful)

For functions with wide input ranges (normalizers, scorers, parsers), `proptest` finds edge cases you'd never think to write:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn normalize_never_panics(input in ".*") {
        let _ = normalize(&input); // just shouldn't panic
    }
}
```

Add `proptest` to `[dev-dependencies]` if it's not there yet.

## Checklist before finishing

- [ ] Tests are in the right place (unit in-file, integration in `tests/`)
- [ ] Each test has a descriptive name
- [ ] AAA structure is clear (even if not commented)
- [ ] No shared mutable state between tests
- [ ] Run the tests and confirm they pass
- [ ] For new logic: test was written before implementation
