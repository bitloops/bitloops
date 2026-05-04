# QAT

QAT in this repository is the acceptance-level, deterministic, mostly-offline quality harness for Bitloops. It is implemented as a set of ignored Rust integration tests backed by a shared Cucumber runner and a large helper layer under `bitloops/tests/qat_support/`.

This directory contains the QAT feature files. The actual execution engine lives in the Rust test harness.

## What QAT covers

QAT is not a single monolithic suite. It is a family of focused lanes that validate different product surfaces:

- onboarding and managed hook installation/removal
- supported agent checkpoint/session persistence
- DevQL current-state sync
- DevQL historical ingest and rewritten-history correctness
- dependency/blast-radius queries
- TestHarness ingest and proof-map queries
- semantic-clone enrichment and query behavior
- deterministic knowledge ingestion
- curated develop-gate coverage for merge-to-`develop`

The suite is intentionally DB-first and artifact-first. Many assertions do not stop at CLI success; they inspect persisted SQLite/DuckDB-backed state, queue state, checkpoint rows, capability-event runs, and query outputs.

## Where The Pieces Live

### Feature specs

- `bitloops/qat/features/onboarding/onboarding.feature`
- `bitloops/qat/features/smoke/agent_cli_smoke.feature`
- `bitloops/qat/features/smoke/checkpoint_timeline_smoke.feature`
- `bitloops/qat/features/agents-checkpoints/agents_checkpoints.feature`
- `bitloops/qat/features/devql/agent_enablement_queryable.feature`
- `bitloops/qat/features/devql/blast_radius_temporal.feature`
- `bitloops/qat/features/devql/cross_capability_smoke.feature`
- `bitloops/qat/features/devql/knowledge_ingestion.feature`
- `bitloops/qat/features/devql/semantic_clones.feature`
- `bitloops/qat/features/devql/test_harness_proof_map.feature`
- `bitloops/qat/features/devql-sync/sync_workspace.feature`
- `bitloops/qat/features/devql-ingest/ingest_workspace.feature`

### Harness entrypoints

- `bitloops/tests/qat.rs`
- `bitloops/tests/qat_agent_smoke.rs`
- `bitloops/tests/qat_develop_gate.rs`
- `bitloops/tests/qat_devql_capabilities.rs`
- `bitloops/tests/qat_devql_sync.rs`
- `bitloops/tests/qat_devql_ingest.rs`
- `bitloops/tests/qat_onboarding.rs`
- `bitloops/tests/qat_agents_checkpoints.rs`

### Shared harness code

- `bitloops/tests/qat_support/entrypoints.rs`
- `bitloops/tests/qat_support/runner.rs`
- `bitloops/tests/qat_support/world.rs`
- `bitloops/tests/qat_support/steps/mod.rs`
- `bitloops/tests/qat_support/helpers/*.rs`
- `bitloops/tests/qat_support/subsets.rs`

## How QAT is wired

### Cargo surface

The source of truth for the current QAT commands is:

- `.cargo/config.toml` for the repo-root aliases
- `bitloops/Cargo.toml` for the `[[test]]` targets and required features

Current checked-in aliases:

```bash
cargo qat
cargo qat-agent-smoke
cargo qat-develop-gate
cargo qat-devql-capabilities
cargo qat-devql-sync
cargo qat-devql-ingest
cargo qat-onboarding
cargo qat-agents-checkpoints
```

All of those run through `cargo-nextest` from the repository root and enable the `qat-tests` feature on the `bitloops` crate.

There is also an internal-only harness target:

```bash
cargo test \
  --manifest-path bitloops/Cargo.toml \
  --features qat-tests,qat-internal-tests \
  --test qat_internal
```

That target compiles internal unit tests for the QAT runner/helpers and is intentionally kept out of the normal focused QAT lanes.

### Test target model

Each public QAT lane is an ignored async integration test. The ignored test body is thin: it resolves the `bitloops` binary and delegates into the shared harness.

Examples:

- `qat_agent_smoke` runs `Suite::AgentSmoke`
- `qat_devql_sync` runs `Suite::DevqlSync`
- `qat_develop_gate` runs a serial filtered orchestration entrypoint instead of a single suite
- `qat` runs the bundled multi-suite lane

