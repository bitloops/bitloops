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
bitloops devql tasks enqueue --kind ingest
```

The CLI resolves repo policy locally, then sends ingestion requests to the daemon. Ingestion no longer owns schema bootstrap.

## Sync Current State

```bash
bitloops devql tasks enqueue --kind sync
bitloops devql tasks enqueue --kind sync --status
bitloops devql tasks enqueue --kind sync --validate --status
```

`bitloops devql tasks enqueue --kind sync` now queues a sync task and returns immediately by default. Use `--status` when you want the CLI to follow that queued task until it completes or fails.

`--validate` queues a read-only validation task instead of mutating the current-state tables.

Successful sync tasks publish current-state generations. Built-in consumers such as `test_harness.current_state` and `semantic_clones.current_state` reconcile asynchronously from that feed, while historical ingest follow-up stays on the enrichment queue.

## Query Data

```bash
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->limit(10)'
bitloops devql query '{ health { relational { backend connected } events { backend connected } } }'
```

Queries are DSL only when the input contains `->`. Otherwise the CLI treats the input as raw GraphQL.

## Semantic And Embedding Settings

Inference provider settings belong in the global daemon config:

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

[inference.runtimes.bitloops_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-embeddings/bitloops-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_embeddings"
model = "bge-m3"
cache_dir = "/Users/alex/.cache/bitloops-embeddings"

[inference.profiles.summary_llm]
task = "text_generation"
driver = "openai"
model = "gpt-5.4-mini"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1"
```

`bitloops enable --install-embeddings`, `bitloops daemon enable --install-embeddings`, and `bitloops init --install-default-daemon` can create the default local profile for you. Edit the daemon config manually only when you want a hosted profile or a customised local profile.

When Bitloops installs the managed runtime, it writes an absolute path under the Bitloops data directory, as shown above. Use `command = "bitloops-embeddings"` only when you are managing that standalone binary yourself on `PATH`.

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
