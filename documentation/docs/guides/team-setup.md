---
sidebar_position: 2
title: Team Setup
---

# Team Setup

How to configure Bitloops for a team so AI reasoning is shared through git while keeping personal settings private.

## What to Commit

Add these to your repository:

```
.bitloops/config.json          # Shared project configuration (stores, knowledge providers)
.bitloops/settings.json        # Shared runtime settings (strategy, enabled state)
.bitloops/checkpoints/         # AI reasoning linked to commits
```

## What to Gitignore

Bitloops creates a `.bitloops/.gitignore` automatically, but verify it includes:

```gitignore title=".bitloops/.gitignore"
settings.local.json
stores/
```

- **`settings.local.json`** — personal overrides (telemetry preference, local enable/disable)
- **`stores/`** — local databases (SQLite, DuckDB, blob). Each developer builds their own by running `bitloops devql ingest`

## Onboarding a New Teammate

When a new developer clones the repo:

```bash
# 1. Install Bitloops
curl -fsSL https://bitloops.com/install.sh | bash

# 2. Initialize their agent
bitloops init --agent claude-code

# 3. Enable capture
bitloops enable --local

# 4. Initialize local databases (uses shared config.json)
bitloops devql init

# 5. Ingest the codebase into their local knowledge graph
bitloops devql ingest
```

Steps 1-3 take under a minute. Step 5 depends on repo size but is typically seconds to a few minutes.

## Project vs Local Settings

### Project-level (`bitloops enable --project`)

Writes to `.bitloops/settings.json` (committed to git). The whole team inherits:

```json title=".bitloops/settings.json"
{
  "strategy": "manual_commit",
  "enabled": true
}
```

### Local-level (`bitloops enable --local`)

Writes to `.bitloops/settings.local.json` (gitignored). Only affects this developer:

```json title=".bitloops/settings.local.json"
{
  "enabled": true,
  "telemetry": false
}
```

Local settings override project settings. This lets individual developers disable capture temporarily without affecting the team.

## Shared Configuration Pattern

A typical team `config.json` with environment variables for secrets:

```json title=".bitloops/config.json"
{
  "stores": {
    "relational": { "provider": "sqlite" },
    "event": { "provider": "duckdb" },
    "blob": { "provider": "local" }
  },
  "knowledge": {
    "providers": {
      "github": {
        "token": "${GITHUB_TOKEN}"
      }
    }
  }
}
```

Each developer sets `GITHUB_TOKEN` in their own shell environment. The config is safe to commit because it contains no actual secrets.

## Shared Databases (Optional)

For teams that want everyone querying the same knowledge graph:

```json title=".bitloops/config.json"
{
  "stores": {
    "relational": {
      "provider": "postgres",
      "postgres_dsn": "${BITLOOPS_PG_DSN}"
    },
    "event": {
      "provider": "clickhouse",
      "clickhouse_url": "${BITLOOPS_CH_URL}"
    }
  }
}
```

With shared databases, only one person needs to run `bitloops devql ingest` — the results are available to everyone.

## Code Review Workflow

With Bitloops configured for the team:

1. Developer works with an AI agent → Bitloops captures the session
2. Developer commits → checkpoint is created in `.bitloops/checkpoints/`
3. Developer pushes → checkpoint data is included in the push
4. Reviewer pulls → runs `bitloops explain` to see AI reasoning behind any commit
5. Review includes both the code diff **and** the AI's decision-making process

This adds transparency to AI-assisted development without adding friction.