### Binary resolution

By default, QAT runs the freshly built `bitloops` test binary via `env!("CARGO_BIN_EXE_bitloops")`.

You can override that with:

```bash
BITLOOPS_QAT_BINARY=/absolute/path/to/bitloops
```

The override is validated eagerly and the path must exist.

### Per-suite binary snapshotting

Before a suite starts, the harness copies the `bitloops` binary into that suite's artifact directory and runs the suite from the copy, not the original binary. This keeps suites isolated from one another and avoids accidental interference from mutable build products.

On macOS, the runner also handles the DuckDB runtime specificity:

- it checks whether the binary links `@rpath/libduckdb.dylib`
- if needed, it adds `@executable_path` to the snapshot binary's rpath
- it stages `libduckdb.dylib` next to the snapshot binary

That behavior exists so per-suite binary snapshots keep working even when DuckDB is dynamically linked.

## Execution model

### Suite routing

`Suite` in `bitloops/tests/qat_support/runner.rs` maps logical suites to directories or files:

- `AgentSmoke` -> `bitloops/qat/features/smoke`
- `Devql` -> `bitloops/qat/features/devql`
- `DevqlSync` -> `bitloops/qat/features/devql-sync`
- `DevqlIngest` -> `bitloops/qat/features/devql-ingest/ingest_workspace.feature`
- `Onboarding` -> `bitloops/qat/features/onboarding`
- `AgentsCheckpoints` -> `bitloops/qat/features/agents-checkpoints`

`DevqlIngest` is deliberately a single file target, while the other suites point at directories.

### Bundle behavior

`cargo qat` is not just "run every suite in parallel."

The bundle entrypoint does this:

1. Run `onboarding` and `agent-smoke` in parallel.
2. Run `devql-sync`.
3. Run `devql-capabilities`.
4. Run `devql-ingest`.

The DevQL-heavy phase is serialized on purpose because those scenarios are more likely to contend on SQLite-backed materialization paths if fully fanned out.

The bundle currently does **not** include the dedicated `agents-checkpoints` lane.

### Develop gate behavior

`cargo qat-develop-gate` is a curated serial run across these suites:

- `AgentSmoke`
- `DevqlSync`
- `Devql`
- `DevqlIngest`
- `AgentsCheckpoints`

Purpose of the develop gate:

- it is the small, high-signal QAT subset intended to run in CI for merges to `develop`
- it is not a separate set of bespoke gate-only scenarios
- it reuses existing scenarios and scenario-outline example rows that already live in the normal QAT feature files
- membership is determined by tagging those existing scenarios or `Examples:` blocks with `@develop_gate`

In practice, `cargo qat-develop-gate` walks the existing suite set above, applies an explicit Cucumber tag expression of `@develop_gate`, and runs only the tagged subset. That is how the same scenarios can participate both in their full focused lane and in the smaller CI gate for `develop`.

Because of that design, the source of truth for the gate is in code and feature tags:

- suite selection in `bitloops/tests/qat_support/subsets.rs`
- filtered orchestration in `bitloops/tests/qat_support/entrypoints.rs`
- scenario membership via `@develop_gate` tags in `bitloops/qat/features/**`

Important detail: the explicit develop-gate tag filter wins over `CUCUMBER_FILTER_TAGS` for that lane.

### Cucumber runtime specifics

The harness configures Cucumber with:

- a single shared step collection from `steps::collection()`
- `fail_on_skipped()`, so missing or skipped steps fail the suite
- `with_default_cli()`, so direct `cargo test` invocations still get normal Cucumber CLI behavior
- default `max_concurrent_scenarios = 1`

The scenario concurrency can be raised with:

```bash
BITLOOPS_QAT_MAX_CONCURRENT_SCENARIOS=4
```

If unset or invalid, it falls back to `1`.

### Tag filtering

Focused suites support Cucumber tag expressions via:

```bash
CUCUMBER_FILTER_TAGS='@test_harness_sync'
```

The filter is applied against the merged feature, rule, and scenario tags. Empty or missing values disable filtering.

## Isolation model

QAT is designed to avoid machine-global leakage.

