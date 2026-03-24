---
sidebar_position: 1
title: Overview
slug: /intro
---

# Bitloops

**The open-source intelligence layer for AI-native development.**

Bitloops sits between your AI coding agents and your codebase. It captures everything your agents do, builds a persistent knowledge graph, and feeds that intelligence back — so every session is smarter than the last.

Think of it as Git for AI reasoning. Git captures _what_ changed. Bitloops captures _why_ and _how_.

## Why does this matter?

AI coding agents are incredible at generating code. But they have a serious blind spot: every session starts from scratch. No memory, no context, no history.

**Your agents don't know your architecture.** They do surface-level pattern matching, not structural analysis. Multi-file changes? Educated guesses. The result: broken dependencies, wasted tokens on re-reading files, and code that conflicts with your design.

**Everything they learn disappears.** An agent spends 10 minutes understanding your auth module, makes a great refactor, and then... the session closes. Gone. The next agent (or the same one tomorrow) starts over. All that reasoning, all those decisions — lost.

**Nobody knows what the AI actually did.** Your teammate opens a PR. The diff looks fine. But what did the AI consider and reject? What constraints was it working under? What files did it read to arrive at this approach? Today, that context is invisible.

**Every agent builds its own little world.** Your team uses Claude Code, Cursor, and Copilot. Each one constructs its own isolated understanding of the codebase. No shared context. No reconciliation. Developers (and their agents) working from competing mental models, with no way to detect conflicts.

**Tokens are expensive and unattributable.** You're spending on AI, but you can't attribute cost to features, teams, or outcomes. Context gaps cause rework. Reasoning loss makes reviews longer. Without observability, you can't optimize.

## What Bitloops does about it

### Capture — Git for AI

Bitloops records the entire developer-AI interaction. As you work, every meaningful change is captured as a **Draft Commit** in real time: the full conversation, what changed, which model was used, the reasoning, and even alternatives the agent considered and rejected.

When you `git commit`, Draft Commits become **Committed Checkpoints** — permanent, immutable, indexed records tied to that commit. Now every AI-assisted commit is reviewable in a way that was previously impossible. Not just the diff, but the full reasoning chain.

### Context — The Senior Engineer Guide

A senior engineer doesn't just read code. They carry a mental picture of the system: dependencies, recent changes and why, past bugs, architectural constraints, previous developers' reasoning. They've built this over months.

AI agents without context start every session as a junior engineer on their first day.

Bitloops gives them what a senior engineer has:

- **Structural understanding** — a complete dependency graph built by parsing your code with Tree-sitter, not heuristics
- **Blast radius** — "what will this break?" answered through full dependency graph traversal
- **Semantic understanding** — what each symbol means in your system, its role, patterns, and domain mapping
- **Historical reasoning** — previous AI sessions on this code, why things were refactored, what was tried and discarded
- **External knowledge** — linked GitHub issues, Jira tickets, Confluence pages, and architectural decisions

All of this is queryable through **DevQL**, a graph-navigation language that agents can call autonomously.

### It compounds

Here's the thing most people miss: Bitloops gets better the longer you use it. A 6-month-old codebase with Bitloops is dramatically richer than a new one. Every session adds reasoning history. Every linked ticket adds context. Every checkpoint preserves decisions.

The agent working on your code in month 6 has access to everything that happened in months 1 through 5. It's not starting from scratch — it's building on accumulated intelligence.

## Supported Agents

Bitloops is agent-agnostic. One knowledge store, any agent:

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code/overview)
- [Cursor](https://www.cursor.com/)
- [GitHub Copilot](https://github.com/features/copilot)
- [Codex](https://openai.com/index/codex/)
- [Gemini](https://ai.google.dev/)
- [OpenCode](https://opencode.ai/)

Use one agent or all six. Bitloops captures them all, independently, into the same knowledge store.

## Where to go from here

**Just want to get started?** The [Quickstart](/getting-started/quickstart) takes about 2 minutes.

**Want to understand how it works?** Read [How Bitloops Works](/concepts/how-bitloops-works) for the architecture, or the [End-to-End Workflow](/guides/end-to-end-workflow) for a real-world walkthrough.

**Already running Bitloops?** Check out [Configuring DevQL](/guides/configuring-devql), [Team Setup](/guides/team-setup), or the [CLI Reference](/reference/cli-commands).

## Open Source

Bitloops is [Apache 2.0 licensed](https://github.com/bitloops/bitloops). Read the code, fork it, break it, fix it, [contribute](/contributors). We're building this in the open because infrastructure should be inspectable.
