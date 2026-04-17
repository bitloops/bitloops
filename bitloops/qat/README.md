# QAT Test Guide

QAT runs as dedicated Rust integration test targets, executed via `cargo-nextest`. The legacy `qat_acceptance` target has been
split into `qat`, `qat_smoke`, `qat_devql_capabilities`, `qat_devql_ingest`, `qat_devql_sync`, `qat_onboarding`, and
`qat_agents_checkpoints`. The focused lane is `cargo qat-agents-checkpoints`. The production binary no longer exposes a `qat` subcommand, and the repo aliases enable the dedicated
`qat-tests` feature automatically.

## Where to run from

All commands below assume you are at the repository root.
The explicit `cargo nextest run` forms also work from the `bitloops/` crate directory when you keep the `--manifest-path bitloops/Cargo.toml` flag.

## Implemented test suites

### Active suite overview

Active QAT currently defines 92 scenarios total:

- 87 scenarios in the bundled `cargo qat` lane
- 5 scenarios in the focused `cargo qat-agents-checkpoints` lane

| Suite              | Command                        | In `cargo qat` | Active scenarios | Primary purpose                                                                   |
| ------------------ | ------------------------------ | -------------- | ---------------: | --------------------------------------------------------------------------------- |
| Onboarding         | `cargo qat-onboarding`         | Yes            |               13 | First-run install, init, enable, disable, and uninstall flows                     |
| Smoke              | `cargo qat-smoke`              | Yes            |               13 | Deterministic agent/checkpoint golden path across supported agents                |
| DevQL Sync         | `cargo qat-devql-sync`         | Yes            |               23 | Current-workspace reconciliation, validation/repair, path/full modes, queue control |
| DevQL Capabilities | `cargo qat-devql-capabilities` | Yes            |               24 | Query surfaces over checkpoints, DevQL state, TestHarness, semantic clones, and deterministic Confluence knowledge |
| DevQL Ingest       | `cargo qat-devql-ingest`       | Yes            |               14 | Commit-history replay, rewrite handling, and bounded backfill catch-up            |
| Agents Checkpoints | `cargo qat-agents-checkpoints` | No             |                5 | Focused checkpoint capture, pre-commit interaction, progression, and ordering     |

### Bundled QAT suites (CI bundle)

Runs onboarding and smoke in parallel, then runs DevQL sync, DevQL capabilities, and DevQL ingest serially.
The DevQL-heavy suites are intentionally serialized to avoid contention on SQLite-backed materialization paths.

- PRs targeting `main`: runs automatically in `.github/workflows/ci.yml`
- For develop-target work: run `.github/workflows/develop-qat.yml` manually from the GitHub Actions UI and select the branch you want to test

```bash
cargo qat
```

Works from both the `bitloops/` crate directory and the repository root.
The checked-in alias uses `cargo-nextest` as the runner.