### Per-suite artifacts root

Every suite run gets a new directory under:

```text
target/qat-runs/<rfc3339-ish-timestamp>-<8char-id>/
```

The runner also writes:

- `target/qat-runs/.last-run`

That file points at the latest suite artifact directory.

### Per-scenario run directory

Within the suite root, `CleanStart` creates a scenario-specific run directory:

```text
<suite-root>/<scenario-slug>-<flow-slug>-<8char-id>/
```

That directory contains:

- `bitloops/` as the repo under test
- `terminal.log` with every command and helper note
- `run.json` with scenario metadata
- `daemon.stderr.log` if a daemon was started
- `home/` for the scenario-isolated HOME/XDG state
- agent transcript files where relevant

### Scenario metadata

`run.json` records:

- scenario name
- scenario slug
- flow name
- run dir
- repo dir
- terminal log path
- binary path
- creation timestamp

### Scenario HOME/XDG isolation

Every `bitloops` command is run with a scenario-local environment:

- `HOME`
- `USERPROFILE`
- `XDG_CONFIG_HOME`
- `XDG_STATE_HOME`
- `XDG_CACHE_HOME`
- `XDG_DATA_HOME`

This matters because QAT exercises daemon config/state, runtime stores, hook-triggered flows, and agent session persistence. Without HOME/XDG isolation, hooks and daemon lookups could hit system state instead of the scenario sandbox.

### Git isolation details

QAT also sets git-specific behavior carefully:

- `git init -q` is run inside the scenario repo
- git identity is configured as `Bitloops QAT <bitloops-qat@example.com>`
- commit signing is disabled
- `git add -A` excludes `:(exclude).bitloops/stores`, so repo-local stores are never committed into the scenario repo history
- the QAT binary directory is prepended to `PATH` so git hooks resolve the same binary snapshot the suite is using
- `HOME` and `XDG_STATE_HOME` are also set on git commands, so hook-triggered work stays pointed at the scenario daemon/runtime state
- helper-driven topology/rewrite commits sometimes set `BITLOOPS_DISABLE_POST_COMMIT_DEVQL_REFRESH=1` so QAT can control exactly when sync/ingest follow-up work happens

### Default command hardening

Every QAT `bitloops` command also sets:

- `BITLOOPS_QAT_ACTIVE=1`
- `BITLOOPS_TEST_TTY=0`
- `ACCESSIBLE=1`
- watcher autostart disabled
- version check disabled
- DevQL embedding provider disabled
- DevQL semantic provider disabled
- DevQL Postgres/ClickHouse env vars removed

This is why semantic-clone scenarios must explicitly inject their own fake runtime config before they can exercise that surface.

## Daemon model

QAT can start a foreground daemon per scenario.

### Startup behavior

The daemon harness:

- picks candidate ports deterministically from the run directory hash
- optionally honors `BITLOOPS_QAT_DAEMON_PORT`
- starts `bitloops daemon start --create-default-config --no-telemetry --http --host 127.0.0.1 --port <candidate>`
- captures stderr to `daemon.stderr.log`

### Readiness check

Readiness is not assumed from process spawn alone. QAT waits until:

- the daemon runtime-state row exists and points at the spawned PID
- an HTTP probe to `/devql/sdl` returns `200`

This is stricter than simply waiting for a port bind.

### Teardown behavior

After each scenario, the harness tries `bitloops daemon stop`, logs failures, and then force-kills the foreground child if it is still alive.

## World state and step contract

`QatWorld` in `world.rs` is the state bag shared across steps. It carries far more than just paths:

- run configuration and paths
- last command stdout/exit code
- daemon URL/process/runtime-state paths
- captured commit SHAs
- expected SHAs and expected paths for ingest topology assertions
- rewrite snapshots for rebase/reset tests
- semantic-clone table snapshots and enrichment observations
- knowledge fixture URLs and captured knowledge item ids
- last TestHarness capability-event baseline
- last captured DevQL task id
- current agent name

`steps/mod.rs` is the public step vocabulary. It is regex-driven and assembles one shared `Collection<QatWorld>`.

Two important specifics:

