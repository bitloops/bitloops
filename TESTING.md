# Testing Guide

Run commands from the repository root.

Install `cargo-nextest` before using the test lanes. On macOS, prefer:

```bash
brew install cargo-nextest
```

For other platforms, follow the official installation guide:
[https://nexte.st/docs/installation/](https://nexte.st/docs/installation/)

## Default commands (Cargo aliases)

| Goal                                             | Command                      |
| ------------------------------------------------ | ---------------------------- |
| Fast compile check                               | `cargo dev-check`            |
| Library tests only                               | `cargo dev-test-lib`         |
| Fast lane tests                                  | `cargo dev-test-fast`        |
| Merge smoke lane                                 | `cargo dev-test-merge`       |
| Slow lane tests                                  | `cargo dev-test-slow`        |
| Full lane tests                                  | `cargo dev-test-full`        |
| Coverage (LCOV)                                  | `cargo dev-coverage`         |
| Coverage (LCOV + HTML, one instrumented run)     | `cargo dev-coverage-all`     |
| Coverage metrics (lines/functions)               | `cargo dev-coverage-metrics` |
| Coverage compare (default 80/75 fallback policy) | `cargo dev-coverage-compare` |
| Coverage (HTML)                                  | `cargo dev-coverage-html`    |
| Install local CLI (signed on macOS)              | `cargo dev-install`          |
| File-size guardrail                              | `cargo dev-file-size`        |
| Format (write fixes)                             | `cargo dev-fmt`              |
| Format (check only)                              | `cargo dev-fmt-check`        |
| Clippy (warnings denied)                         | `cargo dev-clippy`           |
| One-command local gate                           | `cargo dev-loop`             |
| Quality Assurance Tests                          | `cargo qat`                  |
| DevQL capabilities suite                         | `cargo qat-devql-capabilities` |
| DevQL sync suite                                 | `cargo qat-devql-sync`       |
| DevQL ingest suite                               | `cargo qat-devql-ingest`     |
| Quickstart suite                                 | `cargo qat-quickstart`       |

`cargo dev-loop` runs: `fmt` (write fixes) -> `clippy` -> fast tests -> file-size check.
`cargo dev-test-fast` is the default local feedback loop.
`cargo-nextest` is the default runner behind `dev-test-*`, `test-*`, and `qat*`.
That default does not ban `cargo test`: use the checked-in aliases for the standard lanes, and use `cargo test` only where this guide explicitly calls for it or where `cargo-nextest` cannot cover the case.
The checked-in local `nextest` default is `8` test threads.
CI uses the `ci` `nextest` profile, pinned to `6` test threads.
The legacy `qat_acceptance` integration target has been split into `qat`, `qat_smoke`, `qat_devql_capabilities`, `qat_devql_ingest`, `qat_devql_sync`, `qat_onboarding`, and `qat_quickstart`.
`cargo dev-test-merge` runs the fast lane plus a curated set of slow smoke suites and is the blocking gate for pull requests into `develop`.
`cargo dev-test-slow` runs all slow targets only.
`cargo dev-test-full` runs fast + slow and is used for post-merge verification on `develop` and pull requests into `main`.
On macOS, `dev-test-*` and `dev-install` automatically sign produced binaries to reduce repeated policy validation overhead (`syspolicyd`).
`cargo qat` runs onboarding, smoke, DevQL sync, DevQL capabilities, and DevQL ingest in parallel.
`cargo qat` runs the bundled ignored QAT journey through `cargo-nextest`.
`cargo qat-devql-capabilities` is the focused DevQL capabilities alias.
`cargo qat-devql-ingest` is the focused DevQL ingest alias.
`cargo qat-devql-sync` is the focused DevQL sync alias.
`cargo qat-quickstart` is the focused quickstart alias.

### QAT scenario filtering

QAT suites support opt-in Cucumber tag filtering via `CUCUMBER_FILTER_TAGS`. If the variable is unset, the full suite runs as before.

- Run only tagged scenarios in the focused DevQL sync suite:

```bash
CUCUMBER_FILTER_TAGS='@test_harness_sync' cargo qat-devql-sync
```

- Use the direct `cargo test` form when you want the suite to stream step-by-step output:

```bash
CUCUMBER_FILTER_TAGS='@test_harness_sync' \
cargo test \
  --manifest-path bitloops/Cargo.toml \
  --features qat-tests \
  --test qat_devql_sync \
  qat_devql_sync \
  -- --ignored --nocapture
```

- Tag expressions use standard Cucumber syntax, for example:
  - `@test_harness_sync`
  - `@devql and @sync`
  - `@test_harness_sync and not @slow`

### Fast-lane thread tuning

- Override the local fast-lane default with `BITLOOPS_TEST_THREADS=<n> cargo dev-test-fast`.
- For a persistent per-machine override, export `BITLOOPS_TEST_THREADS` from your shell profile, for example `~/.zshrc`.
- Recommended starting points:
  - Apple Silicon laptops with more headroom: try `8` to `10`
  - Older or lower-core laptops: try `4` to `6`
  - CI stays pinned to `6` unless explicitly changed in workflow configuration

## macOS code-signing for local development

By default, local commands use ad-hoc signing (`-`) which requires no secrets and works for all contributors.

Optional team setup for a real keychain identity:

```bash
# list available code-signing identities
security find-identity -v -p codesigning

# pick one identity and export it for your shell profile
export BITLOOPS_CODESIGN_IDENTITY="Developer ID Application: <Name> (<TEAMID>)"
```

Environment toggles:

- `BITLOOPS_CODESIGN=0`: disable local signing (not recommended on affected macOS hosts).
- `BITLOOPS_CODESIGN_IDENTITY=<identity>`: use a keychain identity instead of ad-hoc signing.
- `BITLOOPS_CODESIGN_VERIFY=0`: skip post-sign verification if needed for speed.

Team baseline recommendation:

- No shared secrets file for local development.
- Keep identity material in macOS Keychain.
- Use per-user shell env (`~/.zshrc`) for `BITLOOPS_CODESIGN_IDENTITY` only if a real identity is needed.

## Fast/merge/slow/full lane policy

- Fast lane is the default loop and should stay cheap.
- Merge lane is the default pull-request gate for `develop`: fast coverage plus a small set of slow smoke suites.
- Slow lane is opt-in via `--features slow-tests` and runs all heavy targets only.
- Full lane runs fast + slow and is for post-merge verification on `develop`, pull requests into `main`, and explicit confidence runs.

### Put a new test in slow lane if it does any of the following

- Spawns `bitloops` or other subprocess-heavy end-to-end flows.
- Uses `git` command flows as part of the scenario.
- Starts daemon/server processes or binds local ports.
- Requires isolated `HOME`/`XDG_*` environment simulation.
- Simulates full agent lifecycle/hook workflows.

### Put a test in the merge smoke lane only when it is

- A true cross-surface proof that lower-cost tests cannot replace.
- Small enough to run on every pull request into `develop`.
- Representative of a broader slow suite, rather than a full matrix.

### Keep a test in fast lane when it is

- Pure unit/library logic.
- Small, deterministic integration coverage without daemon/process orchestration.
- Local fixture/temp-dir based and quick to execute.

## Rules for writing new tests

1. Keep tests deterministic.
2. Do not depend on external network or remote services.
3. Use temp directories and explicit test-local state, never shared machine state.
4. Avoid hidden ordering assumptions between tests.
5. Keep assertions behaviour-focused and failure messages explicit.
6. Gate heavy tests behind `slow-tests` in `bitloops/Cargo.toml` `[[test]]` entries.
7. When a slow suite catches a regression, add or move a lower-cost regression test into the closest stable seam.

## Opt-in Postgres test-harness tests (`postgres-tests`)

The `postgres-tests` Cargo feature is **not** enabled by `slow-tests`, `dev-test-slow`, or `dev-coverage`. It compiles and runs library tests that start a temporary Postgres cluster (`initdb` / `pg_ctl`); those stay opt-in so default and CI lanes stay lighter.

- **When to use:** Local verification of the Postgres-backed test harness after changing SQL or repository code in `bitloops/src/capability_packs/test_harness/storage/postgres/`.
- **Requirement:** Postgres client binaries on `PATH` (for example Homebrew `postgresql@16`). If they are missing, the tests skip and exit successfully.
- **Command (from repository root):**

```bash
cargo test -p bitloops --lib --no-default-features --features postgres-tests
```

If the repo gains doctests in future, keep running those through `cargo test --doc`; `cargo-nextest` does not support doctests. This is another explicit exception to the default `nextest`-backed lanes above.

## Checklist before opening a PR

```bash
cargo dev-check
cargo dev-fmt-check
cargo dev-clippy
cargo dev-test-fast
cargo dev-test-merge
cargo dev-file-size
```

If your change touches broad slow suites or post-merge flows, also run:

```bash
cargo dev-test-full
cargo qat
```

## Coverage

```bash
cargo dev-coverage
cargo dev-coverage-all
cargo dev-coverage-metrics
cargo dev-coverage-html
open bitloops/target/llvm-cov-html/html/index.html
```

Coverage baseline metadata is refreshed on pushes to `develop`.
Coverage stays separate from the blocking pull-request gates.

- CI compares coverage against GitHub repository metadata baselines.
- If metadata is missing, CI falls back to `80.00%` lines and `75.00%` functions.
- Tolerance is `0.5` percentage points in both baseline and fallback modes.

## Install local binary

```bash
cargo dev-install
```
