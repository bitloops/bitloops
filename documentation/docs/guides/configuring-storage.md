---
sidebar_position: 3
title: Configuring Storage
---

# Configuring Storage

Storage backends are a daemon concern and belong in the global daemon config.

## Default Behaviour

By default Bitloops uses platform app directories:

- relational database in the data directory
- event database in the data directory
- blob store in the data directory
- embedding model downloads in the cache directory

Linux examples:

```text
~/.config/bitloops/config.toml
~/.local/share/bitloops/stores/relational/relational.db
~/.local/share/bitloops/stores/event/events.duckdb
~/.local/share/bitloops/stores/blob/
~/.cache/bitloops/embeddings/models/
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

## Check The Effective State

```bash
bitloops status
bitloops --connection-status
```

Store locations come only from the active daemon config. If the global config does not exist yet, create it with `bitloops start --create-default-config` or let interactive `bitloops start` create it at the default location.
