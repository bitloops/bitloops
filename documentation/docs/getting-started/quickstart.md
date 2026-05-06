---
sidebar_position: 1
title: Quickstart
---

# Quickstart

This quickstart assumes you want the current daemon-first Bitloops setup.

If you are coming from the old JSON and repo-local storage model, read the [upgrade note](../reference/upgrading-to-the-daemon-architecture.md).

## 1. Install Bitloops

Choose one install method:

```bash
curl -fsSL https://bitloops.com/install.sh | bash
```

```bash
brew tap bitloops/tap && brew install bitloops
```

```bash
cargo install bitloops
```

## 2. Fastest Start: Initialise A Project

```bash
bitloops init --install-default-daemon
```

This is the fastest way to get started from inside a git repository or subproject. It bootstraps the default daemon service if needed, creates or updates `.bitloops.local.toml`, adds it to `.git/info/exclude`, and installs or reconciles hooks for the selected agents.

Use `--sync=true` when you want the initial current-state sync immediately:

```bash
bitloops init --install-default-daemon --sync=true
```

When you use `bitloops init --install-default-daemon` and embeddings are not already configured, interactive init asks whether to use Bitloops cloud, the local runtime, or skip embeddings for now. Bitloops cloud is the recommended default. If you choose the local runtime, Bitloops installs the managed standalone `bitloops-local-embeddings` binary when needed and warms that profile. If init also runs sync or ingest, that managed runtime download happens afterwards.

When Bitloops inference is not already configured, the same init flow asks whether to enable Bitloops inference with Bitloops cloud, local Ollama, or skip it. Enabling it configures semantic summaries, context guidance, architecture fact synthesis, and architecture role adjudication together. In non-interactive mode, pass `--bitloops-inference-runtime local`, `--bitloops-inference-runtime platform`, or `--no-bitloops-inference`.

In an interactive terminal, plain `bitloops init` also asks whether you want to install that same default local embeddings setup when embeddings are still unconfigured.

`bitloops init` can also queue an initial DevQL current-state sync after hook setup. Use `--sync=true` when you want that sync immediately, or `--sync=false` when you want to skip it. If you omit `--sync` in an interactive terminal, Bitloops asks after hook installation whether you want to sync the codebase now.

In non-interactive mode, `bitloops init` requires `--sync=true` or `--sync=false`.

That initial sync only reconciles current workspace state. Use `--ingest=true` during init, or run `bitloops devql tasks enqueue --kind ingest` separately, when you want checkpoint, commit, and event history materialised.

If you want to pin the supported agent set during bootstrap, repeat `--agent <name>` for each supported agent. For example:

```bash
bitloops init --sync=false --agent claude-code --agent codex
```

If telemetry consent is unresolved for an existing daemon config, interactive `bitloops init` can ask again. Non-interactive runs require an explicit telemetry flag.

## 3. Start The Daemon Explicitly When You Need To

```bash
bitloops start --create-default-config
```

Use this path when you want to bootstrap the default daemon before initialising a repo, or when you want to inspect or customise the daemon config separately.

On a fresh machine, use `--create-default-config` once. This writes the default global daemon config at the platform config location and creates the default local SQLite, DuckDB, and blob-store paths.

Interactive `bitloops start` also prompts to create the default config when it is missing. During that first bootstrap, Bitloops asks for telemetry consent unless you pass `--telemetry`, `--telemetry=false`, or `--no-telemetry`.

If you are using a repo-scoped or test-specific daemon config instead of the default global config, create the local file-backed stores for that config with:

```bash
bitloops start --config ./config.toml --bootstrap-local-stores
```

## 4. Add Optional Shared Project Policy

If you want shared capture policy in git, create `.bitloops.toml` in the project root:

```toml title=".bitloops.toml"
[capture]
enabled = true
strategy = "manual-commit"

[watch]
watch_debounce_ms = 750
watch_poll_fallback_ms = 2500
```

Keep `.bitloops.local.toml` for local-only overrides.

## 5. Start Or Open Bitloops

Open the dashboard:

```bash
bitloops dashboard
```

Or manage the daemon yourself:

```bash
bitloops start -d
bitloops start --until-stopped
```

## 6. Query And Ingest

The daemon automatically initialises the DevQL schema on startup. You can ingest and query immediately:

```bash
bitloops devql tasks enqueue --kind ingest
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->limit(10)'
```

DevQL CLI queries are DSL only when the input contains `->`. Otherwise the CLI treats the input as raw GraphQL.

## 7. Sync Current State

When you want to reconcile `artefacts_current`/`artefact_edges_current` with the current workspace:

```bash
bitloops devql tasks enqueue --kind sync
bitloops devql tasks enqueue --kind sync --status
```

By default, `bitloops devql tasks enqueue --kind sync` queues a sync task and returns immediately after printing the task id. Use `--status` when you want the CLI to follow that task until it reaches a terminal state.

When you want to validate that current-state rows match a full-project reconciliation without writing changes:

```bash
bitloops devql tasks enqueue --kind sync --validate --status
```

Use `--validate` as a diagnostic check when debugging drift between source files and current-state query results.

## 8. Check Status

```bash
bitloops status
bitloops checkpoints status --detailed
```

`bitloops status` reports daemon status. `bitloops checkpoints status` reports repo capture status and shows the resolved policy root and fingerprint.

`bitloops status` also shows sync queue totals, and when you run it inside a repo it includes the active or most recent sync task for that repo.

## Toggle Capture Later

```bash
bitloops disable
bitloops disable --devql-guidance
bitloops enable
bitloops enable --capture
bitloops enable --devql-guidance
bitloops enable --capture --devql-guidance
bitloops enable --install-embeddings
bitloops daemon enable --install-embeddings
```

With no target flags in an interactive terminal, `bitloops enable` and `bitloops disable` open a picker for `Capture` and `DevQL Guidance`. In non-interactive mode you must pass explicit target flags.

`--capture` toggles `[capture].enabled` and leaves installed hooks in place. `--devql-guidance` toggles the managed repo-local DevQL guidance surface without changing capture state. `bitloops daemon enable` is an alias to the same implementation.

Use `--install-embeddings` when you want Bitloops to add the default local embeddings profile to the effective daemon config and run the existing runtime warm/bootstrap path. When that path targets the default local runtime, Bitloops installs the managed standalone `bitloops-local-embeddings` binary automatically. In an interactive terminal, plain `bitloops enable` offers that setup automatically with a default-yes `[Y/n]` prompt when embeddings are not already configured.

Embeddings flags require `--capture`. Guidance-only enable does not prompt for telemetry or embeddings setup unless you pass an explicit telemetry flag.

If telemetry consent is unresolved for an existing daemon config, interactive `bitloops enable` can ask again before it edits project policy.

## Remove Bitloops Later

Use `bitloops disable --capture` when you want hooks and watchers to stay installed but stop capturing.

Use `bitloops uninstall` when you want to remove Bitloops-managed machine artefacts as well:

```bash
bitloops uninstall --full
```