Equivalent explicit form:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml --features qat-tests --test qat --run-ignored only -- qat --exact
```

The bundled suites are all part of `cargo qat`, and the focused aliases remain available for targeted runs.

### 1. Smoke (13 scenarios across 2 features)

Exercises the deterministic hook-driven agent lifecycle for `claude-code`, `cursor`,
`gemini`, `copilot`, `codex`, and `opencode` with `open-code` accepted as an alias.
This suite validates the Bitloops golden path without requiring any external agent CLIs
to be installed on the machine or CI runner.

```bash
cargo qat-smoke
```

Or equivalently:

```bash
cargo nextest run --features qat-tests --test qat_smoke --run-ignored only -- qat_smoke --exact
```

**Active coverage:**

- First agent-driven Bitloops session is captured for each supported agent.
- Follow-up agent edits create progression for each supported agent.
- Preserve relative-day commit timeline in a standalone timeline smoke feature.

The agent outlines cover the same setup for each supported agent: clean start, daemon
start, Vite scaffold, init commit, `bitloops init --agent <agent> --sync=false`, enable,
agent change, commit, session assertion, and checkpoint mapping assertion.

### 2. Onboarding (13 scenarios)

Covers the full first-time developer experience: install verification, daemon config,
repository enablement, agent hook installation for every supported agent, disable, and uninstall.

```bash
cargo qat-onboarding
```

Or equivalently:

```bash
cargo nextest run --features qat-tests --test qat_onboarding --run-ignored only -- qat_onboarding --exact
```

**Active scenarios:**

| #   | Scenario                                           | Flow                           |
| --- | -------------------------------------------------- | ------------------------------ |
| 1   | Binary is callable and reports a version           | `install-verify`               |
| 2   | Initialize daemon config from scratch              | `daemon-config-init`           |
| 3   | Enable Bitloops in a fresh git repository          | `enable-repo`                  |
| 4   | Agent hooks — claude-code                          | `agent-hooks-claude`           |
| 5   | Agent hooks — claude-code with sync=true           | `agent-hooks-claude-sync-true` |
| 6   | Agent hooks — codex                                | `agent-hooks-codex`            |
| 7   | Agent hooks — cursor                               | `agent-hooks-cursor`           |
| 8   | Agent hooks — gemini                               | `agent-hooks-gemini`           |
| 9   | Agent hooks — copilot                              | `agent-hooks-copilot`          |
| 10  | Agent hooks — open-code                            | `agent-hooks-open-code`        |
| 11  | Disable shows the repository as disabled, removes agent surfaces, and leaves git hooks intact | `disable-repo`                 |
| 12  | Uninstall removes agent and git hooks              | `uninstall-repo`               |
| 13  | Full uninstall removes managed hooks from the current repository | `uninstall-full`               |

### 3. DevQL Sync (`cargo qat-devql-sync`, 23 scenarios)

Exercises the queue-backed `bitloops devql tasks enqueue --kind sync` workspace
reconciliation flow: full indexing, TestHarness current-state sync, incremental
add/modify/delete detection, branch checkout, daemon downtime catch-up, git pull,
validation and repair, explicit full and path-scoped modes, task queue control,
`--require-daemon` failure handling, and `init --sync=true` integration.

```bash
cargo qat-devql-sync
```

Or equivalently:

```bash
cargo nextest run --features qat-tests --test qat_devql_sync --run-ignored only -- qat_devql_sync --exact
```

**Active coverage:**

- Core reconciliation (7 scenarios)
  - full source indexing
  - TestHarness coverage populate and delete
  - add / modify / delete detection
  - no-op sync
- Git/workspace transitions (3 scenarios)
  - branch checkout
  - daemon downtime catch-up
  - git pull catch-up
- Validation and repair (4 scenarios)
  - validate clean
  - repair after drift
  - drift detection after unreconciled changes
  - accumulated drift expected-count reporting
- Execution modes and queue control (5 scenarios)
  - path-scoped sync
  - explicit full sync
  - `--require-daemon` failure handling
  - task queue observability
  - task queue pause / resume / cancel behavior
- `init --sync=true` integration (3 scenarios)
  - immediate follow-up no-op
  - incremental sync after new files
  - clean validation without further workspace changes

### 4. DevQL Capabilities (`cargo qat-devql-capabilities`, 24 scenarios)

Exercises the strict offline DevQL capability surface: agent queryability,
checkpoint and chat-history retrieval, artefact-scoped dependency blast radius,
artefact-scoped TestHarness proof-map queries, guide-aligned deterministic
semantic clones, deterministic Confluence knowledge add/query/associate/refresh,
knowledge rejection handling, and one final integrated cross-capability smoke.
The semantic-clones lane validates the offline fake-runtime path rather than real
local-model warm-cache behavior.

```bash
cargo qat-devql-capabilities
```

Or equivalently:

```bash
cargo nextest run --features qat-tests --test qat_devql_capabilities --run-ignored only -- qat_devql_capabilities --exact
```

**Active coverage:**

- Agent enablement queryability (2 scenarios)
  - First Claude Code change is queryable through DevQL
  - Claude Code chat history is retrievable after edit and commit
- Blast radius and temporal correctness (5 scenarios)
  - dependency queries in both directions
  - current-workspace graph changes after edits
  - historical `asOf` correctness after ingest
  - repeated ingest idempotency for artefacts and edges
- TestHarness proof-map (5 scenarios)
  - summary, tests, and coverage views
  - explicitly untested artefacts
  - failing versus passing test visibility
- Semantic clones (8 scenarios)
  - historical ingest preserves core history without backfilling semantic-clone history
  - current projection populates current tables
  - current embeddings expose separate code and summary channels
  - sync drives embeddings before clone-edge rebuild fully drains
  - commit snapshots current semantic-clone data into historical tables
  - handler-clone ranking, explanations, and filtering
  - DevQL clone summary grouped counts
  - GraphQL clone summary grouped counts
- Deterministic Confluence knowledge flows (3 scenarios)
  - add/query/associate happy path for deterministic fixtures
  - refresh creates a new version
  - unsupported URL fails cleanly without partial persistence
- Cross-capability deterministic smoke (1 scenario)
  - checkpoints, sessions, artefacts, dependency queries, TestHarness queries, and clone queries compose in one repo

### 5. DevQL Ingest (`cargo qat-devql-ingest`, 14 scenarios)

Exercises the queue-backed `bitloops devql tasks enqueue --kind ingest` commit-history
replay flow: initial backlog ingest, idempotent replay, batching, merge and rewrite
handling, bounded backfill integration through `bitloops init --ingest=true`, and
direct enqueue backfill catch-up.

```bash
cargo qat-devql-ingest
```

Or equivalently:

```bash
cargo nextest run --features qat-tests --test qat_devql_ingest --run-ignored only -- qat_devql_ingest --exact
```

**Active coverage:**

- Initial backlog ingest completes all reachable history.
- Re-ingest at the same HEAD is idempotent.
- Two commits are ingested together in one replay.
- Commits made while the daemon is down are batched on the next ingest.
- Non-fast-forward merge ingests feature commits and the merge commit.
- Fast-forward merge ingests feature commits without creating a merge SHA.
- Cherry-pick ingests the cherry-picked SHAs.
- Rebase with edit rewrites SHAs and ingests rewritten history.
- Reset rewrite introduces replacement SHAs and ingests them.
- `bitloops init --ingest=true --backfill=1` ingests only the latest reachable commit.
- A later full ingest catches up after `--backfill=1`.
- `bitloops init --ingest=true --backfill=2` stays bounded and a later full ingest catches up.
- Direct enqueue with `--backfill 1` ingests only the latest reachable commit.
- A later direct enqueue full ingest catches up after `--backfill 1`.

### 6. Agents Checkpoints (`cargo qat-agents-checkpoints`, 5 scenarios)

This focused lane is not part of `cargo qat`. It validates checkpoint capture directly:
first checkpoint bootstrap, pre-commit interaction visibility, single-agent progression,
relative-day timeline integrity, and multi-agent checkpoint ordering.

```bash
cargo qat-agents-checkpoints
```

Or equivalently:

```bash
cargo nextest run --features qat-tests --test qat_agents_checkpoints --run-ignored only -- qat_agents_checkpoints --exact
```

**Active coverage:**

- Supported agent can complete bootstrap and create the first checkpoint.
- Agent interaction exists before the first checkpoint is committed.
- Single-agent checkpoint progression stays ordered across multiple commits.
- Single-agent checkpoint timeline stays coherent across yesterday and today.
- Multiple agents can interleave checkpoint activity without breaking history order.

## Runtime notes

- Smoke uses deterministic simulated agent behavior for non-Claude agents, and Claude can
  fall back to deterministic simulation when the external CLI or auth flow is unavailable.
- The semantic-clones lane uses a fake embeddings runtime on purpose; it is validating
  the deterministic offline path, not real local-model warm-cache behavior.
- Most DevQL QAT flows now run through `bitloops devql tasks enqueue --kind ...`
  rather than legacy direct `bitloops devql sync` or `bitloops devql ingest` commands.

## Daemon prerequisite

Most QAT flows require a running Bitloops daemon before `bitloops init`, `bitloops enable`,
and `bitloops devql ...` commands.

Feature setup should include:

`And I start the daemon in bitloops`

immediately after `Given I run CleanStart ...`.

QAT starts one daemon per scenario (isolated `HOME`/`XDG_CONFIG_HOME`) and stops it in the
scenario teardown hook.

## Artifacts and folder layout

QAT writes run artifacts under `<current working directory>/target/qat-runs`.

Common cases:

- repository root: `target/qat-runs`
- `bitloops/` crate directory: `target/qat-runs` in that directory
  - absolute path: `<repo>/bitloops/target/qat-runs`

Each QAT invocation creates one suite folder:

`<timestamp>-<short-id>`

Inside each suite folder, each scenario creates one run folder:

`<scenario-slug>-<flow-slug>-<short-id>`

Inside each scenario folder you will see:

- `run.json`: scenario metadata (scenario name, flow, paths, binary path, creation time)
- `terminal.log`: all executed commands with status/stdout/stderr
- `bitloops/`: isolated test repository used by the scenario
- `home/`: isolated HOME/XDG state for tools
- optional markers like `.qat-claude-fallback`

`.last-run` in the runs root points to the latest suite folder.

## Why many `qat-runs` folders appear

This is expected. QAT keeps historical suites and does not auto-delete old runs.

If you run QAT 15 times, you will have 15 top-level suite folders.

## Environment variables

- `BITLOOPS_QAT_BINARY` (override the binary under test; otherwise `CARGO_BIN_EXE_bitloops` is used)
- `BITLOOPS_QAT_MAX_CONCURRENT_SCENARIOS` (default `1`; per-suite scenario concurrency, separate from bundle-level scheduling under `cargo qat`, which runs onboarding + smoke in parallel and the DevQL suites serially)
- `BITLOOPS_QAT_COMMAND_TIMEOUT_SECS` (default `180`)
- `BITLOOPS_QAT_EVENTUAL_TIMEOUT_SECS` (default `15`; generic eventual-wait timeout for persistence-backed assertions)
- `BITLOOPS_QAT_TESTLENS_EVENTUAL_TIMEOUT_SECS` (default `15`; TestHarness query eventual window)
- `BITLOOPS_QAT_SEMANTIC_CLONES_EVENTUAL_TIMEOUT_SECS` (default `60`; semantic-clone eventual window)
- `BITLOOPS_QAT_DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_SECS` (default `60`; sync TestHarness capability-event wait timeout)
- `BITLOOPS_QAT_DAEMON_PORT` (force the per-scenario daemon port instead of the deterministic allocator)
- `BITLOOPS_QAT_CLAUDE_TIMEOUT_SECS` (default `30`)
- `BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS` (default `300`)
- `BITLOOPS_QAT_CLAUDE_CMD` (override Claude prompt command)
- `BITLOOPS_QAT_CLAUDE_AUTH_STATUS_CMD` (override auth status command)
- `BITLOOPS_QAT_CLAUDE_AUTH_LOGIN_CMD` (override auth login command)
- `BITLOOPS_QAT_DISABLE_CLAUDE_FALLBACK=1` (disable Claude fallback simulation)

Example:

```bash
BITLOOPS_QAT_COMMAND_TIMEOUT_SECS=300 cargo qat-devql-capabilities
```

## Troubleshooting

If a run appears stuck:

1. Check the active run:

```bash
cat target/qat-runs/.last-run
```

2. Inspect the scenario log:

```bash
head -200 <scenario-run-dir>/terminal.log
```

3. For long suites, expect delays around `devql init`,
   `devql tasks enqueue --kind sync --status`, and workspace indexing.
