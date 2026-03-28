---
sidebar_position: 2
title: Configuration
---

# Configuration Reference

Bitloops uses two configuration files in the `.bitloops/` directory of your project.

## `config.json` — Project Configuration

The main configuration file for stores, knowledge providers, and project-level settings. This file can be committed to git to share settings across the team.

### Full Schema

```json
{
  "version": "1.0",
  "scope": "project",
  "settings": {
    "stores": {
      "relational": {
        "provider": "sqlite | postgres",
        "sqlite_path": ".bitloops/stores/relational/relational.db",
        "postgres_dsn": "postgres://user:pass@host:5432/db"
      },
      "event": {
        "provider": "duckdb | clickhouse",
        "duckdb_path": ".bitloops/stores/event/events.duckdb",
        "clickhouse_url": "http://localhost:8123"
      },
      "blob": {
        "provider": "local | s3 | gcs",
        "local_path": ".bitloops/stores/blob",
        "s3_bucket": "bucket-name",
        "s3_region": "us-east-1",
        "gcs_bucket": "bucket-name"
      }
    },
    "knowledge": {
      "providers": {
        "github": {
          "token": "${GITHUB_TOKEN}"
        },
        "jira": {
          "site_url": "https://org.atlassian.net",
          "email": "${ATLASSIAN_EMAIL}",
          "token": "${ATLASSIAN_TOKEN}"
        },
        "confluence": {
          "site_url": "https://org.atlassian.net",
          "email": "${ATLASSIAN_EMAIL}",
          "token": "${ATLASSIAN_TOKEN}"
        }
      }
    },
    "semantic": {
      "provider": "openai",
      "model": "gpt-4.1-mini",
      "api_key": "${OPENAI_API_KEY}"
    },
    "dashboard": {
      "local_dashboard": {
        "tls": true
      }
    }
  }
}
```

### Dashboard Local TLS Hints

`dashboard.local_dashboard` stores local HTTPS hints for daemon startup and the dashboard launcher.

| Field                                      | Type    | Default | Description                                                                                  |
| ------------------------------------------ | ------- | ------- | -------------------------------------------------------------------------------------------- |
| `dashboard.local_dashboard.tls`            | boolean | unset   | When `true`, dashboard fast-path assumes local TLS material is already available.            |

Notes:

- These are local TLS hints for `bitloops daemon start` and `bitloops dashboard`.
- To force a full recheck of local dashboard TLS setup, run `bitloops daemon start --recheck-local-dashboard-net`.
- To force loopback HTTP without TLS, run `bitloops daemon start --http --host 127.0.0.1`.

### Environment Variable Interpolation

Use `${VAR_NAME}` syntax to reference environment variables. This keeps secrets out of committed config files.

```json
{
  "knowledge": {
    "providers": {
      "github": {
        "token": "${GITHUB_TOKEN}"
      }
    }
  }
}
```

Bitloops resolves these at runtime from your shell environment.

## `settings.json` — Project Settings

Controls runtime behavior. Can be committed to git for shared settings.

```json
{
  "strategy": "manual_commit",
  "enabled": true,
  "telemetry": true
}
```

| Field       | Values          | Description                                           |
| ----------- | --------------- | ----------------------------------------------------- |
| `strategy`  | `manual_commit` | When checkpoints are created (default: on git commit) |
| `enabled`   | `true \| false` | Whether capture is active                             |
| `telemetry` | `true \| false` | Whether anonymous telemetry is sent                   |

## `settings.local.json` — Local Settings

Personal settings that override `settings.json`. This file is gitignored and never shared.

Use this for:

- Enabling/disabling capture without affecting teammates
- Personal telemetry preferences
- Local-only configuration overrides

Created when you run `bitloops enable --local`.

## Configuration Precedence

1. `settings.local.json` (highest priority, personal)
2. `settings.json` (project-level, shared)
3. Built-in defaults (lowest priority)

## Defaults

If no `config.json` exists, Bitloops uses these defaults:

| Store      | Default Provider | Default Path                                |
| ---------- | ---------------- | ------------------------------------------- |
| Relational | SQLite           | `.bitloops/stores/relational/relational.db` |
| Event      | DuckDB           | `.bitloops/stores/event/events.duckdb`      |
| Blob       | Local            | `.bitloops/stores/blob/`                    |
