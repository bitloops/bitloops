# Configuration Reference

This file documents user-facing configuration for Bitloops CLI.

Scope:

- Unified project configuration (hooks, stores, knowledge, semantic, dashboard, watch)
- Configuration layering and precedence
- Monorepo project-root discovery
- Supported runtime `BITLOOPS_*` environment variables
- Build-time dashboard URL config

Test-only env vars are intentionally excluded.

## 1) Unified configuration model

Bitloops uses **one configuration domain**. All settings — hooks/strategy, storage backends, knowledge, semantic, dashboard, and watch — live in a single schema and follow the same merge rules.

### File pair

| File | Purpose | Git visibility |
| ---- | ------- | -------------- |
| `<project_root>/.bitloops/config.json` | Shared team configuration (committed) | Tracked |
| `<project_root>/.bitloops/config.local.json` | Personal overrides (gitignored) | Ignored |
| `~/.bitloops/config.json` | Optional global defaults (lowest precedence) | N/A |

`<project_root>` is the **Bitloops project root** — see §6 for monorepo discovery rules.

### Config envelope format

Every config file uses the envelope format:

```json
{
  "version": "1.0",
  "scope": "project",
  "settings": {
    "strategy": "manual-commit",
    "enabled": true,
    "stores": { ... },
    "knowledge": { ... },
    "semantic": { ... },
    "dashboard": { ... },
    "watch": { ... }
  }
}
```

- `version` — config schema version (currently `"1.0"`)
- `scope` — must match the file's location: `"global"`, `"project"`, or `"project_local"`
- `settings` — the actual configuration keys (see §2–§4)

Scope mismatch between the declared `scope` and the file's location is an error.
Unknown keys are rejected (strict schema).

### Layer precedence (lowest → highest)

1. **Code defaults** — when a key is absent everywhere
2. **Global** — `~/.bitloops/config.json` (scope: `"global"`)
3. **Project shared** — `<project_root>/.bitloops/config.json` (scope: `"project"`)
4. **Project local** — `<project_root>/.bitloops/config.local.json` (scope: `"project_local"`)
5. **Environment variables** — highest precedence for documented keys

### Merge semantics

- **Objects:** deep-merge by key. A higher layer adds or overrides individual keys without wiping siblings.
- **Arrays:** replace (whole array wins from higher layer).
- **`null`:** clears the key from lower layers.
- **Absent keys:** inherit from the next lower layer.

Each layer only needs to contain **deltas**. For example, a local override with just `"enabled": false` inherits everything else from the project and global layers.

## 2) Hooks and strategy settings

These control Bitloops session behaviour. They live inside the `settings` block of the config envelope.

| Key | Type | Default | Notes |
| --- | ---- | ------- | ----- |
| `strategy` | string | `manual-commit` | Valid: `manual-commit`, `auto-commit`. |
| `enabled` | boolean | `true` | Enable/disable switch for Bitloops in this project. |
| `local_dev` | boolean | `false` | Uses local dev hook commands for agent hook wiring. |
| `log_level` | string | empty | Stored in config; runtime level controlled by `BITLOOPS_LOG_LEVEL`. |
| `strategy_options` | object | `{}` | Strategy-specific options. |
| `telemetry` | boolean or null | `null` | `null` means consent not captured yet. |

Known `strategy_options`:

- `summarize.enabled` (boolean)
- `push_sessions` (boolean)

## 3) Store backend configuration

Backend selection is **provider-less**: local backends (SQLite, DuckDB, local blob) are always available with defaults. Remote backends (Postgres, ClickHouse, S3, GCS) are **additive** — they activate when their connection string or bucket is present.

### Recommended store shape

```json
{
  "version": "1.0",
  "scope": "project",
  "settings": {
    "stores": {
      "relational": {
        "sqlite_path": ".bitloops/stores/relational/relational.db"
      },
      "event": {
        "duckdb_path": ".bitloops/stores/event/events.duckdb"
      },
      "blob": {
        "local_path": ".bitloops/stores/blob"
      }
    }
  }
}
```

### Store keys

