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

### 1. Fastest onboarding path

For scripted onboarding, run the installer with its default-config flag, then initialise the repository:

```bash
curl -fsSL https://bitloops.com/install.sh | bash -s -- --default-config
bitloops init --sync=true
```

On Windows, use the same installer intent with PowerShell `-DefaultConfig` or CMD `--default-config`. This configures the daemon with the generated default config, creates `.bitloops.local.toml`, installs hooks, and follows the initial current-state sync.

The manual browser-based alternative is:

```bash
bitloops configure --web
bitloops init --sync=true
```

Use the lower-level `bitloops start --create-default-config` path below only when someone needs to start the daemon without opening configuration or running the installer configure step.

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
```

The fastest scripted path is the installer default-config flow, then `bitloops init --sync=true`. The manual alternative is `bitloops configure --web`, then `bitloops init --sync=true`.

This creates `.bitloops.local.toml`, adds it to `.git/info/exclude`, and installs or reconciles hooks.

Daemon-level inference, telemetry, and capability-pack settings belong to `bitloops configure --web`.

`bitloops init` can also queue an initial DevQL current-state sync after hooks are installed. Use `--sync=true` to run it immediately, or `--sync=false` to skip it. If you omit `--sync` in an interactive terminal, Bitloops asks after hook setup whether you want to sync the codebase.

In non-interactive mode, `bitloops init` requires `--sync=true` or `--sync=false`.

`bitloops init` does not run DevQL ingest unless you opt in. Use `--ingest=true` during init, or run `bitloops devql tasks enqueue --kind ingest` later, when you want to populate checkpoint, commit, and event history.

Use `--agent <name>` repeatedly when a team wants to pin the supported agent set during bootstrap. For example:

```bash
bitloops init --sync=false --agent claude-code --agent codex
```

If telemetry consent later becomes unresolved for an existing daemon config, resolve it with `bitloops configure --web` or by installing a complete daemon config with `bitloops configure --file <path>`.

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

Use `bitloops enable --capture` and `bitloops disable --capture` to toggle `[capture].enabled` in the nearest discovered project policy without reinstalling hooks. Use `bitloops enable --devql-guidance` and `bitloops disable --devql-guidance` when you want to manage the repo-local DevQL guidance surface separately.

Use `bitloops enable --install-embeddings` or `bitloops daemon enable --install-embeddings` when a developer also needs the default local embeddings profile added to the effective daemon config. Interactive `bitloops enable` offers that setup automatically with a default-yes `[Y/n]` prompt when embeddings are not already configured.

If telemetry consent is unresolved for an existing daemon config, interactive `bitloops enable` can ask before it edits project policy.

## What Not To Commit

Do not commit:

- provider secrets
- machine-specific store paths unless your team explicitly standardises them
- daemon runtime state

Those belong to each developer’s daemon config or local environment.
