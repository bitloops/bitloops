---
sidebar_position: 6
title: Capability Packs
---

# Capability Packs

Capability packs add higher-level analysis and enrichment on top of Bitloops data.

## Configuration Split

Capability-pack inference configuration belongs in the global daemon config:

```toml
[semantic_clones.inference]
summary_generation = "summary_llm"

[inference.profiles.summary_llm]
task = "text_generation"
driver = "openai"
model = "gpt-5.4-mini"
api_key = "${OPENAI_API_KEY}"
```

Repo policy belongs in `.bitloops.toml` when a repo wants to opt into shared capture behaviour or shared knowledge imports.

## Why This Matters

Capability packs often depend on daemon-managed services such as:

- text-generation profiles
- embedding profiles
- stored checkpoints and events

That is why the daemon config, not repo policy, owns those concerns.