| Key | Type | Default | Notes |
| --- | ---- | ------- | ----- |
| `stores.relational.sqlite_path` | string | `.bitloops/stores/relational/relational.db` | Local SQLite. Always available. Relative paths resolved from project root. `~` is expanded. |
| `stores.relational.postgres_dsn` | string | none | When present, Postgres is additionally available for shared/team data. |
| `stores.event.duckdb_path` | string | `.bitloops/stores/event/events.duckdb` | Local DuckDB. Always available. Relative paths resolved from project root. `~` is expanded. |
| `stores.event.clickhouse_url` | string | none | When present, ClickHouse is used for event storage. |
| `stores.event.clickhouse_user` | string | none | Optional ClickHouse username. |
| `stores.event.clickhouse_password` | string | none | Optional ClickHouse password. |
| `stores.event.clickhouse_database` | string | `default` | Optional ClickHouse database name. |
| `stores.blob.local_path` | string | `.bitloops/stores/blob` | Local blob store. Always available. Relative paths resolved from project root. `~` is expanded. |
| `stores.blob.s3_bucket` | string | none | When present, S3 is used for blob storage. |
| `stores.blob.s3_region` | string | none | Optional S3 region. |
| `stores.blob.s3_access_key_id` | string | none | Optional static credentials for S3. |
| `stores.blob.s3_secret_access_key` | string | none | Optional static credentials for S3. |
| `stores.blob.gcs_bucket` | string | none | When present, GCS is used for blob storage. |
| `stores.blob.gcs_credentials_path` | string | none | Optional path to GCS credentials JSON. |

### Knowledge provider keys

These live under `knowledge.providers` inside `settings`.

| Key | Type | Default | Notes |
| --- | ---- | ------- | ----- |
| `knowledge.providers.github.token` | string | none | Required for GitHub issue/PR knowledge ingestion. |
| `knowledge.providers.atlassian.site_url` | string | none | Shared default Atlassian site for Jira and Confluence. |
| `knowledge.providers.atlassian.email` | string | none | Shared default Atlassian email for basic auth. |
| `knowledge.providers.atlassian.token` | string | none | Shared default Atlassian token for basic auth. |
| `knowledge.providers.jira.site_url` | string | none | Optional Jira override. Falls back to `atlassian`. |
| `knowledge.providers.jira.email` | string | none | Optional Jira override. Falls back to `atlassian`. |
| `knowledge.providers.jira.token` | string | none | Optional Jira override. Falls back to `atlassian`. |
| `knowledge.providers.confluence.site_url` | string | none | Optional Confluence override. Falls back to `atlassian`. |
| `knowledge.providers.confluence.email` | string | none | Optional Confluence override. Falls back to `atlassian`. |
| `knowledge.providers.confluence.token` | string | none | Optional Confluence override. Falls back to `atlassian`. |

Provider values support `${ENV_VAR}` interpolation. That interpolation is limited to `knowledge.providers`.

### Semantic keys

| Key | Type | Default | Notes |
| --- | ---- | ------- | ----- |
| `semantic.provider` | string | none | Semantic provider identifier. |
| `semantic.model` | string | none | Model name used by semantic features. |
| `semantic.api_key` | string | none | Provider API key. |
| `semantic.base_url` | string | none | Optional custom API base URL. |

### Dashboard keys

Dashboard local TLS hints are nested under `dashboard.local_dashboard`.

| Key | Type | Default | Notes |
| --- | ---- | ------- | ----- |
| `dashboard.local_dashboard.tls` | boolean | unset | When `true`, dashboard fast-path assumes local TLS material is already present and valid. |

Example:

```json
{
  "version": "1.0",
  "scope": "project",
  "settings": {
    "dashboard": {
      "local_dashboard": {
        "tls": true
      }
    }
  }
}
```

CLI interaction:

- `bitloops dashboard --recheck-local-dashboard-net` forces a full local dashboard TLS recheck.
- `bitloops dashboard --http --host 127.0.0.1` explicitly forces loopback HTTP mode (no TLS).

## 4) Manual knowledge ingestion

`bitloops devql knowledge add <url>` supports:

- GitHub issue URLs
- GitHub pull request URLs
- Jira issue URLs
- Confluence page URLs

Examples:

```bash
bitloops devql knowledge add https://github.com/bitloops/bitloops/issues/42
bitloops devql knowledge add https://github.com/bitloops/bitloops/pull/137 --commit 6b7845a
bitloops devql knowledge add https://bitloops.atlassian.net/browse/CLI-1370
bitloops devql knowledge add https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548
```

Storage behavior:

- SQLite stores knowledge source, repository-scoped item, and relation metadata.
- DuckDB stores queryable document-version metadata.
- Blob storage stores the full knowledge payload content.
- With the default local backend, payloads are stored under `<project_root>/.bitloops/stores/blob/knowledge/...`.

## 5) CLI commands

### Enable / Disable

```bash
bitloops enable              # writes to config.json (or config.local.json if shared exists)
bitloops enable --local      # writes to config.local.json
bitloops enable --project    # writes to config.json
bitloops disable             # writes enabled: false to config.local.json
bitloops disable --project   # writes enabled: false to config.json
```

### Status

```bash
bitloops status              # short: "Enabled (manual-commit)" or "Disabled (...)"
bitloops status --detailed   # shows project and local layers separately
```

Status reads from `config.json` and `config.local.json`. Legacy `settings.json` files are not read.

