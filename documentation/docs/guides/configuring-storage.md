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
- repo runtime SQLite under the active daemon config root

Linux examples:

```text
~/.config/bitloops/config.toml
~/.local/share/bitloops/stores/relational/relational.db
~/.local/share/bitloops/stores/event/events.duckdb
~/.local/share/bitloops/stores/blob/
~/.cache/bitloops-local-embeddings/
~/.local/state/bitloops/daemon/runtime.sqlite
```

Repo-scoped runtime state lives in a config-root runtime database, for example:

```text
<config root>/stores/runtime/runtime.sqlite
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

Inference configuration lives alongside store config in the daemon config, but it uses separate sections from `[stores]`:

```toml
[semantic_clones]
summary_mode = "auto"
embedding_mode = "semantic_aware_once"
ann_neighbors = 5
enrichment_workers = 1

[semantic_clones.inference]
summary_generation = "summary_llm"
code_embeddings = "local_code"
summary_embeddings = "local_code"

[inference.runtimes.bitloops_inference]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-inference/bitloops-inference"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.runtimes.bitloops_local_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-local-embeddings/bitloops-local-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "bge-m3"
cache_dir = "/Users/alex/.cache/bitloops-local-embeddings"

[inference.profiles.summary_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "openai_chat_completions"
model = "gpt-5.4-mini"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1/chat/completions"
temperature = "0.1"
max_output_tokens = 200
```

`bitloops enable --install-embeddings` can create the default local embeddings profile for you. `bitloops init --install-default-daemon` can configure Bitloops cloud, the default local profile, or skip embeddings during onboarding, and `bitloops inference install` installs or repairs the managed standalone inference runtime. `bitloops init --install-default-daemon` or interactive `bitloops enable` can also bind summaries to Ollama automatically when it is available. Edit the daemon config manually only when you need a different profile name, model, or hosted provider.

When Bitloops installs a managed runtime, it writes an absolute path under the Bitloops data directory, as shown above. Use `command = "bitloops-local-embeddings"` or `command = "bitloops-inference"` only when you are managing those standalone binaries yourself on `PATH`.

Embedding model downloads are cache, not durable relational or event store data.

## Internal Runtime Stores

These SQLite files are not configured under `[stores]`:

| Runtime surface | Default path | Purpose |
| --- | --- | --- |
| Daemon runtime store | `<state dir>/daemon/runtime.sqlite` | daemon runtime state, service metadata, supervisor metadata, sync queue state, enrichment queue state |
| Repo runtime store | `<config root>/stores/runtime/runtime.sqlite` | sessions, temporary checkpoints, pre-prompt states, pre-task markers, interaction spool |

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
