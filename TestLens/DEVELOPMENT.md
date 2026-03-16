# TestLens Development / QA Guide

This guide covers the updated prototype flow with three ingestion paths:

1. `ingest-tests` (Tree-sitter)
2. `ingest-coverage` (LCOV)
3. `ingest-results` (Jest JSON)

Production artefacts are seeded first; test artefacts/links/runs are discovered via ingestion (not prepopulated).

## 1) Prerequisites

- Rust toolchain (`cargo`)
- `cargo-llvm-cov` for the Rust coverage flow
- Node.js + npm
- `sqlite3`

## 2) Key Paths

- Repo root: current working copy
- DB (default): `./testlens.db`
- TypeScript fixture repo: `./testlens-fixture`
- Rust fixture repo: `./testlens-fixture-rust`
- Real Rust workspace fixture repo: `./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`
- Real Rust workspace fixture commit: `75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`
- LCOV output: `./testlens-fixture/coverage/lcov.info`
- Jest JSON output: `./test-results.json`

## 3) Quick Start

### 3.1 TypeScript/Jest Fixture

From repo root:

```bash
cargo install --path . --force

# 0) Reset DB (recommended for a clean run)
rm -f ./testlens.db

# 1) Initialize schema only
testlens init --db ./testlens.db

# 2) Ingest production artefacts from target repo for the commit
testlens ingest-production-artefacts --db ./testlens.db --repo-dir ./testlens-fixture --commit fixture-dev

# 3) Build test suite/scenario artefacts (Tree-sitter)
testlens ingest-tests --db ./testlens.db --repo-dir ./testlens-fixture --commit fixture-dev

# Optional: verify test artefacts were created
sqlite3 ./testlens.db "select count(*) from artefacts where commit_sha='fixture-dev' and canonical_kind='test_suite';"
sqlite3 ./testlens.db "select count(*) from artefacts where commit_sha='fixture-dev' and canonical_kind='test_scenario';"

# 4) Generate fixture outputs
cd testlens-fixture
npx jest --coverage --json --outputFile=../test-results.json --runInBand || true
cd ..

# 5) Ingest coverage + run results
testlens ingest-coverage --db ./testlens.db --lcov ./testlens-fixture/coverage/lcov.info --commit fixture-dev
testlens ingest-results --db ./testlens.db --jest-json ./test-results.json --commit fixture-dev

# 6) Query
testlens query --db ./testlens.db --artefact UserRepository.findById --commit fixture-dev
testlens query --db ./testlens.db --artefact UserRepository.findById --commit fixture-dev --view summary
testlens query --db ./testlens.db --artefact UserRepository.findByEmail --commit fixture-dev --view tests
testlens query --db ./testlens.db --artefact UserRepository.findByEmail --commit fixture-dev --view tests --min-strength 0.0
testlens query --db ./testlens.db --artefact UserRepository.findById --commit fixture-dev --view coverage
```

Notes:

- `testlens init` creates schema but does not clear existing data from prior runs.
- `rm -f ./testlens.db` gives a clean initial state.
- Helper scripts still exist for convenience:
  - `scripts/init-fixture-db.sh` = `init` + `ingest-production-artefacts`
  - `scripts/ingest-fixture-coverage.sh` = Jest run + `ingest-tests` + `ingest-coverage` + `ingest-results`

### 3.2 Rust Target Fixture (Compact Coverage Flow)

This flow exercises the Rust-first target fixture and validates production discovery, test discovery, static linkage, and query behavior.

From repo root:

```bash
cargo install --path . --force

# 0) Reset DB (recommended for a clean run)
rm -f ./testlens.db

# 1) Initialize schema only
testlens init --db ./testlens.db

# 2) Ingest Rust production artefacts from the target repo
testlens ingest-production-artefacts --db ./testlens.db --repo-dir ./testlens-fixture-rust --commit fixture-rust

# 3) Build Rust test suite/scenario artefacts and static links
testlens ingest-tests --db ./testlens.db --repo-dir ./testlens-fixture-rust --commit fixture-rust

# Optional: verify Rust test artefacts + links were created
sqlite3 ./testlens.db "select count(*) from artefacts where commit_sha='fixture-rust' and canonical_kind='test_suite';"
sqlite3 ./testlens.db "select count(*) from artefacts where commit_sha='fixture-rust' and canonical_kind='test_scenario';"
sqlite3 ./testlens.db "select count(*) from test_links where commit_sha='fixture-rust';"

# Optional: run the Rust fixture tests directly
cd testlens-fixture-rust
cargo test
cd ..

# 4) Query static linkage before coverage ingestion
testlens query --db ./testlens.db --artefact UserRepository.find_by_id --commit fixture-rust
testlens query --db ./testlens.db --artefact UserService.create_user --commit fixture-rust

# 5) Generate Rust LCOV coverage and ingest it
cd testlens-fixture-rust
cargo llvm-cov --lcov --output-path ../rust-fixture.lcov
cd ..
testlens ingest-coverage --db ./testlens.db --lcov ./rust-fixture.lcov --commit fixture-rust

# 6) Query coverage
testlens query --db ./testlens.db --artefact UserRepository.find_by_id --commit fixture-rust --view coverage
```

