# Ruff Fixture Quickstart

This quickstart shows how to run TestLens against the real Ruff workspace fixture in this repository:

- Repo: `./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`
- Commit: `75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5`

It is the best local fixture for the agent-helpful stories we can already exercise now:

- pre-change safety assessment
- discovering which tests matter before a change
- spotting untested artefacts
- finding untested code paths via coverage
- distinguishing "no static link" from "actually unexercised"
- suppressing weak cross-cutting links by default

On March 16, 2026, a fresh local run against this fixture produced:

- production ingest: `files: 1468, artefacts: 15854`
- test ingest: `files: 929, suites: 873, scenarios: 4859, links: 64760`
- coverage ingest: `scope: workspace, hits: 71161` (ruff_linter crate via `cargo-llvm-cov`)
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
| `ingest-coverage` | Validated with workspace LCOV | `cargo llvm-cov --lcov` + `ingest-coverage --scope workspace`; 71k hits on `ruff_linter` |
| `ingest-coverage-batch` | Optional | Bulk per-test ingestion via JSON manifest |
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
testlens ingest-coverage-batch --help
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

# Generate and ingest workspace coverage (requires cargo-llvm-cov)
cd ./75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  && cargo llvm-cov --lcov --output-path ../target/ruff-fixture.lcov -p ruff_linter \
  && cd .. \
  && testlens ingest-coverage \
       --db ./target/ruff-real-project.db \
       --lcov ./target/ruff-fixture.lcov \
       --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
       --scope workspace \
       --tool cargo-llvm-cov
```

Expected shape from a validated local run:

- production ingest: `files: 1468, artefacts: 15854`
- test ingest: `files: 929, suites: 873, scenarios: 4859, links: 64760`
- coverage ingest: `scope: workspace, hits: 71161` (for `ruff_linter` crate)
- `ingest-tests` prints timeout notes when `cargo test -- --list` or `cargo test --doc -- --list` cannot finish in time on the full Ruff workspace; the run still succeeds in `source-only` mode

## 4) Interesting queries — what an agent learns before editing code

These queries use artefacts from two real Ruff tasks (F523 and ERA001) to show what TestLens tells an agent before it starts changing code.

### F523: pre-change safety assessment

An agent preparing the F523 fix (`"".format(x)` empty-format false positive) asks: is the main rule entry point already tested?

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_positional_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary
```

Returns `partially_tested` with `2` linked tests and `line_coverage_pct: 98.3` (after coverage ingest). The agent knows this is not a blind edit.

### F523: discover the concrete tests and local test style

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_positional_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests \
  --min-strength 0.0
```

Returns:
- `rules[StringDotFormatExtraPositionalArguments, F523.py]` — the Ruff rule harness
- `StringDotFormatExtraPositionalArguments[doctest:416]` — a nearby doctest

This points the agent at the exact fixture path (`F523.py`) and the naming convention to follow.

### F523: inspect the neighboring F522 rule for regression context

The F522/F523/F524/F525 rules share a dispatch path. Before changing shared `.format(...)` logic, the agent checks the sibling:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact string_dot_format_extra_named_arguments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests \
  --min-strength 0.0
```

Returns `rules[StringDotFormatExtraNamedArguments, F522.py]` — confirming the neighboring rule uses the same harness pattern and the agent should run both after the change.

### F523: helper-level gap detection — where static linkage stops

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact remove_unused_positional_arguments_from_format_call \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary
```

Returns `untested` with 0 linked tests — but `line_coverage_pct: 100.0` after coverage ingest. Static linkage says "untested"; coverage reveals the helper is fully exercised indirectly through the F523 harness. The agent knows this is not a blind spot.

Compare with a true blind spot:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact transform_expression \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view summary
```

Returns `untested` with 0 linked tests and no coverage (not in `ruff_linter` crate). This is genuinely unprotected.

### ERA001: rule-level harness discovery

An agent preparing the ERA001 fix (script-metadata false positive) asks the same question:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact commented_out_code \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests \
  --min-strength 0.0
