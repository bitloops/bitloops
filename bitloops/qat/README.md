# QAT Test Guide

This document explains how to run Bitloops QAT (Cucumber) journeys and how to inspect artifacts.

QAT runs as a standard Cargo integration test. The production binary no longer exposes a
`qat` subcommand, and no Cargo feature flag is required.

## Where to run from

All commands below assume you are inside the `bitloops/` crate directory.
Cargo aliases are configured in `.cargo/config.toml` and only resolve from this directory.

## Implemented test suites

### Onboarding + DevQL sync + Smoke (CI bundle)

Runs onboarding + DevQL sync in parallel, then smoke after that (41 scenarios total).

- PRs targeting `main`: runs automatically in `.github/workflows/ci.yml`
- For develop-target work: run `.github/workflows/develop-qat.yml` manually from the GitHub Actions UI and select the branch you want to test

```bash
cargo qat
```

Works from both the `bitloops/` crate directory and the repository root.

### 1. Smoke (13 scenarios)

Exercises the deterministic hook-driven agent lifecycle for `claude-code`, `cursor`,
`gemini`, `copilot`, `codex`, and `opencode` with `open-code` accepted as an alias.
This suite validates the Bitloops golden path without requiring any external agent CLIs
to be installed on the machine or CI runner.

```bash
cargo qat-smoke
```

Or equivalently:

```bash
cargo test --test qat_acceptance qat_smoke -- --ignored
```

**Scenarios:**

- First agent-driven Bitloops session is captured for each supported agent.
- Follow-up agent edits create progression for each supported agent.
- Preserve relative-day commit timeline.

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
cargo test --test qat_acceptance qat_onboarding -- --ignored
```

**Scenarios:**

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

### 3. DevQL Sync (15 scenarios)

Exercises the `devql sync` workspace reconciliation flow: full indexing, incremental
add/modify/delete detection, branch checkout, daemon downtime catch-up, git pull,
sync validation and repair, and `init --sync=true` integration.

```bash
cargo qat-devql-sync
```

Or equivalently:

```bash
cargo test --test qat_acceptance qat_devql_sync -- --ignored
```

**Scenarios:**

| #   | Scenario                                           | Flow                            |
| --- | -------------------------------------------------- | ------------------------------- |
| 1   | Full sync indexes workspace source files           | `SyncFullIndex`                 |
| 2   | Sync detects newly added source files              | `SyncNewFiles`                  |
| 3   | Sync detects modified source files                 | `SyncModifiedFiles`             |
| 4   | Sync removes artefacts for deleted files           | `SyncDeletedFiles`              |
| 5   | No-op sync reports zero changes                    | `SyncNoop`                      |
| 6   | Sync after branch checkout reflects new state      | `SyncBranchCheckout`            |
| 7   | Sync catches up after daemon downtime              | `SyncDaemonDowntime`            |
| 8   | Sync indexes changes from git pull                 | `SyncGitPull`                   |
| 9   | Sync validate detects drift (never synced)         | `SyncValidateDrift`             |
| 10  | Sync validate reports clean after full sync        | `SyncValidateClean`             |
| 11  | Sync validate detects drift after changes          | `SyncValidateDriftAfterChange`  |
| 12  | Sync repair restores clean state                   | `SyncRepair`                    |
| 13  | Init sync=true — follow-up sync reports no changes | `SyncInitSyncTrueNoop`          |
| 14  | Init sync=true — incremental sync for new files    | `SyncInitSyncTrueIncremental`   |
| 15  | Init sync=true — validation stays clean            | `SyncInitSyncTrueValidateClean` |

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
- optional markers like `.qat-claude-fallback`, `.qat-semantic-clones-fallback`, `.qat-knowledge-fallback`

`.last-run` in the runs root points to the latest suite folder.

## Why many `qat-runs` folders appear

This is expected. QAT keeps historical suites and does not auto-delete old runs.

If you run QAT 15 times, you will have 15 top-level suite folders.

## Environment variables

- `BITLOOPS_QAT_BINARY` (override the binary under test; otherwise `CARGO_BIN_EXE_bitloops` is used)
- `BITLOOPS_QAT_MAX_CONCURRENT_SCENARIOS` (default `1`; per-suite scenario concurrency, separate from the onboarding + DevQL sync + smoke bundle running in parallel under `cargo qat`)
- `BITLOOPS_QAT_COMMAND_TIMEOUT_SECS` (default `180`)
- `BITLOOPS_QAT_CLAUDE_TIMEOUT_SECS` (default `30`)
- `BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS` (default `300`)
- `BITLOOPS_QAT_CLAUDE_CMD` (override Claude prompt command)
- `BITLOOPS_QAT_CLAUDE_AUTH_STATUS_CMD` (override auth status command)
- `BITLOOPS_QAT_CLAUDE_AUTH_LOGIN_CMD` (override auth login command)
- `BITLOOPS_QAT_DISABLE_CLAUDE_FALLBACK=1` (disable Claude fallback simulation)

Example:

```bash
BITLOOPS_QAT_COMMAND_TIMEOUT_SECS=300 cargo qat-devql-sync
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

3. For long suites, expect delays around `devql init`, `devql sync`, and workspace indexing.