Rust fixture notes:

- This quickstart now validates `Production Artefact Discovered`, `Test Artefact Discovered`, `Static Test Link Established`, and Rust LCOV ingestion.
- `testlens query` already surfaces `covering_tests` from static links even before any coverage data is ingested.
- `cargo llvm-cov --lcov` is the documented Rust coverage-generation path for the fixture.
- `ingest-results` is currently Jest JSON based, so it is part of the TypeScript/Jest flow, not the Rust fixture flow.

### 3.3 Real Rust Workspace Fixture (Agent Story Quickstart)

This flow uses the real Ruff workspace fixture at commit `75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`.
It is the best local path for exercising the agent-helpful stories that are already ready now:

- Scenario 1 / 10: pre-change safety assessment and deciding whether to add tests first
- Scenario 2 / 8: understanding what tests exist and what local test style looks like
- Scenario 6: spotting effectively untested code
- Scenario 9: suppressing weaker cross-cutting links by default

On March 16, 2026, a fresh local run against this fixture produced:

- `873` test suites
- `4859` test scenarios
- `64760` links
- `enumeration: source-only` with explicit timeout notes from Cargo list commands

Reference:

- `docs/validation/agent_user_stories_ready_now.md`
- `docs/quickstart_ruff_fixture.md`

From repo root:

```bash
cargo install --path . --force

# 0) Reset DB (recommended for a clean run)
rm -f ./target/ruff-real-project.db

# 1) Initialize schema only
testlens init --db ./target/ruff-real-project.db

# 2) Ingest production artefacts from the real workspace
testlens ingest-production-artefacts --db ./target/ruff-real-project.db --repo-dir ./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5

# 3) Build Rust test suite/scenario artefacts and static links
testlens ingest-tests --db ./target/ruff-real-project.db --repo-dir ./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5

# Optional: verify that the workspace produced test links
sqlite3 ./target/ruff-real-project.db "select count(*) from artefacts where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5' and canonical_kind='test_suite';"
sqlite3 ./target/ruff-real-project.db "select count(*) from artefacts where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5' and canonical_kind='test_scenario';"
sqlite3 ./target/ruff-real-project.db "select count(*) from test_links where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5';"
```

Query the real workspace by user story:

```bash
# Scenario 1 / 10: Is this artefact safe to change before editing?
testlens query --db ./target/ruff-real-project.db --artefact RootDatabase.upcast --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --view summary
testlens query --db ./target/ruff-real-project.db --artefact string_dot_format_extra_positional_arguments --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --view summary

# Scenario 2 / 8: Which concrete tests cover this artefact, and what naming/style do they use?
testlens query --db ./target/ruff-real-project.db --artefact RootDatabase.upcast --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --view tests
testlens query --db ./target/ruff-real-project.db --artefact string_dot_format_extra_positional_arguments --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --view tests --min-strength 0.0

# Scenario 9: Show the weaker links that are hidden by the default min-strength filter
testlens query --db ./target/ruff-real-project.db --artefact RootDatabase.upcast --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --view tests --min-strength 0.0

# Cross-cutting example: inspect a noisy artefact with many linked tests
testlens query --db ./target/ruff-real-project.db --artefact LineColumn.default --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --view tests

# Benchmark residuals: helper-level artefacts that still have no direct linked tests
testlens query --db ./target/ruff-real-project.db --artefact remove_unused_positional_arguments_from_format_call --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --view summary
testlens query --db ./target/ruff-real-project.db --artefact transform_expression --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --view summary
```

Real workspace notes:

