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

### 1. Initialise the daemon config

```bash
bitloops init
```

### 2. Configure machine-specific stores and credentials

```toml title="config.toml"
[stores.relational]
sqlite_path = "/Users/alex/.local/share/bitloops/stores/relational/relational.db"

[stores.events]
duckdb_path = "/Users/alex/.local/share/bitloops/stores/event/events.duckdb"

[knowledge.providers.github]
token = "${GITHUB_TOKEN}"
```

### 3. Commit shared repo policy

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

### 4. Enable the repo locally

```bash
bitloops enable
```

### 5. Open the dashboard or start the daemon

```bash
bitloops dashboard
```

## Local Overrides

Personal overrides go in `.bitloops.local.toml`, which `bitloops enable` ensures is ignored through `.git/info/exclude`.

Example:

```toml title=".bitloops.local.toml"
[capture]
enabled = false
```

## What Not To Commit

Do not commit:

- provider secrets
- machine-specific store paths unless your team explicitly standardises them
- daemon runtime state

Those belong to each developer’s daemon config or local environment.
