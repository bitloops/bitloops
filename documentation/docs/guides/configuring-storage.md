---
sidebar_position: 3
title: Configuring Storage
---

# Configuring Storage

Storage backends are a daemon concern and belong in the global daemon config.

## Storage Boundaries

Bitloops now separates storage by purpose:

- `RuntimeStore`: local-only SQLite used for workflow and daemon runtime state
- `RelationalStore`: the approved relational boundary for queryable checkpoint and DevQL relational data

The configured `[stores]` sections control `RelationalStore`, the event backend, and the blob backend. `RuntimeStore` paths are derived by the host.

## Default Behaviour

By default Bitloops uses platform app directories:

- relational database in the data directory
- event database in the data directory
- blob store in the data directory
- embedding model downloads in the cache directory
- daemon runtime SQLite in the state directory
- repo runtime SQLite in `.bitloops/stores/runtime/runtime.sqlite`

Linux examples:

```text
~/.config/bitloops/config.toml
~/.local/share/bitloops/stores/relational/relational.db
~/.local/share/bitloops/stores/event/events.duckdb
~/.local/share/bitloops/stores/blob/
~/.cache/bitloops/embeddings/models/
~/.local/state/bitloops/daemon/runtime.sqlite
```

Repo-scoped runtime state lives in:

```text
<repo>/.bitloops/stores/runtime/runtime.sqlite
```

## Local SQLite, DuckDB, And Blob Defaults

```toml
[stores.relational]
sqlite_path = "/Users/alex/.local/share/bitloops/stores/relational/relational.db"

[stores.events]
duckdb_path = "/Users/alex/.local/share/bitloops/stores/event/events.duckdb"

[stores.blob]
local_path = "/Users/alex/.local/share/bitloops/stores/blob"
```

## Remote Stores

```toml
[stores.relational]
postgres_dsn = "${BITLOOPS_POSTGRES_DSN}"

[stores.events]
clickhouse_url = "http://localhost:8123"
clickhouse_user = "${BITLOOPS_CLICKHOUSE_USER}"
clickhouse_password = "${BITLOOPS_CLICKHOUSE_PASSWORD}"
clickhouse_database = "bitloops"

[stores.blob]
s3_bucket = "bitloops-artifacts"
s3_region = "eu-west-1"
s3_access_key_id = "${AWS_ACCESS_KEY_ID}"
s3_secret_access_key = "${AWS_SECRET_ACCESS_KEY}"
```

## Embedding Cache

Semantic and embeddings configuration lives alongside store config in the daemon config, but it uses separate sections from `[stores]`:

```toml
[semantic]
provider = "openai_compatible"
model = "qwen2.5-coder"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1"

[semantic_clones]
summary_mode = "auto"
embedding_mode = "semantic_aware_once"
embedding_profile = "local-code"

[embeddings.runtime]
command = "bitloops-embeddings"
startup_timeout_secs = 10
request_timeout_secs = 60

[embeddings.profiles.local-code]
kind = "local_fastembed"
model = "jinaai/jina-embeddings-v2-base-code"
cache_dir = "/Users/alex/.cache/bitloops/embeddings/models"
```

Embedding model downloads are cache, not durable relational or event store data.

## Internal Runtime Stores

These SQLite files are not configured under `[stores]`:

| Runtime surface | Default path | Purpose |
| --- | --- | --- |
| Daemon runtime store | `<state dir>/daemon/runtime.sqlite` | daemon runtime state, service metadata, supervisor metadata, sync queue state, enrichment queue state |
| Repo runtime store | `<repo>/.bitloops/stores/runtime/runtime.sqlite` | sessions, temporary checkpoints, pre-prompt states, pre-task markers, interaction spool |

`RuntimeStore` is always local SQLite. `RelationalStore` is configured through `[stores.relational]` and can use SQLite or Postgres.

## Check The Effective State

```bash
bitloops status
bitloops --connection-status
```

Configured relational, event, and blob store locations come from the active daemon config. If the global config does not exist yet, create it with `bitloops start --create-default-config` or let interactive `bitloops start` create it at the default location.

If you are using an explicit daemon config for a repo-scoped or test setup, create the matching local store artefacts with:

```bash
bitloops start --config /path/to/config.toml --bootstrap-local-stores
```
