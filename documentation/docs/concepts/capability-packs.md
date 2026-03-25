---
sidebar_position: 4
title: Capability Packs
---

# Capability Packs

Capability packs extend Bitloops with intelligence features beyond core session capture and structural analysis. Each pack adds a specific dimension of codebase understanding.

## Knowledge Pack — External Context Ingestion

The Knowledge Pack connects code to the decisions and discussions that shaped it. It ingests external documents and associates them with your codebase through explicit, versioned relations.

### What It Ingests

| Source | Content |
|--------|---------|
| **GitHub** | Issues, pull requests, PR review comments, discussions |
| **Jira** | Tickets, epics, requirements |
| **Confluence** | Design documents, ADRs, architectural decisions |

### How It Works

Each external document has a **stable logical identity** with **immutable content versions**. When a document is refreshed (e.g., a Jira ticket is updated), the new version is stored alongside the old one — nothing is overwritten.

Relations are **append-only**: a knowledge item can be associated with specific commits, checkpoints, artefacts, or other knowledge items. This creates a rich web connecting "what the code does" with "why it exists."

```json title=".bitloops/config.json"
{
  "knowledge": {
    "providers": {
      "github": { "token": "${GITHUB_TOKEN}" },
      "jira": {
        "site_url": "https://org.atlassian.net",
        "email": "${ATLASSIAN_EMAIL}",
        "token": "${ATLASSIAN_TOKEN}"
      }
    }
  }
}
```

### What This Enables

An AI agent modifying a function can see:
- The GitHub issue that requested the feature
- The PR discussion where the approach was debated
- The Confluence page documenting the architectural decision
- Previous AI sessions that worked on related code

This is institutional knowledge that normally exists only in senior engineers' heads.

See [Connecting Knowledge Sources](/guides/connecting-knowledge-sources) for setup.

## Semantic Clones Pack — Similar Code Detection

The Semantic Clones Pack detects similar implementations, duplicates, and patterns across your codebase using a multi-signal approach.

### Detection Signals

Each comparison combines three independent signals:

| Signal | What It Measures |
|--------|-----------------|
| **Semantic** | Embedding similarity — do these functions do the same thing conceptually? |
| **Lexical** | Name and identifier similarity — do they use similar naming? |
| **Structural** | AST shape similarity — do they have similar code structure? |

Each signal produces an **explainable component score**, so results are transparent and auditable.

### Relation Kinds

| Relation | Meaning |
|----------|---------|
| **Similar implementation** | Same logic, different names or locations |
| **Exact duplicate** | Identical code in multiple places |
| **Divergent fork** | Were similar once, have since diverged |
| **Pattern reference** | Represents a preferred local pattern to follow |

### What This Enables

- Agents **follow existing conventions** instead of inventing new patterns
- Agents **avoid duplicating** logic that already exists elsewhere
- Developers discover **latent duplication** across the codebase

```json title=".bitloops/config.json"
{
  "semantic": {
    "provider": "openai",
    "model": "gpt-4.1-mini",
    "api_key": "${OPENAI_API_KEY}"
  }
}
```

Embeddings are generated using the FastEmbed library and stored locally in the blob store.

## Test Harness Pack — Verification Maps

The Test Harness Pack maps the relationship between tests and production code, giving agents awareness of the proof structure around any artefact.

### What It Builds

A **verification map** for each artefact:

| Component | Description |
|-----------|-------------|
| **Covering tests** | Which tests exercise this artefact, classified as unit, integration, or E2E |
| **Branch coverage gaps** | Which code paths lack test coverage |
| **Verification level** | Summary of how well-tested the artefact is |

### Test Classification

Tests are classified by their **coverage fan-out**, not naming conventions:

- **Unit test** — tests a single artefact directly
- **Integration test** — tests multiple coordinated artefacts
- **E2E test** — tests a full user workflow

### How It Works

The Test Harness combines:
1. **Static analysis** of test code (which tests import, call, or reference which artefacts)
2. **CI coverage report ingestion** (which lines and branches are covered)

### What This Enables

- Agents **avoid breaking tests** by seeing which tests cover the code they're modifying
- Agents **identify untested paths** where new tests should be written
- Agents **decide what kind of test** to write based on verification gaps

Run analysis with:

```bash
bitloops testlens
```

## Pack Architecture

Each capability pack follows a consistent lifecycle:

1. **Registration** — the pack declares its capabilities and required database migrations
2. **Migration** — tables are created in the relevant stores
3. **Ingestion** — the pack processes its data sources (code, external APIs, CI reports)
4. **Query** — data becomes available through DevQL stages
5. **Health check** — operational status reported to the dashboard

Packs are loaded through the **Extension Host**, which manages their lifecycle. The architecture is designed for extensibility — the core is stable, first-party packs add value without destabilizing it, and the plugin contract supports future third-party extensions.

:::tip
Use `${VAR_NAME}` syntax in config to reference environment variables — keeps secrets out of committed files.
:::
