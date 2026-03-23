---
sidebar_position: 3
title: The Intelligence Layer
---

# The Intelligence Layer

Bitloops doesn't just record what happened — it builds an intelligence layer on top of your codebase. This is the context that makes your AI agents genuinely useful.

## What's in the Intelligence Layer?

Think of it as everything a senior engineer carries in their head, made persistent and queryable.

### Structural Intelligence

Your codebase parsed into a proper dependency graph using [Tree-sitter](https://tree-sitter.github.io/) — deterministic, parser-backed, not grep or heuristics.

- Every function, class, struct, module, interface, and type — extracted with full definitions
- Every dependency edge — imports, calls, references, inheritance, implementations
- Cross-file relationships — how symbols connect across your entire codebase

This is what powers blast radius analysis: "if I change this function, what breaks?" Bitloops traverses the full graph to give you a precise answer.

### Semantic Intelligence

Beyond structure, Bitloops understands what your code _means_:

- **Purpose summaries** — what a symbol does in the context of your system, generated through a smart cascade (docstring → LLM summary → template fallback)
- **Similarity detection** — finds functions that do the same thing with different names, spots duplicates, identifies divergent forks
- **Pattern recognition** — surfaces the conventions your codebase follows, so agents can match them

The similarity engine combines semantic (embeddings), lexical (naming), and structural (AST shape) signals. Results are explainable — not just "these seem similar" but _why_ they're similar.

### Historical Intelligence

Every captured session becomes part of the intelligence layer. This is where the compounding effect kicks in.

When an agent works on your auth module, it can access:

- Previous sessions that touched that code — what was changed and why
- Decisions that were made — and the reasoning behind them
- Alternatives that were rejected — and why they were rejected
- Planning discussions — constraints, rules, architectural direction

This context flows across sessions automatically. No files to maintain, no memory to curate.

### External Intelligence

Code tells you _what_ exists. External knowledge tells you _why_ it exists.

Bitloops ingests and links:

- **GitHub** issues, PRs, and review discussions
- **Jira** tickets and epics
- **Confluence** design docs and architectural decisions

Each document is versioned and connected to specific commits and artefacts. The links are append-only — refreshing a document preserves the full history.

### Test Intelligence

The relationship between tests and production code, mapped as **verification maps**:

- Which tests cover which artefacts (classified as unit, integration, or E2E based on coverage fan-out, not naming)
- Branch-level coverage gaps
- How well-tested a given module actually is

Agents see this before making changes — they avoid breaking tests, identify untested paths, and know when to write new ones.

## How It's Served

All of this intelligence is queryable through **[DevQL](/concepts/devql)** — a graph-navigation language:

```bash
# What depends on this function?
bitloops devql query "artefacts(symbol_fqn:'auth::validate') → deps(direction:'in')"

# What was this function like at the last release?
bitloops devql query "asOf(ref:'v1.0') → artefacts(symbol_fqn:'auth::validate')"
```

Agents call DevQL autonomously. They get precise, high-signal context in milliseconds — no scanning your entire repo, no wasting tokens on files they don't need.

The [Dashboard](/guides/using-the-dashboard) provides the same intelligence visually: browse artefacts, explore dependencies, review session history, check coverage.

## The Compounding Effect

Here's what makes the intelligence layer different from a static index: **it gets better every day.**

- **Week 1:** Structural graph + a few captured sessions
- **Month 1:** 30+ sessions with reasoning history, linked GitHub issues, semantic understanding of key modules
- **Month 6:** A rich institutional knowledge base — every decision, every rejected alternative, every external context from five months of development

The agent working on your codebase in month 6 doesn't start from scratch. It has the accumulated understanding of every session that came before it.

That's not a file you maintain. That's an intelligence layer that builds itself.