## 6) Monorepo support

In a monorepo, one `.git` lives at the repository top level while day-to-day work happens in nested packages. Bitloops uses two independent roots:

| Concept | How resolved | Used for |
| ------- | ------------ | -------- |
| **Git root** | Walk up until `.git` found | Git hooks (`.git/hooks`), worktree identity |
| **Bitloops project root** | Walk up until `.bitloops/` found; fall back to git root | Config, stores, agent directories, metadata |

### Discovery algorithm

1. Start at the current working directory.
2. Walk upward. The **first** ancestor containing a `.bitloops/` directory is the **Bitloops project root**.
3. If no `.bitloops/` is found, fall back to the **git root**.

### Example layout

```
monorepo/                          ← git root (.git here)
  packages/
    web-app/                       ← Bitloops project root for this app
      .bitloops/                   ← config, stores, metadata
      .claude/                     ← agent hooks (peer of .bitloops)
      .cursor/
      src/
    api/                           ← separate Bitloops project root
      .bitloops/
      .claude/
      src/
```

### Key rules

- **Config paths** are anchored at the Bitloops project root, not git root.
- **Git hooks** install at git root (one set per repository). Hook scripts resolve the Bitloops project root from cwd at runtime.
- **Agent directories** (`.claude/`, `.cursor/`, `.codex/`, `.gemini/`) live beside `.bitloops/` at the Bitloops project root.
- **`.bitloops/.gitignore`** handles ignore rules for `config.local.json`, stores, logs, etc.

## 7) Runtime environment variables

### Semantic env vars

| Variable | Purpose |
| -------- | ------- |
| `BITLOOPS_DEVQL_SEMANTIC_PROVIDER` | Overrides `semantic.provider`. |
| `BITLOOPS_DEVQL_SEMANTIC_MODEL` | Overrides `semantic.model`. |
| `BITLOOPS_DEVQL_SEMANTIC_API_KEY` | Overrides `semantic.api_key`. |
| `BITLOOPS_DEVQL_SEMANTIC_BASE_URL` | Overrides `semantic.base_url`. |

### Dashboard runtime env vars

| Variable | Purpose |
| -------- | ------- |
| `BITLOOPS_DASHBOARD_MANIFEST_URL` | Explicit manifest URL; highest precedence. |
| `BITLOOPS_DASHBOARD_CDN_BASE_URL` | Base URL used to derive manifest and relative bundle asset URLs. |
| `BITLOOPS_DEV` | Enables extra dashboard startup output for development. |

Manifest resolution order:

1. `BITLOOPS_DASHBOARD_MANIFEST_URL`
2. `BITLOOPS_DASHBOARD_CDN_BASE_URL` + `/bundle_versions.json`
3. Compiled `dashboard_manifest_url`
4. Compiled `dashboard_cdn_base_url` + `/bundle_versions.json`

### Logging env var

| Variable | Purpose |
| -------- | ------- |
| `BITLOOPS_LOG_LEVEL` | Logger level (`DEBUG`, `INFO`, `WARN` or `WARNING`, `ERROR`). Invalid values fall back to `INFO`. |

### Telemetry env vars

| Variable | Purpose |
| -------- | ------- |
| `BITLOOPS_TELEMETRY_OPTOUT` | Any non-empty value disables telemetry dispatch. |
| `BITLOOPS_POSTHOG_API_KEY` | Overrides telemetry API key. |
| `BITLOOPS_POSTHOG_ENDPOINT` | Overrides telemetry endpoint (default: `https://eu.i.posthog.com`). |
| `BITLOOPS_TELEMETRY_DISTINCT_ID` | Explicit distinct ID override. |
| `BITLOOPS_TELEMETRY_FORCE_NO_DISTINCT_ID` | Any non-empty value disables distinct ID generation. |

## 8) Build-time dashboard URL config (`bitloops/config/dashboard_urls.json`)

Used at build time by `bitloops/build.rs` to compile dashboard bundle URL defaults into the binary.

Expected keys:

| Key | Required | Rules |
| --- | -------- | ----- |
| `dashboard_cdn_base_url` | yes | Non-empty, valid `http://` or `https://` URL |
| `dashboard_manifest_url` | yes | Non-empty, valid `http://` or `https://` URL |

Template file:

- `bitloops/config/dashboard_urls.template.json`

If this file is missing or invalid, `cargo check` or `cargo build` in `bitloops/` fails fast with guidance.

## 9) Optional build metadata env vars

These are consumed by `bitloops/build.rs` and embedded into CLI version output:

- `BITLOOPS_BUILD_VERSION`
- `BITLOOPS_BUILD_COMMIT`
- `BITLOOPS_BUILD_TARGET`
- `BITLOOPS_BUILD_DATE`
