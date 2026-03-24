---
sidebar_position: 3
title: Key Features
---

# Key Features

Bitloops isn't just one thing. It's a set of capabilities that work together to make your AI agents genuinely useful — not just fast.

Here's what's under the hood.

## Capture — The Full Reasoning Trail

Every AI session is recorded. Not just the final output — the entire journey.

As you work with an agent, Bitloops captures **Draft Commits** in real time: the prompts you sent, the agent's responses, which files it read, what it changed, what it decided _not_ to change, and the alternatives it considered. When you `git commit`, those become **Committed Checkpoints** — permanent records linked to that commit.

This means:

- You can explain any commit: "what was the AI thinking?"
- Code reviews include the reasoning, not just the diff
- Nothing gets lost between sessions — decisions, rejected approaches, context that informed the output
- Your team can see exactly how AI-generated code was produced

The conversations _are_ the context. Every session feeds back into the knowledge store, so the next agent that touches that code inherits the full history of what happened and why.

## Structural Context — Your Codebase as a Graph

Bitloops parses your code using [Tree-sitter](https://tree-sitter.github.io/) (deterministic, parser-backed — not heuristics) and builds a structured knowledge graph:

- **Artefacts** — every function, class, struct, module, interface, and type, with their full definitions
- **Dependencies** — imports, calls, references, inheritance, implementations
- **Cross-file relationships** — how symbols connect across your codebase

This isn't a text search. It's a real dependency graph that agents can traverse to understand your architecture before making changes.

Currently supports **Rust**, **TypeScript**, and **JavaScript**, with more languages coming.

## Blast Radius — "What Will This Break?"

Probably the most useful question you can ask before changing code.

Bitloops computes transitive impact through full dependency graph traversal. Change a function's signature? Bitloops tells you every caller — direct and indirect — that would break. Rename a type? Here's everything that references it.

This is available through DevQL queries and gives agents (and you) precise impact analysis before touching anything.

## Semantic Understanding

Beyond structure, Bitloops builds conceptual understanding of your code:

- What a symbol **means** in the context of your system — its role, usage patterns, domain mapping
- **Purpose summaries** generated through a smart cascade: docstring extraction → LLM summary → template fallback
- **Similarity detection** — finds functions that do the same thing with different names, identifies duplicates, spots divergent forks of originally similar code

The similarity engine combines three signals — semantic (embeddings), lexical (naming), and structural (AST shape) — so results are explainable, not just "these seem similar."

This helps agents follow your existing patterns instead of inventing new ones.

## Historical Context — Sessions as Knowledge

Here's where it gets interesting. Every captured session becomes part of the intelligence layer.

When an agent works on your auth module next month, it can access:

- The session where someone refactored the token validation last week
- The reasoning behind why they chose JWT over session tokens
- The alternatives that were considered and rejected
- The review discussion that followed

This is context that normally lives only in senior engineers' heads. Bitloops makes it persistent and queryable.

And because it compounds — every session adds more — a codebase with 6 months of Bitloops history is dramatically richer than a fresh one.

## External Knowledge — The "Why" Behind Code

Code tells you _what_ exists. But not _why_ it exists.

Bitloops ingests context from outside your codebase and connects it to the relevant code:

- **GitHub** — issues, PRs, review discussions
- **Jira** — tickets, epics, requirements
- **Confluence** — design docs, ADRs, architectural decisions

Each document is versioned and linked to specific commits and artefacts. When an agent modifies a function, it can see the Jira ticket that requested the feature, the PR discussion where the approach was debated, and the Confluence page documenting the architectural decision.

This is institutional knowledge that otherwise exists only in people's heads — or buried in comment threads nobody reads.

## Test Awareness

Bitloops maps the relationship between tests and production code, building **verification maps**:

- Which tests cover which artefacts (classified as unit, integration, or E2E)
- Which code paths lack coverage
- How well-tested a given function or module actually is

Agents can see this before making changes — so they avoid breaking existing tests, identify untested paths, and know when to write new ones.

## DevQL — Query It All

All of this intelligence is queryable through **DevQL**, a graph-navigation language:

```bash
# What depends on this function?
bitloops devql query "artefacts(symbol_fqn:'auth::validate') → deps(direction:'in')"

# What did this function look like at the last release?
bitloops devql query "asOf(ref:'v1.0') → artefacts(symbol_fqn:'auth::validate')"
```

Agents can call DevQL autonomously to get exactly the context they need, in milliseconds, without scanning your entire repo.

## The Dashboard

For humans who prefer visuals over CLI output, there's a local dashboard:

```bash
bitloops dashboard
```

Browse checkpoints, session transcripts, artefact relationships, AI usage patterns, and store health — all from `localhost:5667`.

---

That's the toolkit. Each feature is useful on its own, but they're designed to work together: capture feeds context, context improves agent output, better output creates richer checkpoints, richer checkpoints feed better context.

It's a flywheel. And it starts with three commands in the [Quickstart](/getting-started/quickstart).
