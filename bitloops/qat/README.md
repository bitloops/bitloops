# QAT Test Guide

This document explains how to run Bitloops QAT (Cucumber) journeys and how to inspect artifacts.

QAT is behind a non-default Cargo feature. This means default `cargo test` or `cargo run`
does not include the `qat` command unless you explicitly enable `--features qat`.

## Where to run from

Run commands from the repository root:

```bash
cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat
```

If you are already in `bitloops/` (the Rust crate directory), you can run:

```bash
cargo run --features qat -- qat
```

## Test suites

QAT supports three main entry points:

1. Default Claude Code suite (2 scenarios):

```bash
cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat
```

2. Smoke suite (2 scenarios):

```bash
cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat --smoke
```

3. DevQL suite (23 scenarios):

```bash
cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat --devql
```

You can also run a single feature file:

```bash
cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat --feature bitloops/qat/features/devql/blast_radius_temporal.feature
```

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

- `--smoke`: usually short
- default Claude suite: moderate
- `--devql`: typically long (many scenarios; can take tens of minutes)

## Useful options and env vars

CLI options:

- `--feature <path>`: run one feature file or directory
- `--runs-dir <path>`: custom artifacts root
- `--concurrency <n>`: max concurrent scenarios

Environment variables:

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
BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS=600 cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat
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
