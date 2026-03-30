---
sidebar_position: 4
title: Configuring DevQL
---

# Configuring DevQL

DevQL is now a thin client over the local Bitloops daemon.

## Initialise The Stores

```bash
bitloops devql init
```

This initialises the configured relational and event stores from the global daemon config.

## Ingest Data

```bash
bitloops devql ingest
```

The CLI resolves repo policy locally, then sends ingestion requests to the daemon.

## Query Data

```bash
bitloops devql query "files changed last 7 days"
```

## Semantic And Embedding Settings

Semantic and embedding provider settings belong in the global daemon config:

```toml
[semantic]
provider = "openai_compatible"
model = "qwen2.5-coder"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1"

[stores]
embedding_provider = "local"
embedding_model = "jinaai/jina-embeddings-v2-base-code"
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
bitloops checkpoints status --detailed
bitloops --connection-status
```

Use `bitloops status` for daemon health and `bitloops checkpoints status --detailed` for policy root and fingerprint debugging.
