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
bitloops start
```

The first start creates the default global daemon config if needed.

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
bitloops init
```

This creates `.bitloops.local.toml`, adds it to `.git/info/exclude`, installs hooks, and runs the initial baseline sync.

Use `--agent <name>` when a team wants to pin the supported agent set during bootstrap.

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

Use `bitloops enable` and `bitloops disable` to toggle `[capture].enabled` in the nearest discovered project policy without reinstalling hooks.

## What Not To Commit

Do not commit:

- provider secrets
- machine-specific store paths unless your team explicitly standardises them
- daemon runtime state

Those belong to each developer’s daemon config or local environment.