- This flow is verified for multi-crate Rust workspace discovery, source-discovered Rust test linkage, and static query stories.
- The current Ruff fixture run produces production artefacts, Rust test artefacts, and static links without assuming repo-root `src/`.
- Rust test ingestion now supports inline `src/**.rs` test modules and parameterized `#[test_case(...)]` scenarios on the synthetic Ruff-style acceptance fixture.
- `RootDatabase.upcast` currently returns `partially_tested` with `9` linked tests on the March 16, 2026 validation run.
- `string_dot_format_extra_positional_arguments` currently returns `partially_tested` with the expected F523 harness case plus a linked doctest.
- `RootDatabase.new` is no longer a good untested example on the current run; it now returns `partially_tested` with `4` linked tests.
- `LineColumn.default` is a good stress case for cross-cutting or noisy results because it currently links to many tests.
- Large workspaces like Ruff may fall back to `enumeration: source-only` when `cargo test -- --list` or `cargo test --doc -- --list` time out. That degraded mode is explicit in CLI output and does not block source-discovered ingestion.
- Residual benchmark gap: helper-level artefacts such as `remove_unused_positional_arguments_from_format_call` and `transform_expression` still do not link directly from the current static model.
- Keep the compact Rust fixture above for the documented LCOV flow until Rust coverage ingestion is validated on this larger workspace.

## 4) CLI Commands

- `testlens init`
  - `--db <path>` SQLite path (default `./testlens.db`)
  - `--seed` inserts production artefacts for fixture
  - `--commit <sha>` commit stamp

- `testlens ingest-tests`
  - `--repo-dir <path>` fixture repo path
  - `--commit <sha>` commit stamp
  - `--db <path>` SQLite path

- `testlens ingest-production-artefacts`
  - `--repo-dir <path>` repo to scan for production source files
  - `--commit <sha>` commit stamp
  - `--db <path>` SQLite path

- `testlens ingest-coverage`
  - `--lcov <path>` LCOV file path
  - `--commit <sha>` commit stamp
  - `--db <path>` SQLite path

- `testlens ingest-results`
  - `--jest-json <path>` Jest JSON output file
  - `--commit <sha>` commit stamp
  - `--db <path>` SQLite path

- `testlens query`
  - `--artefact <id|fqn|path>` artefact selector
  - `--commit <sha>` commit stamp
  - `--classification <unit|integration|e2e>` optional filter
  - `--view <full|summary|tests|coverage>` optional query level (default `full`)
  - `--min-strength <0.0-1.0>` optional override for `tests` / `full` views; default is `0.3`
  - `--db <path>` SQLite path

- `testlens list`
  - `--commit <sha>` commit stamp
  - `--kind <canonical_kind>` optional filter
  - `--db <path>` SQLite path

## 5) How `ingest-tests` Builds `test_suite` / `test_scenario` Artefacts

Precondition:

- Production artefacts must already exist in `artefacts` for the same `--commit`
  (typically via `ingest-production-artefacts`).
- `ingest-tests` resolves `repo_id` from previously ingested production rows for the same commit; if none exist, ingestion fails.

Discovery rules:

- Included:
  - `*.test.ts`
  - `*.spec.ts`
  - files under `__tests__/`
- Ignored:
  - `node_modules`
  - `coverage`
  - `dist`
  - `target`

Parsing + materialization flow:

1. Parse each discovered test file with Tree-sitter TypeScript grammar.
2. Create/Upsert a test file artefact in `artefacts` (`canonical_kind = file`).
3. For each `describe(...)` call, create a `test_suite` artefact with source span.
4. For each `it(...)` / `test(...)` inside a suite, create a `test_scenario` artefact with source span and suite parent.
5. Extract relative imports from the test file and normalize to repo-relative paths.
6. Extract call sites from scenario bodies (identifiers/member calls/new expressions).
7. Match called symbols against production artefacts loaded for the commit and create `test_links` rows (`link_source = static_analysis`).

Rerun semantics for the same commit:

- `ingest-tests` clears prior commit-scoped test discovery state before rebuilding:
  - `test_links`
  - `test_runs`
  - `test_coverage`
  - `test_classifications`
  - test artefacts in `artefacts` (`test_suite`, `test_scenario`, and test file rows)
- This keeps ingestion deterministic and prevents stale coverage/results after test discovery changes.

## 6) Install CLI Globally (Cargo)

```bash
# From the repo root
cargo install --path . --force
```

- Installs the `testlens` binary into Cargo's bin directory (typically `~/.cargo/bin`).
- Validate install:

```bash
testlens --help
```

## 7) Full QA Flow (Manual)

