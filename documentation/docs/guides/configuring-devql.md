---
sidebar_position: 4
title: Configuring DevQL
---

# Configuring DevQL

DevQL is a thin client over the local Bitloops daemon. Run the repo-scoped commands below from inside a Git repository or Bitloops project.

## Schema Bootstrap

```bash
bitloops devql init
```

The daemon bootstraps the DevQL schema automatically on startup. `bitloops devql init` remains available when you want to explicitly ensure the configured relational and event schemas exist.

## Ingest Data

```bash
bitloops devql ingest
```

The CLI resolves repo policy locally, then sends ingestion requests to the daemon. Ingestion no longer owns schema bootstrap.

## Sync Current State

```bash
bitloops devql sync
bitloops devql sync --status
bitloops devql sync --validate --status
```

`bitloops devql sync` now queues a sync task and returns immediately by default. Use `--status` when you want the CLI to follow that queued task until it completes or fails.

`--validate` queues a read-only validation task instead of mutating the current-state tables.

## Query Data

```bash
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->limit(10)'
bitloops devql query '{ health { relational { backend connected } events { backend connected } } }'
```

Queries are DSL only when the input contains `->`. Otherwise the CLI treats the input as raw GraphQL.

## Semantic And Embedding Settings

Semantic and embedding provider settings belong in the global daemon config:

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

## Watch Behaviour

Watcher behaviour belongs in repo policy:

```toml title=".bitloops.toml"
[watch]
watch_debounce_ms = 750
watch_poll_fallback_ms = 2500
```

## Troubleshooting

```bash
bitloops status
bitloops devql packs --with-health
bitloops checkpoints status --detailed
bitloops --connection-status
```

Use `bitloops status` for daemon health, `bitloops devql packs --with-health` for capability-pack and embeddings health, and `bitloops checkpoints status --detailed` for policy root and fingerprint debugging.

`bitloops status` also shows sync queue totals, and when run inside a repository it includes the active or most recent sync task for that repo.
