---
sidebar_position: 6
title: Capability Packs
---

# Capability Packs

Capability packs add higher-level analysis and enrichment on top of Bitloops data.

## Configuration Split

Capability-pack credentials and model configuration belong in the global daemon config:

```toml
[semantic]
provider = "openai_compatible"
model = "qwen2.5-coder"
api_key = "${OPENAI_API_KEY}"
```

Repo policy belongs in `.bitloops.toml` when a repo wants to opt into shared capture behaviour or shared knowledge imports.

## Why This Matters

Capability packs often depend on daemon-managed services such as:

- semantic providers
- embedding providers
- stored checkpoints and events

That is why the daemon config, not repo policy, owns those concerns.