```bash
# 1) Build/check
cargo fmt --check
cargo check

# 2) Initialize DB with schema + production artefacts
./scripts/init-fixture-db.sh ./testlens.db qa1

# 3) Generate fixture outputs
cd testlens-fixture
npx jest --coverage --json --outputFile=../test-results.json --runInBand || true
cd ..

# 4) Ingest all three data streams
testlens ingest-tests --db ./testlens.db --repo-dir ./testlens-fixture --commit qa1
testlens ingest-coverage --db ./testlens.db --lcov ./testlens-fixture/coverage/lcov.info --commit qa1
testlens ingest-results --db ./testlens.db --jest-json ./test-results.json --commit qa1

# 5) Query
testlens query --db ./testlens.db --artefact UserRepository.findById --commit qa1
testlens query --db ./testlens.db --artefact UserService.createUser --commit qa1
testlens query --db ./testlens.db --artefact hashPassword --commit qa1
```

## 8) SQLite Validation Queries

```bash
sqlite3 ./testlens.db ".tables"

# Seeded/ingested production artefacts only
sqlite3 ./testlens.db "select count(*) from artefacts where commit_sha='qa1' and path like 'src/%';"
sqlite3 ./testlens.db "select count(*) from test_links where commit_sha='qa1';"
sqlite3 ./testlens.db "select count(*) from test_runs where commit_sha='qa1';"

# After ingest-tests
sqlite3 ./testlens.db "select count(*) from artefacts where commit_sha='qa1' and canonical_kind='test_suite';"
sqlite3 ./testlens.db "select count(*) from artefacts where commit_sha='qa1' and canonical_kind='test_scenario';"
sqlite3 ./testlens.db "select count(*) from test_links where commit_sha='qa1';"

# After ingest-coverage
sqlite3 ./testlens.db "select count(*) from test_coverage where commit_sha='qa1';"
sqlite3 ./testlens.db "select count(*) from test_classifications where commit_sha='qa1';"

# After ingest-results
sqlite3 ./testlens.db "select status, count(*) from test_runs where commit_sha='qa1' group by status order by status;"
```

## 9) Expected Behaviors

- `UserRepository.findById` returns covering tests and non-null coverage.
- `UserService.createUser` includes a failing test in `last_run.status`.
- `hashPassword` returns `covering_tests: []`, `coverage: null`, `verification_level: untested`.
- `query --view summary` returns only `artefact` + `summary`.
- `query --view tests` applies the default `min_strength` filter; `--min-strength 0.0` returns the full unfiltered match set.
- `query --view coverage` returns only `artefact` + `coverage`.
- If Jest JSON includes unknown tests, `ingest-results` logs warnings and continues.

## 10) Troubleshooting

- `Database not found. Run init-fixture-db.sh first.`
  - Run `./scripts/init-fixture-db.sh ./testlens.db <commit>`

- `Artefact not found`
  - Verify the artefact exists in `artefacts` for that commit.

- No coverage rows after `ingest-coverage`
  - Ensure `ingest-tests` ran first (coverage joins through discovered `test_links`).

- Unmatched Jest results
  - Ensure `ingest-tests` ran for the same commit and paths are from the same repo checkout.

## 11) BDD (Gherkin) for CLI-1345

Executable Gherkin coverage for ticket `CLI-1345` lives in:

- `features/cli_1345.feature`
- `tests/e2e.rs` (integration test harness)
- `tests/e2e/cli_1345_gherkin.rs`
- shared helpers:
  - `tests/e2e/support/fixture.rs` (temp workspace + fixture file generation)
  - `tests/e2e/support/sqlite.rs` (schema init + production artefact seeding helpers)
  - `tests/e2e/support/cli.rs` (CLI command runner + list helpers)
  - `tests/e2e/support/types.rs` (shared JSON-deserialized list models)

Acceptance tests now live under `tests/e2e/`. Unit tests remain co-located inline inside the implementation module files under `src/*.rs`.

For initial state in BDD tests, use:

- `initialize_schema(db_path)` to create the TestLens SQLite schema
- `seed_source_file_for_commits(...)` or `seed_production_artefacts(...)` to insert
  commit-addressed production artefacts before running `ingest-tests`

Run only the BDD suite:

```bash
cargo test --test e2e
```

Run a single acceptance flow by test name:

```bash
cargo test --test e2e rust_quickstart_e2e_gherkin
```

Run all tests (unit + BDD):

```bash
cargo test
```

Each test run also writes the current validation snapshot to `target/validation/current_status.md`.
