# Ruff Fixture Quickstart

This quickstart shows how to run TestLens against the real Ruff workspace fixture in this repository:

- Repo: `./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`
- Commit: `75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`

It is the best local fixture for the agent-helpful stories we can already exercise now:

- pre-change safety assessment
- discovering which tests matter before a change
- spotting untested artefacts
- suppressing weak cross-cutting links by default

On March 16, 2026, a fresh local run against this fixture produced:

- production ingest: `files: 1468, artefacts: 15854`
- test ingest: `files: 929, suites: 873, scenarios: 4859, links: 64760`
- enumeration mode: `source-only`

## Scope

Current CLI coverage against the Ruff fixture:

| Command | Status on Ruff fixture | Notes |
| --- | --- | --- |
| `init` | Works now | Initializes the SQLite schema for the Ruff run |
| `ingest-production-artefacts` | Works now | Parses the multi-crate Rust workspace correctly |
| `ingest-tests` | Works now | Discovers Rust tests and static links across workspace crates |
| `list` | Works now | Lists production and test artefacts for the Ruff commit |
| `query` | Works now | Summary and test views are validated on Ruff |
| `ingest-coverage` | Optional / not part of the validated quickstart yet | Requires generating LCOV from the Ruff workspace first |
| `ingest-results` | Not applicable to Ruff | Expects Jest JSON; use the TypeScript fixture for this command |
| `help` | Works now | Use `--help` on any command |

## Prerequisites

- Rust toolchain with `cargo`
- `sqlite3`

Optional for the coverage section:

- `cargo-llvm-cov`

## 1) Install the CLI

Run these from `/Users/markos/code/bitloops/bitloops/TestLens`:

```bash
cargo install --path . --force
```

## 2) Inspect the CLI surface

```bash
testlens --help
testlens init --help
testlens ingest-production-artefacts --help
testlens ingest-tests --help
testlens ingest-coverage --help
testlens ingest-results --help
testlens list --help
testlens query --help
```

## 3) Minimal working Ruff flow

This is the validated path for the Ruff fixture.

```bash
rm -f ./target/ruff-real-project.db

testlens init --db ./target/ruff-real-project.db

testlens ingest-production-artefacts \
  --db ./target/ruff-real-project.db \
  --repo-dir ./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5

testlens ingest-tests \
  --db ./target/ruff-real-project.db \
  --repo-dir ./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5
```

Expected shape from a validated local run:

- production ingest: `files: 1468, artefacts: 15854`
- test ingest: `files: 929, suites: 873, scenarios: 4859, links: 64760`
- `ingest-tests` prints timeout notes when `cargo test -- --list` or `cargo test --doc -- --list` cannot finish in time on the full Ruff workspace; the run still succeeds in `source-only` mode

## 4) Run every applicable CLI command on Ruff

### `init`

```bash
rm -f ./target/ruff-real-project.db
testlens init --db ./target/ruff-real-project.db
```

Notes:

- `init --seed` is not the Ruff flow. It seeds the prototype fixture data, not the real Ruff workspace.

### `ingest-production-artefacts`

```bash
testlens ingest-production-artefacts \
  --db ./target/ruff-real-project.db \
  --repo-dir ./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5
```

### `ingest-tests`

```bash
testlens ingest-tests \
  --db ./target/ruff-real-project.db \
  --repo-dir ./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5
```

### `list`

List production functions:

```bash
testlens list --db ./target/ruff-real-project.db --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --kind function
```

List discovered Rust test scenarios:

```bash
testlens list --db ./target/ruff-real-project.db --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 --kind test_scenario
```

### `query`

Pre-change safety summary for a lightly linked artefact:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact RootDatabase.upcast \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary
```

This currently returns `partially_tested` with `4` linked tests.

Check the benchmark-relevant F523 rule artefact:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_positional_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary
```

This currently returns `partially_tested` with `2` linked tests.

See the concrete covering tests and their naming/style:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_positional_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests \
  --min-strength 0.0
```

Reveal weaker links hidden by the default `min_strength` threshold:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact RootDatabase.upcast \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests \
  --min-strength 0.0
```

Inspect a noisy cross-cutting artefact:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact LineColumn.default \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests
```

Inspect the current residual helper-attribution gap:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact remove_unused_positional_arguments_from_format_call \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary

testlens query \
  --db ./target/ruff-real-project.db \
  --artefact transform_expression \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary
```

These two helper-level artefacts still return `untested` on the March 16, 2026 run.

### `ingest-coverage`

This command exists, but it is not part of the validated Ruff quickstart yet.

Use it only after you generate an LCOV file for the Ruff workspace. The command shape is:

```bash
testlens ingest-coverage \
  --db ./target/ruff-real-project.db \
  --lcov ./target/ruff-real-project.lcov \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5
```

After coverage ingestion, the matching query shape is:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact RootDatabase.upcast \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view coverage
```

### `ingest-results`

This command is not part of the Ruff flow.

It currently expects Jest JSON:

```bash
testlens ingest-results \
  --db ./target/ruff-real-project.db \
  --jest-json ./test-results.json \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5
```

That makes it applicable to the TypeScript/Jest fixture, not the Rust Ruff workspace.

## 5) Useful validation checks

```bash
sqlite3 ./target/ruff-real-project.db "select count(*) from artefacts where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5' and canonical_kind='test_suite';"
sqlite3 ./target/ruff-real-project.db "select count(*) from artefacts where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5' and canonical_kind='test_scenario';"
sqlite3 ./target/ruff-real-project.db "select count(*) from test_links where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5';"
```

## 6) What this Ruff quickstart is good for

Use this fixture when you want to validate the user stories that help an agent decide what to do before editing code:

- Is this artefact effectively untested?
- Which existing tests appear relevant?
- Are there weak links I should ignore by default?
- Is this artefact noisy and cross-cutting?

Current concrete examples from the validated run:

- `RootDatabase.upcast`: `partially_tested` with `9` linked tests
- `string_dot_format_extra_positional_arguments`: `partially_tested` with the F523 harness case plus a doctest
- `RootDatabase.new`: `partially_tested` with `4` linked tests
- `remove_unused_positional_arguments_from_format_call`: `untested`
- `transform_expression`: `untested`

For the current ready-now mapping, see `docs/validation/agent_user_stories_ready_now.md`.
