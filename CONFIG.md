# Configuration Reference

This file documents user-facing configuration for Bitloops CLI.

Scope:

- Repository runtime config
- Repository settings
- Supported runtime `BITLOOPS_*` environment variables
- Build-time dashboard URL config

Test-only env vars are intentionally excluded.

## 1) Repository runtime config (`<repo>/.bitloops/config.json`)

Primary use:

- Storage backend/provider selection
- Knowledge provider credentials
- Semantic feature provider settings
- Dashboard host preference

Store configuration precedence:

1. Explicit values in `<repo>/.bitloops/config.json`
2. Built-in defaults under `<repo>/.bitloops/stores`

Important:

- This shape is **not** backwards-compatible with legacy `devql.*` keys.
- Global user config at `~/.bitloops/config.json` is not used for store backends.

### Recommended config shape

```json
{
  "stores": {
    "relational": {
      "provider": "sqlite",
      "sqlite_path": ".bitloops/stores/relational/relational.db"
    },
    "event": {
      "provider": "duckdb",
      "duckdb_path": ".bitloops/stores/event/events.duckdb"
    },
    "blob": {
      "provider": "local",
      "local_path": ".bitloops/stores/blob"
    }
  },
  "knowledge": {
    "providers": {
      "github": {
        "token": "${GITHUB_TOKEN}"
      },
      "jira": {
        "site_url": "https://bitloops.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_TOKEN}"
      },
      "confluence": {
        "site_url": "https://bitloops.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_TOKEN}"
      }
    }
  },
  "semantic": {
    "provider": "openai",
    "model": "gpt-4.1-mini",
    "api_key": "YOUR_KEY"
  },
  "dashboard": {
    "use_bitloops_local": false
  }
}
```

### Store keys

| Key                                | Type                     | Default                                            | Notes                                                                                        |
| ---------------------------------- | ------------------------ | -------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `stores.relational.provider`       | `sqlite` \| `postgres`   | `sqlite`                                           | Selects relational backend.                                                                  |
| `stores.relational.sqlite_path`    | string                   | `<repo>/.bitloops/stores/relational/relational.db` | Used when provider is `sqlite`. Relative paths are resolved from repo root. `~` is expanded. |
| `stores.relational.postgres_dsn`   | string                   | none                                               | Required when provider is `postgres`.                                                        |
| `stores.event.provider`            | `duckdb` \| `clickhouse` | `duckdb`                                           | Selects event backend.                                                                       |
| `stores.event.duckdb_path`         | string                   | `<repo>/.bitloops/stores/event/events.duckdb`      | Used when provider is `duckdb`. Relative paths are resolved from repo root. `~` is expanded. |
| `stores.event.clickhouse_url`      | string                   | `http://localhost:8123`                            | Used when provider is `clickhouse`.                                                          |
| `stores.event.clickhouse_user`     | string                   | none                                               | Optional ClickHouse username.                                                                |
| `stores.event.clickhouse_password` | string                   | none                                               | Optional ClickHouse password.                                                                |
| `stores.event.clickhouse_database` | string                   | `default`                                          | Optional ClickHouse database.                                                                |
| `stores.blob.provider`             | `local` \| `s3` \| `gcs` | `local`                                            | Selects blob storage backend.                                                                |
| `stores.blob.local_path`           | string                   | `<repo>/.bitloops/stores/blob`                     | Used when provider is `local`. Relative paths are resolved from repo root. `~` is expanded.  |
| `stores.blob.s3_bucket`            | string                   | none                                               | Required when provider is `s3`.                                                              |
| `stores.blob.s3_region`            | string                   | none                                               | Optional when provider is `s3`.                                                              |
| `stores.blob.s3_access_key_id`     | string                   | none                                               | Optional static credentials for S3.                                                          |
| `stores.blob.s3_secret_access_key` | string                   | none                                               | Optional static credentials for S3.                                                          |
| `stores.blob.gcs_bucket`           | string                   | none                                               | Required when provider is `gcs`.                                                             |
| `stores.blob.gcs_credentials_path` | string                   | none                                               | Optional path to GCS credentials JSON.                                                       |

### Knowledge provider keys

These live under `knowledge.providers`.

| Key                             | Type   | Default | Notes                                                                                       |
| ------------------------------- | ------ | ------- | ------------------------------------------------------------------------------------------- |
| `knowledge.providers.github.token`        | string | none    | Required for GitHub issue/PR knowledge ingestion.                                           |
| `knowledge.providers.jira.site_url`       | string | none    | Required for Jira issue knowledge ingestion. Must match the Atlassian site in the URL.      |
| `knowledge.providers.jira.email`          | string | none    | Required for Jira basic auth.                                                               |
| `knowledge.providers.jira.token`          | string | none    | Required for Jira basic auth.                                                               |
| `knowledge.providers.confluence.site_url` | string | none    | Required for Confluence page knowledge ingestion. Must match the Atlassian site in the URL. |
| `knowledge.providers.confluence.email`    | string | none    | Required for Confluence basic auth.                                                         |
| `knowledge.providers.confluence.token`    | string | none    | Required for Confluence basic auth.                                                         |

Provider values support `${ENV_VAR}` interpolation in `<repo>/.bitloops/config.json`. That interpolation is intentionally limited to `knowledge.providers`.

### Semantic keys

| Key                 | Type   | Default | Notes                                 |
| ------------------- | ------ | ------- | ------------------------------------- |
| `semantic.provider` | string | none    | Semantic provider identifier.         |
| `semantic.model`    | string | none    | Model name used by semantic features. |
| `semantic.api_key`  | string | none    | Provider API key.                     |
| `semantic.base_url` | string | none    | Optional custom API base URL.         |