- the step layer currently only supports the repo name `bitloops`; helpers reject any other repo name
- many `When` registrations intentionally reuse the same helper functions as `Given`, so the Gherkin reads naturally without separate execution logic

## Agent simulation specifics

The smoke/checkpoint scenarios are deliberately offline and deterministic. They do not rely on real remote agent APIs to edit files.

Current public steps route agent prompts through deterministic helpers that:

- mutate known fixture files in predictable ways
- write synthetic agent transcripts
- invoke the corresponding Bitloops hook entrypoints so Bitloops persists session/interaction state as if a real agent session had happened

Supported agents in the current smoke surface:

- `claude-code`
- `cursor`
- `gemini`
- `copilot`
- `codex`
- `opencode` / `open-code`

Per-agent hook simulation details:

| Agent | Hook flow simulated by QAT |
| --- | --- |
| `claude-code` | `session-start` -> `user-prompt-submit` -> `stop` |
| `cursor` | `before-submit-prompt` -> transcript append -> `stop` |
| `gemini` | `session-start` -> `before-agent` -> transcript append -> `after-agent` |
| `copilot` | `session-start` -> `user-prompt-submitted` -> transcript append -> `agent-stop` |
| `codex` | `session-start` -> transcript append -> `stop` |
| `opencode` | `session-start` -> `turn-start` -> transcript append -> `turn-end` |

Transcript locations vary by adapter:

- most agents: `<run-dir>/agent-sessions/<agent>/<session-id>.<jsonl|json>`
- Copilot: `<run-dir>/home/.copilot/session-state/<session-id>/events.jsonl`

Session persistence assertions are adapter-aware. They check both the expected transcript path and the persisted Bitloops session/context state.

There is older helper code for external Claude auth/fallback plumbing, but the currently registered public prompt steps use the deterministic offline path instead.

## Suite-by-suite behavior

### 1. Onboarding

Feature file:

- `bitloops/qat/features/onboarding/onboarding.feature`

What it validates:

- `bitloops --version`
- daemon config bootstrap from scratch
- relational/event/blob store paths exist
- repo-local enable/init flow
- managed hook installation per supported agent
- `sync=true` init path
- disable flow
- hook-only uninstall
- full uninstall of managed hooks

Hook path assertions are agent-specific:

| Agent | Expected managed file |
| --- | --- |
| `claude-code` | `.claude/settings.json` |
| `codex` | `.codex/hooks.json` |
| `cursor` | `.cursor/hooks.json` |
| `gemini` | `.gemini/settings.json` |
| `copilot` | `.github/hooks/bitloops.json` |
| `open-code` | `.opencode/plugins/bitloops.ts` |

QAT also checks the managed post-commit hook content, not just file existence.

### 2. Agent Smoke

Feature files:

- `bitloops/qat/features/smoke/agent_cli_smoke.feature`
- `bitloops/qat/features/smoke/checkpoint_timeline_smoke.feature`

What it validates:

- first-session capture for supported agents
- follow-up edit progression
- checkpoint mapping persistence
- relative-day commit timeline preservation

The `agent_cli_smoke.feature` file is a scenario-outline matrix. Only selected example blocks are tagged `@develop_gate`.

### 3. Agents Checkpoints

Feature file:

- `bitloops/qat/features/agents-checkpoints/agents_checkpoints.feature`

What it validates:

- first checkpoint creation from an agent-edited repo
- pre-commit interaction persistence before checkpoint condensation
- multiple committed checkpoints stay ordered
- yesterday/today checkpoint timeline coherence
- multi-agent interleaving without history corruption

This suite uses a deterministic Rust project fixture rather than the Vite scaffold used in smoke.

### 4. DevQL capabilities

Feature files:

- `bitloops/qat/features/devql/agent_enablement_queryable.feature`
- `bitloops/qat/features/devql/blast_radius_temporal.feature`
- `bitloops/qat/features/devql/cross_capability_smoke.feature`
- `bitloops/qat/features/devql/knowledge_ingestion.feature`
- `bitloops/qat/features/devql/semantic_clones.feature`
- `bitloops/qat/features/devql/test_harness_proof_map.feature`

This suite is the broadest one. It mixes several DevQL surfaces:

