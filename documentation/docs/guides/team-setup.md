---
sidebar_position: 6
title: Team Setup
---

# Team Setup

The new Bitloops model separates team-shared repo policy from machine-specific daemon configuration.

## Recommended Split

Each developer has:

- their own global daemon config
- their own daemon data, cache, and state directories
- their own provider credentials

The repository can carry:

- `.bitloops.toml` for shared capture policy
- optional imported knowledge TOML files

Each developer may also keep:

- `.bitloops.local.toml` for personal overrides

## Team Onboarding Flow

### 1. Start the daemon once on each machine

```bash
bitloops start --create-default-config
```

On a fresh machine, use `--create-default-config` once. That writes the default global daemon config and creates the default local SQLite, DuckDB, and blob-store paths.

Interactive `bitloops start` can also prompt to create the default config when it is missing. During that first bootstrap, Bitloops asks for telemetry consent unless you pass `--telemetry`, `--telemetry=false`, or `--no-telemetry`.

### 2. Configure machine-specific stores and credentials

```toml title="config.toml"
[stores.relational]
sqlite_path = "/Users/alex/.local/share/bitloops/stores/relational/relational.db"

[stores.events]
duckdb_path = "/Users/alex/.local/share/bitloops/stores/event/events.duckdb"

[knowledge.providers.github]
token = "${GITHUB_TOKEN}"
```

### 3. Bootstrap a project locally

From the repository root or a subproject directory:

```bash
bitloops init --sync=true
bitloops init --install-default-daemon --sync=true
```

Use plain `bitloops init` when the daemon is already running. Use `bitloops init --install-default-daemon` when you want init to bootstrap the default daemon service before continuing.

This creates `.bitloops.local.toml`, adds it to `.git/info/exclude`, and installs or reconciles hooks.

`bitloops init` can also queue an initial DevQL current-state sync after hooks are installed. Use `--sync=true` to run it immediately, or `--sync=false` to skip it. If you omit `--sync` in an interactive terminal, Bitloops asks after hook setup whether you want to sync the codebase.

In non-interactive mode, `bitloops init` requires `--sync=true` or `--sync=false`.

`bitloops init` still does not run DevQL ingest. Use `bitloops devql ingest` when you want to populate checkpoint, commit, and event history.

Use `--agent <name>` when a team wants to pin the supported agent set during bootstrap.

If telemetry consent later becomes unresolved for an existing daemon config, interactive `bitloops init` can ask again. Non-interactive runs require an explicit telemetry flag.

### 4. Commit shared project policy when you need it

```toml title=".bitloops.toml"
[capture]
enabled = true
strategy = "manual-commit"

[watch]
watch_debounce_ms = 750
watch_poll_fallback_ms = 2500

[imports]
knowledge = ["bitloops/knowledge.toml"]
```

One simple workflow is to start from the generated `.bitloops.local.toml`, rename or copy the relevant sections into `.bitloops.toml`, and commit the shared file.

### 5. Open the dashboard or keep using the daemon

```bash
bitloops dashboard
```

## Local Overrides

Personal overrides go in `.bitloops.local.toml`, which `bitloops init` ensures is ignored through `.git/info/exclude`.

Example:

```toml title=".bitloops.local.toml"
[capture]
enabled = false
```

Use `bitloops enable` and `bitloops disable` to toggle `[capture].enabled` in the nearest discovered project policy without reinstalling hooks. If telemetry consent is unresolved for an existing daemon config, interactive `bitloops enable` can ask before it edits project policy.

## What Not To Commit

Do not commit:

- provider secrets
- machine-specific store paths unless your team explicitly standardises them
- daemon runtime state

Those belong to each developer’s daemon config or local environment.
