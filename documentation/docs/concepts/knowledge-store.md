---
sidebar_position: 5
title: Knowledge Store
---

# Knowledge Store

The knowledge store is daemon-owned. The daemon is responsible for credentials, provider access, storage, and query execution.

## Split Of Responsibility

Global daemon config defines:

- provider credentials
- provider endpoints
- store backends

Repo policy defines:

- which imported knowledge source definitions apply to this repo

## Example

Global daemon config:

```toml
[knowledge.providers.github]
token = "${GITHUB_TOKEN}"
```

Repo policy:

```toml
[imports]
knowledge = ["bitloops/knowledge.toml"]
```

Imported knowledge definition:

```toml
[sources.github]
repositories = ["bitloops/bitloops"]
```

## Why This Split Exists

It keeps secrets and daemon infrastructure out of the repo while still allowing teams to share which knowledge sources matter for a given codebase.