### Manual knowledge ingestion

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
- With the default local backend, payloads are stored under `<repo>/.bitloops/stores/blob/knowledge/...`.

### Dashboard key

| Key                            | Type    | Default | Notes                                                                                 |
| ------------------------------ | ------- | ------- | ------------------------------------------------------------------------------------- |
| `dashboard.use_bitloops_local` | boolean | `false` | When `true`, dashboard defaults to host `bitloops.local` unless `--host` is provided. |

## 2) Repository settings (`.bitloops/settings*.json`)

Files:

- Project settings: `<repo>/.bitloops/settings.json`
- Local override: `<repo>/.bitloops/settings.local.json`

Merge behaviour:

1. Load `settings.json` or defaults if missing
2. Overlay `settings.local.json` if present
3. Apply defaults for missing or empty values

Unknown keys are rejected in these files.

### Settings keys

| Key                | Type            | Default         | Notes                                                                                   |
| ------------------ | --------------- | --------------- | --------------------------------------------------------------------------------------- |
| `strategy`         | string          | `manual-commit` | Valid built-ins: `manual-commit`, `auto-commit`.                                        |
| `enabled`          | boolean         | `true`          | Global enable/disable switch for Bitloops in the repo.                                  |
| `local_dev`        | boolean         | `false`         | Uses local dev hook commands for agent hook wiring.                                     |
| `log_level`        | string          | empty           | Stored in settings; runtime logger level is controlled by env var `BITLOOPS_LOG_LEVEL`. |
| `strategy_options` | object          | `{}`            | Strategy-specific options map.                                                          |
| `telemetry`        | boolean or null | `null`          | `null` means consent not captured yet.                                                  |

Known `strategy_options` in code:

- `summarize.enabled` (boolean)
- `push_sessions` (boolean)

## 3) Runtime environment variables

### Semantic env vars

| Variable                           | Purpose                        |
| ---------------------------------- | ------------------------------ |
| `BITLOOPS_DEVQL_SEMANTIC_PROVIDER` | Overrides `semantic.provider`. |
| `BITLOOPS_DEVQL_SEMANTIC_MODEL`    | Overrides `semantic.model`.    |
| `BITLOOPS_DEVQL_SEMANTIC_API_KEY`  | Overrides `semantic.api_key`.  |
| `BITLOOPS_DEVQL_SEMANTIC_BASE_URL` | Overrides `semantic.base_url`. |

### Dashboard runtime env vars

| Variable                          | Purpose                                                          |
| --------------------------------- | ---------------------------------------------------------------- |
| `BITLOOPS_DASHBOARD_MANIFEST_URL` | Explicit manifest URL; highest precedence.                       |
| `BITLOOPS_DASHBOARD_CDN_BASE_URL` | Base URL used to derive manifest and relative bundle asset URLs. |
| `BITLOOPS_DEV`                    | Enables extra dashboard startup output for development.          |

Manifest resolution order:

1. `BITLOOPS_DASHBOARD_MANIFEST_URL`
2. `BITLOOPS_DASHBOARD_CDN_BASE_URL` + `/bundle_versions.json`
3. Compiled `dashboard_manifest_url`
4. Compiled `dashboard_cdn_base_url` + `/bundle_versions.json`

### Logging env var

| Variable             | Purpose                                                                                           |
| -------------------- | ------------------------------------------------------------------------------------------------- |
| `BITLOOPS_LOG_LEVEL` | Logger level (`DEBUG`, `INFO`, `WARN` or `WARNING`, `ERROR`). Invalid values fall back to `INFO`. |

### Telemetry env vars

| Variable                                  | Purpose                                                             |
| ----------------------------------------- | ------------------------------------------------------------------- |
| `BITLOOPS_TELEMETRY_OPTOUT`               | Any non-empty value disables telemetry dispatch.                    |
| `BITLOOPS_POSTHOG_API_KEY`                | Overrides telemetry API key.                                        |
| `BITLOOPS_POSTHOG_ENDPOINT`               | Overrides telemetry endpoint (default: `https://eu.i.posthog.com`). |
| `BITLOOPS_TELEMETRY_DISTINCT_ID`          | Explicit distinct ID override.                                      |
| `BITLOOPS_TELEMETRY_FORCE_NO_DISTINCT_ID` | Any non-empty value disables distinct ID generation.                |

## 4) Build-time dashboard URL config (`bitloops_cli/config/dashboard_urls.json`)

Used at build time by `bitloops_cli/build.rs` to compile dashboard bundle URL defaults into the binary.

Expected keys:

| Key                      | Required | Rules                                        |
| ------------------------ | -------- | -------------------------------------------- |
| `dashboard_cdn_base_url` | yes      | Non-empty, valid `http://` or `https://` URL |
| `dashboard_manifest_url` | yes      | Non-empty, valid `http://` or `https://` URL |

Template file:

- `bitloops_cli/config/dashboard_urls.template.json`

If this file is missing or invalid, `cargo check` or `cargo build` in `bitloops_cli/` fails fast with guidance.

## 5) Optional build metadata env vars

These are consumed by `bitloops_cli/build.rs` and embedded into CLI version output:

- `BITLOOPS_BUILD_VERSION`
- `BITLOOPS_BUILD_COMMIT`
- `BITLOOPS_BUILD_TARGET`
- `BITLOOPS_BUILD_DATE`
