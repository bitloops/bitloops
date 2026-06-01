---
sidebar_position: 1
title: Quickstart
---

# Quickstart

This quickstart assumes you want the current daemon-first Bitloops setup.

If you are coming from the old JSON and repo-local storage model, read the [upgrade note](../reference/upgrading-to-the-daemon-architecture.md).

## 1. Install Bitloops With The Default Config

For the fastest scripted setup, pass the installer default-config flag. This installs Bitloops, writes the generated default daemon config, runs the non-interactive configure flow for that default config, creates the default local stores, and starts or reuses the daemon.

macOS, Linux, WSL:

```bash
curl -fsSL https://bitloops.com/install.sh | bash -s -- --default-config
```

or using 2 commands

```
curl -fsSL https://bitloops.com/install.sh | bash
bitloops configure --default-config
```

Windows PowerShell:

```powershell
& ([scriptblock]::Create((irm https://bitloops.com/install.ps1))) -DefaultConfig
```

Windows CMD:

```cmd
curl.exe -fsSL https://bitloops.com/install.cmd -o install.cmd
install.cmd --default-config
```


## 2. Initialise A Project

```bash
bitloops init
```

## 3. Start The Daemon Explicitly When You Need To

```bash
bitloops start -d
```

## 4. Edit Repo Policy

If you want to change repo specific policy that you selected at init, edit `bitloops.local.toml` 

## 5. Start Or Open Bitloops

Open the dashboard at: `localhost:5667`


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
