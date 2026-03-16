# Configuration Reference

This file documents the user-facing configuration values used by Bitloops CLI.

Scope:
- Runtime config files
- Repository settings
- Supported `BITLOOPS_*` environment variables
- Build-time dashboard URL config

Test-only env vars are intentionally excluded.

## 1) Global user config (`~/.bitloops/config.json`)

Primary use:
- DevQL backend/provider selection
- Dashboard host preference

DevQL precedence:
1. Environment variables
2. `~/.bitloops/config.json`
3. Built-in defaults

### Recommended DevQL shape

```json
{
  "devql": {
    "relational": {
      "provider": "sqlite",
      "sqlite_path": "~/.bitloops/devql/relational.db"
    },
    "events": {
      "provider": "duckdb",
      "duckdb_path": "~/.bitloops/devql/events.duckdb"
    }
  },
  "dashboard": {
    "use_bitloops_local": false
  }
}
```

### DevQL keys

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `devql.relational.provider` | `sqlite` \| `postgres` | `sqlite` | If omitted and Postgres DSN is present, provider is inferred as `postgres`. |
| `devql.relational.sqlite_path` | string | `~/.bitloops/devql/relational.db` | `~` is expanded to the user home directory. |
| `devql.relational.postgres_dsn` | string | none | Required when using `postgres` relational provider. |
| `devql.events.provider` | `duckdb` \| `clickhouse` | `duckdb` | If omitted and any ClickHouse key is present, provider is inferred as `clickhouse`. |
| `devql.events.duckdb_path` | string | `~/.bitloops/devql/events.duckdb` | `~` is expanded to the user home directory. |
| `devql.events.clickhouse_url` | string | none | For legacy `devql` command paths, fallback is `http://localhost:8123` if unset. |
| `devql.events.clickhouse_user` | string | none | Optional. |
| `devql.events.clickhouse_password` | string | none | Optional. |
| `devql.events.clickhouse_database` | string | none | For legacy `devql` command paths, fallback is `default` if unset. |

### Legacy/alias keys still accepted

The parser still accepts these aliases for backwards compatibility:
- `postgres_dsn` or `pg_dsn`
- `clickhouse_url` or `ch_url`
- `clickhouse_user` or `ch_user`
- `clickhouse_password` or `ch_password`
- `clickhouse_database` or `ch_database`
- `relational_provider`
- `events_provider`
- `sqlite_path`
- `duckdb_path`

These can appear under `devql`, and top-level fallback keys are also supported if `devql` is not present.

### Dashboard key

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `dashboard.use_bitloops_local` | boolean | `false` | When `true`, dashboard defaults to host `bitloops.local` (unless `--host` is provided). |

## 2) Repository settings (`.bitloops/settings*.json`)

Files:
- Project settings: `<repo>/.bitloops/settings.json`
- Local override: `<repo>/.bitloops/settings.local.json`

Merge behaviour:
1. Load `settings.json` (or defaults if missing)
2. Overlay `settings.local.json` (if present)
3. Apply defaults for missing/empty values

Unknown keys are rejected in these files.

### Settings keys

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `strategy` | string | `manual-commit` | Valid built-ins: `manual-commit`, `auto-commit`. |
| `enabled` | boolean | `true` | Global enable/disable switch for Bitloops in the repo. |
| `local_dev` | boolean | `false` | Uses local dev hook commands (`cargo run ...`) for agent hook wiring. |
| `log_level` | string | empty | Stored in settings; runtime logger level is controlled by env var `BITLOOPS_LOG_LEVEL`. |
| `strategy_options` | object | `{}` | Strategy-specific options map. |
| `telemetry` | boolean or null | `null` | `null` means consent not captured yet. |

Known `strategy_options` in code:
- `summarize.enabled` (boolean): enables auto-summary generation for manual-commit checkpoints.
- `push_sessions` (boolean): only explicit `false` is currently interpreted as disabled by settings helpers.

## 3) Runtime environment variables

### DevQL override env vars

| Variable | Purpose |
| --- | --- |
| `BITLOOPS_DEVQL_RELATIONAL_PROVIDER` | Overrides relational provider (`sqlite`/`postgres`). |
| `BITLOOPS_DEVQL_EVENTS_PROVIDER` | Overrides events provider (`duckdb`/`clickhouse`). |
| `BITLOOPS_DEVQL_SQLITE_PATH` | Overrides SQLite DB path. |
| `BITLOOPS_DEVQL_DUCKDB_PATH` | Overrides DuckDB DB path. |
| `BITLOOPS_DEVQL_PG_DSN` | Postgres DSN. |
| `BITLOOPS_DEVQL_CH_URL` | ClickHouse URL. |
| `BITLOOPS_DEVQL_CH_USER` | ClickHouse username. |
| `BITLOOPS_DEVQL_CH_PASSWORD` | ClickHouse password. |
| `BITLOOPS_DEVQL_CH_DATABASE` | ClickHouse database name. |

### Dashboard runtime env vars

| Variable | Purpose |
| --- | --- |
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
| --- | --- |
| `BITLOOPS_LOG_LEVEL` | Logger level (`DEBUG`, `INFO`, `WARN`/`WARNING`, `ERROR`). Invalid values fall back to `INFO`. |

### Telemetry env vars

| Variable | Purpose |
| --- | --- |
| `BITLOOPS_TELEMETRY_OPTOUT` | Any non-empty value disables telemetry dispatch. |
| `BITLOOPS_POSTHOG_API_KEY` | Overrides telemetry API key. |
| `BITLOOPS_POSTHOG_ENDPOINT` | Overrides telemetry endpoint (default: `https://eu.i.posthog.com`). |
| `BITLOOPS_TELEMETRY_DISTINCT_ID` | Explicit distinct ID override. |
| `BITLOOPS_TELEMETRY_FORCE_NO_DISTINCT_ID` | Any non-empty value disables distinct ID generation. |

## 4) Build-time dashboard URL config (`bitloops_cli/config/dashboard_urls.json`)

Used at build time by `bitloops_cli/build.rs` to compile dashboard bundle URL defaults into the binary.

Expected keys:

| Key | Required | Rules |
| --- | --- | --- |
| `dashboard_cdn_base_url` | yes | Non-empty, valid `http://` or `https://` URL |
| `dashboard_manifest_url` | yes | Non-empty, valid `http://` or `https://` URL |

Template file:
- `bitloops_cli/config/dashboard_urls.template.json`

If this file is missing or invalid, `cargo check`/`cargo build` in `bitloops_cli/` fails fast with guidance.

## 5) Optional build metadata env vars

These are consumed by `bitloops_cli/build.rs` and embedded into CLI version output:
- `BITLOOPS_BUILD_VERSION`
- `BITLOOPS_BUILD_COMMIT`
- `BITLOOPS_BUILD_TARGET`
- `BITLOOPS_BUILD_DATE`