- checkpoint queries
- chatHistory
- dependency graph queries
- TestHarness proof-map queries
- semantic-clone queries and summaries
- knowledge add/refresh/association
- a cross-capability integrated smoke

Specific implementation notes:

- dependency assertions use GraphQL `incomingDeps`/`outgoingDeps` under the hood for precise counts and `asOf(commit: ...)` temporal scoping
- TestHarness assertions normalize the query output because the raw payload shapes differ between summary/tests/coverage views
- clone summary coverage is exercised through both the DevQL DSL and GraphQL `cloneSummary`

### 5. DevQL sync

Feature file:

- `bitloops/qat/features/devql-sync/sync_workspace.feature`

This suite focuses on current workspace reconciliation and task queue behavior.

It covers:

- full sync baseline indexing
- TestHarness materialization during sync
- deletion of test coverage after test-file deletion
- added/changed/removed workspace files
- no-op sync
- branch checkout
- daemon downtime catch-up
- git pull catch-up
- validate/repair flows
- path-scoped sync
- explicit full sync
- `--require-daemon` failure mode
- queue observability, pause/resume, and cancellation
- `bitloops init --sync=true` semantics

Important specificity: many sync assertions do not just parse CLI summary text. They inspect persisted runtime queue state and completed sync summaries from the daemon runtime store to prove the correct HEAD and change counts were materialized.

### 6. DevQL ingest

Feature file:

- `bitloops/qat/features/devql-ingest/ingest_workspace.feature`

This suite is explicitly history-first, not current-state-first.

It covers:

- initial backlog replay
- idempotent re-ingest
- batching of multiple commits
- daemon-downtime backlog replay
- non-fast-forward merge topology
- fast-forward merge topology
- cherry-pick behavior
- rebase-edit rewrite handling
- reset-and-rewrite handling
- bounded backfill via `init --backfill`
- bounded backfill via direct enqueue

The assertions are DB-first. They look at:

- `commit_ingest_ledger`
- `file_state`
- `artefacts_current`
- reachable git SHAs vs completed ledger SHAs

Rewrite scenarios also capture pre-rewrite reachable segments and prove old SHAs disappear while new SHAs become ledger-complete.

## TestHarness specifics

The TestHarness helpers build deterministic repo fixtures with:

- source files
- test files
- `coverage/lcov.info`
- per-test LLVM JSON coverage
- Jest result JSON fixtures

QAT exercises three ingestion commands:

```bash
bitloops devql test-harness ingest-tests
bitloops devql test-harness ingest-coverage
bitloops devql test-harness ingest-results
```

For sync-driven TestHarness materialization, QAT also waits on daemon-side capability-event completion. It checks both:

- `bitloops daemon status --json`
- persisted runtime-store records in `capability_workplane_cursor_runs` and `pack_reconcile_runs`

That is why the TestHarness sync scenarios can assert more than "the command exited successfully."

## Semantic clone specifics

The semantic-clone lane is intentionally offline and deterministic.

### Fake runtime

QAT writes scenario-local config that points semantic-clone inference at a tiny fake embeddings runtime script. That config:

- uses deterministic embedding mode
- enables `summary_mode = "auto"`
- uses two enrichment workers
- defines fake code and summary embedding profiles
- writes config into both repo-local and scenario-global config locations

### Pack health gate

Before clone scenarios run, QAT checks `bitloops devql packs --json --with-health` and requires these health checks to be healthy:

- `semantic_clones.profile_resolution`
- `semantic_clones.runtime_command`
- `semantic_clones.runtime_handshake`

### Assertions

The semantic-clone helpers inspect both current and historical tables, including:

- `symbol_features`
- `symbol_semantics`
- `symbol_embeddings`
- `symbol_clone_edges`
- current-table counterparts
- representation-kind counts for `code` and `summary`

The suite distinguishes:

- ingest-only historical replay
- current projection from sync
- enrichment drain behavior
- clone ranking quality
- explanation payload presence
- grouped summary output

One especially specific scenario samples daemon enrichment status repeatedly and proves embeddings appear before clone-edge rebuild work fully drains.

## Knowledge specifics