```

Returns `rules[CommentedOutCode, ERA001.py]` — pointing at the core fixture to extend.

### ERA001: script-block boundary helper tests

The ERA001 task is about deterministic end-of-block handling. TestLens surfaces the exact relevant tests:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact skip_script_comments \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests \
  --min-strength 0.0
```

Returns `script_comment` and `script_comment_end_precedence` — a unit test named almost exactly after the edge case the task wants protected.

### ERA001: broad detection regression surface

Before tightening script-metadata exemptions, the agent needs the existing safety net:

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact comment_contains_code \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests
```

Returns 10 linked unit tests including `comment_contains_code_basic`, `comment_contains_code_with_multiline`, `comment_contains_code_with_default_allowlist`, etc. The agent knows what general ERA001 sensitivity it must preserve.

### Cross-cutting artefact — when TestLens suppresses noise by default

```bash
testlens query \
  --db ./target/ruff-real-project.db \
  --artefact LineColumn.default \
  --commit 75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5 \
  --view tests
```

This artefact is called from many tests. The default `min_strength` threshold filters out weak cross-cutting links, preventing the agent from being overwhelmed by hundreds of tangential tests.

### Summary table

| Artefact | Task | Linked tests | Line coverage | What an agent learns |
| --- | --- | --- | --- | --- |
| `string_dot_format_extra_positional_arguments` | F523 | 2 (harness + doctest) | 98.3% | Safe to change — strong coverage, known tests |
| `string_dot_format_extra_named_arguments` | F523 | 1 (F522 harness) | — | Sibling rule to check for regression |
| `remove_unused_positional_arguments_from_format_call` | F523 | 0 | 100% | No static link but fully exercised indirectly |
| `transform_expression` | F523 | 0 | (not in `ruff_linter`) | True blind spot — no tests and no coverage |
| `commented_out_code` | ERA001 | 1 (ERA001 harness) | — | Points at the exact fixture to extend |
| `skip_script_comments` | ERA001 | 2 (script_comment tests) | — | Direct unit tests for the boundary parser |
| `comment_contains_code` | ERA001 | 10 unit tests | 100% | Broad regression surface that must not break |

## 5) Coverage reference

### How coverage changes query output

After ingesting workspace LCOV, the summary view gains `line_coverage_pct`, `branch_coverage_pct`, and `coverage_mode`:

```json
{
  "summary": {
    "verification_level": "partially_tested",
    "total_covering_tests": 2,
    "line_coverage_pct": 98.3,
    "branch_coverage_pct": 0.0,
    "coverage_mode": "artefact_only"
  }
}
```

### `coverage_mode` and `evidence` fields

- `coverage_mode` on summary: `"none"` (no coverage ingested), `"artefact_only"` (workspace LCOV — line stats without test attribution), `"per_test_line"` or `"per_test_branch"` (isolated per-test captures)
- `evidence` on each covering test: `"static_only"` (linked by Tree-sitter parsing) or `"isolated_line_hit"` (confirmed by per-test coverage capture)

With workspace LCOV, all tests show `evidence: "static_only"` — this is honest. Workspace LCOV cannot prove which specific test exercised which line.

### Scope parameter

The `--scope` parameter controls coverage attribution:

| Scope | Format | What it produces |
| --- | --- | --- |
| `workspace` | LCOV | Artefact-level line/branch stats. No per-test attribution. |
| `package` | LCOV | Same as workspace, scoped to a single crate. |
| `test-scenario` | LLVM JSON only | Per-test attribution with `evidence: "isolated_line_hit"` and 0.95 confidence. Requires `--test-artefact-id`. |
| `doctest` | LLVM JSON only | Same as test-scenario, for doctests. |

LCOV is rejected for `test-scenario` scope because merged LCOV is too lossy for per-test attribution.

## 6) Validation checks

```bash
sqlite3 ./target/ruff-real-project.db "select count(*) from artefacts where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5' and canonical_kind='test_suite';"
sqlite3 ./target/ruff-real-project.db "select count(*) from artefacts where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5' and canonical_kind='test_scenario';"
sqlite3 ./target/ruff-real-project.db "select count(*) from test_links where commit_sha='75a24bbc67aa31b825b6326cfb6e6afdf3ca90d5';"
sqlite3 ./target/ruff-real-project.db "select count(*) from coverage_hits;"
```
