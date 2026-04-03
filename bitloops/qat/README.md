# QAT Test Guide

This document explains how to run Bitloops QAT (Cucumber) journeys and how to inspect artifacts.

QAT runs as a standard Cargo integration test. The production binary no longer exposes a
`qat` subcommand, and no Cargo feature flag is required.

## Where to run from

Run commands from the repository root:

```bash
cargo test --manifest-path bitloops/Cargo.toml --test qat_acceptance qat_claude_code -- --ignored
```

If you are already in `bitloops/` (the Rust crate directory), you can run:

```bash
cargo test --test qat_acceptance qat_claude_code -- --ignored
```

## Test suites

QAT supports three main entry points:

1. Default Claude Code suite (2 scenarios):

```bash
cargo test --test qat_acceptance qat_claude_code -- --ignored
```

2. Smoke suite (2 scenarios):

```bash
cargo test --test qat_acceptance qat_smoke -- --ignored
```

3. DevQL suite (23 scenarios):

```bash
cargo test --test qat_acceptance qat_devql -- --ignored
```

## Daemon prerequisite

Most QAT flows now require a running Bitloops daemon before `bitloops init`, `bitloops enable`,
and `bitloops devql ...` commands.

Feature setup should include:

`And I start the daemon in bitloops`

immediately after `Given I run CleanStart ...`.

QAT starts one daemon per scenario (isolated `HOME`/`XDG_CONFIG_HOME`) and stops it in the
scenario teardown hook.

## Claude auth behavior

Claude-based scenarios include the step:

`I ensure Claude Code auth in bitloops`

It performs:

1. `claude auth status --json`
2. If not logged in: `claude auth login --claudeai`
3. Status check again

If auth still fails, the step fails.

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

## Runtime expectations

- `qat_smoke`: usually short
- `qat_claude_code`: moderate
- `qat_devql`: typically long (many scenarios; can take tens of minutes)

## Useful options and env vars

Cargo test selectors:

- `qat_smoke`: run only the smoke suite
- `qat_claude_code`: run only the Claude Code suite
- `qat_devql`: run only the DevQL suite

Environment variables:

- `BITLOOPS_QAT_BINARY` (override the binary under test; otherwise `CARGO_BIN_EXE_bitloops` is used)
- `BITLOOPS_QAT_MAX_CONCURRENT_SCENARIOS` (default `1`)
- `BITLOOPS_QAT_COMMAND_TIMEOUT_SECS` (default `180`)
- `BITLOOPS_QAT_CLAUDE_TIMEOUT_SECS` (default `30`)
- `BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS` (default `300`)
- `BITLOOPS_QAT_CLAUDE_CMD` (override Claude prompt command)
- `BITLOOPS_QAT_CLAUDE_AUTH_STATUS_CMD` (override auth status command)
- `BITLOOPS_QAT_CLAUDE_AUTH_LOGIN_CMD` (override auth login command)
- `BITLOOPS_QAT_DISABLE_CLAUDE_FALLBACK=1` (disable Claude fallback simulation)

Example:

```bash
BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS=600 cargo test --test qat_acceptance qat_claude_code -- --ignored
```

## Troubleshooting

If a run appears stuck:

1. Check the active run:

```bash
cat bitloops/target/qat-runs/.last-run
```

2. Inspect the scenario log:

```bash
sed -n '1,200p' <scenario-run-dir>/terminal.log
```

3. For long suites, expect delays around `devql init`, `devql ingest`, semantic clone rebuild, and knowledge ingestion.