Knowledge scenarios stay offline by spinning up a tiny local HTTP stub server on `127.0.0.1:<ephemeral-port>`.

The stub:

- serves queued deterministic Confluence REST payloads
- only supports GET requests
- is wired into provider config written into the scenario repo/config
- triggers a daemon restart if needed so the daemon picks up the new provider config

The knowledge assertions inspect:

- DevQL knowledge query results
- provider/source kind values
- persisted knowledge associations in `knowledge_relation_assertions`
- version counts after refresh
- clean failure on unsupported URLs with zero partial persistence

## Failure reporting and artifacts

QAT failure messages are designed to be rerunnable and artifact-oriented.

### Suite-level failure output

On suite failure the runner reports:

- suite id
- rerun command
- feature path
- parsing error count
- skipped step count
- artifact directory
- failed scenario names and feature line numbers when available

### Bundle-level failure output

If the bundled lane fails, the bundle aggregator keeps all failing suite names and includes a focused rerun hint for each one.

### What to inspect first

For a failing scenario, the most useful files are usually:

- `target/qat-runs/.last-run`
- `<run-dir>/terminal.log`
- `<run-dir>/run.json`
- `<run-dir>/daemon.stderr.log`
- scenario-local transcript files under `<run-dir>/agent-sessions/`

## Useful environment variables

### Runner selection and filtering

- `BITLOOPS_QAT_BINARY`
- `BITLOOPS_QAT_MAX_CONCURRENT_SCENARIOS`
- `CUCUMBER_FILTER_TAGS`

### Generic timeouts

- `BITLOOPS_QAT_COMMAND_TIMEOUT_SECS`
- `BITLOOPS_QAT_EVENTUAL_TIMEOUT_SECS`

### TestHarness and semantic-clone polling

- `BITLOOPS_QAT_TESTLENS_EVENTUAL_TIMEOUT_SECS`
- `BITLOOPS_QAT_DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_SECS`
- `BITLOOPS_QAT_SEMANTIC_CLONES_EVENTUAL_TIMEOUT_SECS`

### Daemon startup

- `BITLOOPS_QAT_DAEMON_PORT`

### Claude-only legacy helper plumbing

- `BITLOOPS_QAT_CLAUDE_TIMEOUT_SECS`
- `BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS`
- `BITLOOPS_QAT_CLAUDE_AUTH_STATUS_CMD`
- `BITLOOPS_QAT_CLAUDE_AUTH_LOGIN_CMD`
- `BITLOOPS_QAT_CLAUDE_CMD`
- `BITLOOPS_QAT_DISABLE_CLAUDE_FALLBACK`

Those Claude-specific variables mainly matter for the older external-Claude helper path, not the current deterministic smoke-step path.

## How to run QAT during development

### Typical focused runs

```bash
cargo qat-agent-smoke
cargo qat-agents-checkpoints
cargo qat-devql-sync
cargo qat-devql-ingest
cargo qat-devql-capabilities
cargo qat-onboarding
```

### Develop gate

```bash
cargo qat-develop-gate
```

### Full bundled lane

```bash
cargo qat
```

### Stream Cucumber output directly

When you need raw step-by-step output instead of the alias:

```bash
CUCUMBER_FILTER_TAGS='@test_harness_sync' \
cargo test \
  --manifest-path bitloops/Cargo.toml \
  --features qat-tests \
  --test qat_devql_sync \
  qat_devql_sync \
  -- --ignored --nocapture
```

## Maintainer notes

### Source of truth

For current suite names and cargo aliases, trust:

- `.cargo/config.toml`
- `bitloops/Cargo.toml`
- `bitloops/tests/qat_support/runner.rs`

The human-readable docs outside this folder can lag renames.

### Adding to the develop gate

The actual filtered orchestration is code-driven via `@develop_gate`.

- tag standalone scenarios directly with `@develop_gate`
- for scenario outlines, tag the specific `Examples:` block you want in the gate
- keep the gate description in this README aligned with the actual suite selection and tags

### Adding new step behavior

The contract is centralized:

- regex registration in `bitloops/tests/qat_support/steps/mod.rs`
- implementation in `given.rs`, `then.rs`, and helpers

Because the runner uses `fail_on_skipped()`, partial step wiring is immediately visible.

