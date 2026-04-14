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

[inference.runtimes.bitloops_inference]
command = "bitloops-inference"
args = []
request_timeout_secs = 300

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

Repo policy belongs in `.bitloops.toml` when a repo wants to opt into shared capture behaviour or shared knowledge imports.

## Why This Matters

Capability packs often depend on daemon-managed services such as:

- text-generation profiles
- embedding profiles
- stored checkpoints and events

That is why the daemon config, not repo policy, owns those concerns.
