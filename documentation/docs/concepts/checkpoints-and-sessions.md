---
sidebar_position: 3
title: Checkpoints & Sessions
---

# Checkpoints & Sessions

Bitloops tracks AI activity through two states: a **live state** that's continuously updated as you work, and a **committed state** that becomes permanent when you `git commit`.

## The Two States

### Live State — Draft Commits

While you're working with an AI agent, Bitloops continuously writes to your local SQLite database. Every meaningful event gets recorded immediately:

- Prompts, responses, reasoning, tool use
- File changes and which artefacts were affected
- Decisions, planning, constraints the agent followed
- Alternatives considered and rejected

This is the **live state**. It updates in real time. If your session is interrupted — power goes out, you close the terminal, whatever — the data is already in the database. Nothing is held in memory.

You can check what's being tracked mid-session:

```bash
bitloops checkpoints status
```

```
Capture:     enabled
Agents:      claude-code, cursor
Session:     active (started 2m ago)
Checkpoints: 12 total (committed)
```

### Committed State — Committed Checkpoints

When you `git commit`, the live state is **promoted to committed state**:

- Draft Commit data moves from the current tables to the committed checkpoint tables
- The checkpoint is linked to the commit SHA
- Session metadata, reasoning summary, and transcript are finalized

In a **default setup** (SQLite), this all happens locally — the data simply moves from one set of tables to another within the same database.

In a **team setup** with remote PostgreSQL, the committed data is written to the shared database — so every team member has access to the reasoning behind every commit.

```
Live state (SQLite, continuously updated)
        ↓  git commit
Committed state (SQLite or PostgreSQL, permanent)
```

The committed state is immutable. Once a checkpoint is created, it doesn't change. This is what makes the reasoning trail trustworthy.

## What a Committed Checkpoint Contains

Each checkpoint is a complete record of the AI session that led to a commit:

- **The full conversation** — every prompt and response
- **Reasoning and decisions** — why this approach, not another
- **Alternatives rejected** — what was tried and discarded
- **Symbols touched** — which functions, classes, modules were modified
- **Model identity** — which AI model produced the output
- **Structured summary** — a human-readable summary of what happened and why

### On disk

```
.bitloops/checkpoints/v1/<commit-sha>/
├── session.json         # Session metadata (agent, model, duration)
├── summary.md           # Structured summary of decisions and reasoning
└── transcript.jsonl     # Full conversation log with tool use
```

This directory is **committed to git**. Push your branch, and the reasoning travels with it. Your team sees it in code reviews. It persists across machines.

## Why This Matters

### For code reviews

A diff shows _what_ changed. A checkpoint shows _everything else_: what was asked, how the agent reasoned, what it rejected, what context it used. Reviews go from "does this look right?" to "do I agree with the approach?"

### For institutional knowledge

The reasoning from today's session becomes context for tomorrow's. An agent working on your auth module next month can see what was changed last week, why, and what didn't work. This happens automatically — no files to update, no memory to maintain.

### For multi-agent teams

It doesn't matter which agent produced the code. Claude Code, Cursor, Copilot — every session feeds into the same committed state. One history, regardless of which tool wrote it.

## Working With Checkpoints

```bash
# See the reasoning behind the last commit
bitloops explain

# Browse checkpoint history interactively
bitloops rewind

# Switch to a different branch's session
bitloops resume

# Diagnose stuck sessions
bitloops doctor
```

## Capture Strategies

Configure when committed checkpoints are created in `.bitloops/settings.json`:

```json
{
  "strategy": "manual_commit"
}
```

- **`manual_commit`** (default) — committed checkpoints created when you `git commit`
- **Session-based** — checkpoints created at session boundaries
