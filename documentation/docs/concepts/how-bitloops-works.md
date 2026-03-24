---
sidebar_position: 1
title: How Bitloops Works
---

# How Bitloops Works

Once you install and enable Bitloops, it runs silently in the background. Here's what actually happens.

## Listen and Capture

Bitloops hooks into your AI agents and listens to every conversation. Prompts, reasoning, decisions, alternatives, file changes — all recorded as **Draft Commits** in real time.

When you `git commit`, those Draft Commits become **Committed Checkpoints**: permanent, immutable records linked to that commit.

Deep dive: [Capture: Git for AI](/concepts/capture) and [Checkpoints & Sessions](/concepts/checkpoints-and-sessions).

## Store Continuously

Bitloops maintains a local SQLite database that's **continuously updated** with your local edits and agent activity. Every file change, every new artefact, every dependency edge — indexed in real time.

When you commit, the current state is promoted to the committed tables. In a default setup, that's SQLite. If you've configured PostgreSQL for your team, the committed data goes there instead — so everyone shares the same intelligence.

Deep dive: [The Knowledge Store](/concepts/knowledge-store).

## Feed Context Back

When a new request is made, Bitloops provides the AI agent with far better context than it could get from reading files alone:

- **AST analysis** — parsed dependency graph of your entire codebase
- **Semantic analysis** — what symbols mean, how they're used, similar patterns
- **Test analysis** — which tests cover what, where the gaps are
- **Blast radius analysis** — what breaks if this changes
- **Historical reasoning** — previous sessions, decisions, rejected approaches
- **External knowledge** — linked GitHub issues, Jira tickets, design docs

All queryable through [DevQL](/concepts/devql) — agents call it autonomously.

Deep dive: [The Intelligence Layer](/concepts/intelligence-layer).

## The Loop

**Capture and context form a reinforcing loop.**

```
You work with your AI agent
        ↓
Bitloops captures the session (Draft Commits)
        ↓
You commit → Committed Checkpoints created
        ↓
Knowledge graph updated with new artefacts, decisions, reasoning
        ↓
Next session starts → agent gets richer context
        ↓
Better context → fewer tokens wasted → better output
        ↓
Better output → richer checkpoints → even better context
        ↓
Repeat
```

The more you use it, the better the context becomes. The better the context, the less tokens get wasted on reconstruction. The less waste, the better the output.

It's a flywheel, and it starts turning from day one.

## Across Sessions. Across Agents. Automatically.

**The context flows automatically.**

No `CLAUDE.md` files to maintain. No `agent.md` to update. No `.cursorrules` to keep in sync. No memory files to curate by hand.

Switch from Claude Code to Cursor? The context is there. Come back to a project after two weeks? The history is there. Onboard a new teammate? The reasoning behind every AI-assisted commit is there.

One knowledge store. Every agent. Every session. Automatically.

## Data Flow

A typical session, step by step:

1. You open your AI agent and give it a task
2. Bitloops hooks fire → session starts, Draft Commits begin recording
3. You send prompts → each one captured with the agent's response and reasoning
4. The agent modifies files → changes tracked, SQLite updated in real time
5. You `git commit` → Draft Commits promoted to Committed Checkpoints
6. Committed data moves to the permanent tables (SQLite or remote PostgreSQL)
7. Knowledge graph updated with new artefacts and relationships
8. Next session → agent queries DevQL for precise context instead of re-reading files
9. Richer context → better output → the loop continues
