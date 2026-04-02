---
sidebar_position: 1
title: Quickstart
---

# Quickstart

This quickstart assumes you want the new daemon-first Bitloops setup.

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

## 2. Start The Daemon

```bash
bitloops start --create-default-config
```

On a fresh machine, use `--create-default-config` once. This writes the default global daemon config at the platform config location and creates the default local SQLite, DuckDB, and blob-store paths.

Interactive `bitloops start` also prompts to create the default config when it is missing. During that first bootstrap, Bitloops asks for telemetry consent unless you pass `--telemetry`, `--telemetry=false`, or `--no-telemetry`.

## 3. Initialise A Project

From inside a git repository or subproject:

```bash
bitloops init
bitloops init --install-default-daemon
```

Use plain `bitloops init` when the daemon is already running. Use `bitloops init --install-default-daemon` when you want init to bootstrap the default daemon service first.

This creates `.bitloops.local.toml` in the current directory, adds it to `.git/info/exclude`, installs hooks, and runs the initial baseline sync through the daemon.

If you want to pin the supported agent set during bootstrap, pass `--agent <name>`.

If telemetry consent is unresolved for an existing daemon config, interactive `bitloops init` can ask again. Non-interactive runs require an explicit telemetry flag.

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
bitloops devql ingest
bitloops devql query "files changed last 7 days"
```

## 7. Sync Current State

When you want to reconcile `artefacts_current`/`artefact_edges_current` with the current workspace:

```bash
bitloops devql sync
```

When you want to validate that current-state rows match a full-project reconciliation without writing changes:

```bash
bitloops devql sync --validate
```

Use `--validate` as a diagnostic check when debugging drift between source files and current-state query results.

## 8. Check Status

```bash
bitloops status
bitloops checkpoints status --detailed
```

`bitloops status` reports daemon status. `bitloops checkpoints status` reports repo capture status and shows the resolved policy root and fingerprint.

## Toggle Capture Later

```bash
bitloops disable
bitloops enable
```

These commands edit the nearest discovered project policy and leave installed hooks in place. If telemetry consent is unresolved for an existing daemon config, interactive `bitloops enable` can ask again before it edits project policy.

## Remove Bitloops Later

Use `bitloops disable` when you want hooks and watchers to stay installed but stop capturing.

Use `bitloops uninstall` when you want to remove Bitloops-managed machine artefacts as well:

```bash
bitloops uninstall --full
```