### Determinism expectations

New QAT coverage should preserve the current design principles:

- no external network dependency
- no reliance on a user's real HOME/XDG state
- deterministic fixture repos
- assertions against persisted state, not only terminal text
- focused suites with rerunnable failure output

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
| 11  | Disable stops capture and status reflects disabled | `disable-repo`                 |
| 12  | Uninstall removes agent and git hooks              | `uninstall-repo`               |
| 13  | Full uninstall removes all artefacts               | `uninstall-full`               |

### 3. DevQL Sync (`cargo qat-devql-sync`, 23 scenarios)

Exercises the queue-backed `bitloops devql tasks enqueue --kind sync` workspace
reconciliation flow: full indexing, TestHarness current-state sync, incremental
add/modify/delete detection, branch checkout, daemon downtime catch-up, git pull,
validation and repair, explicit full and path-scoped modes, task queue control,
`--require-daemon` failure handling, and watcher-driven `init --sync=true`
added-file materialization.

```bash
cargo qat-devql-sync
```

Or equivalently:

```bash
cargo nextest run --features qat-tests --test qat_devql_sync --run-ignored only -- qat_devql_sync --exact
```

**Active scenarios:**

| #   | Scenario                                                                  | Flow                            |
| --- | ------------------------------------------------------------------------- | ------------------------------- |
| 1   | Full sync indexes workspace source files                                  | `SyncFullIndex`                 |
| 2   | Sync materializes test-harness coverage for discovered tests              | `SyncTestHarnessPopulate`       |
| 3   | Sync removes test-harness coverage when test files are deleted            | `SyncTestHarnessDeleteTestFile` |
| 4   | Sync detects newly added source files                                     | `SyncNewFiles`                  |
| 5   | Sync detects and re-indexes modified source files                         | `SyncModifiedFiles`             |
| 6   | Sync removes artefacts for deleted source files                           | `SyncDeletedFiles`              |
| 7   | No-op sync reports zero changes                                           | `SyncNoop`                      |
| 8   | Sync after branch checkout reflects the new branch state                  | `SyncBranchCheckout`            |
| 9   | Sync catches up after daemon downtime with accumulated changes            | `SyncDaemonDowntime`            |
| 10  | Sync indexes changes introduced by git pull                               | `SyncGitPull`                   |
| 11  | Sync validate reports clean after a full sync                             | `SyncValidateClean`             |
| 12  | Sync repair restores clean state after drift                              | `SyncRepair`                    |
| 13  | Init with sync=true makes immediate follow-up sync report no changes      | `SyncInitSyncTrueNoop`          |
| 14  | Watcher-driven materialization after init --sync=true                     | `SyncInitSyncTrueIncremental`   |
| 15  | Init with sync=true keeps sync validation clean without workspace changes | `SyncInitSyncTrueValidateClean` |

The helper layer still supports negative drift assertions, and the active sync set now
includes drift validation plus accumulated-drift coverage.

### 4. DevQL Capabilities (`cargo qat-devql-capabilities`, 25 scenarios)

Exercises the strict offline DevQL capability surface: agent queryability,
checkpoint and chat-history retrieval, artefact-scoped dependency blast radius,
artefact-scoped TestHarness proof-map queries, guide-aligned deterministic
semantic clones, architecture graph entry-point/assertion reads, deterministic
Confluence knowledge add/query/associate/refresh, knowledge rejection handling,
and one final integrated cross-capability smoke.
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
- Architecture graph (1 scenario)
  - CLI entry points from language/config evidence plus assert, suppress, and revoke behaviour
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
relative-day timeline integrity, and multi-agent checkpoint ordering. `cargo qat-quickstart`
remains as a temporary compatibility alias to the same suite.

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

- Smoke uses deterministic simulated agent behavior for non-Claude agents rather than
  depending on real external agent CLIs.
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

QAT writes run artifacts under:

- `target/qat-runs` when running from `bitloops/`
- `bitloops/target/qat-runs` when running from repository root with `--manifest-path bitloops/Cargo.toml`

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
- `BITLOOPS_QAT_EVENTUAL_TIMEOUT_SECS` (default `120`)
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
