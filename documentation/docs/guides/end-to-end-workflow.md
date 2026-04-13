---
sidebar_position: 1
title: End-to-End Workflow
---

# End-to-End Workflow

A realistic development session showing how Bitloops captures AI activity, creates traceability, and serves context.

## The Scenario

You're working on an Express.js API. You want to add rate limiting to your endpoints. You'll use your AI coding agent to implement it and see what Bitloops captures at each step.

## Before You Start

Bitloops is initialized and enabled (see [Quickstart](/getting-started/quickstart)):

```bash
bitloops checkpoints status
```

```
Capture:     enabled
Agents:      claude-code, cursor
Session:     idle
Checkpoints: 8 total (committed)
```

## Step 1: Start Your AI Session

Open your AI agent and give it a task:

```
> Add rate limiting middleware to the API. Use express-rate-limit with a
> 100 requests per 15 minute window. Apply it globally but skip health
> check endpoints.
```

Bitloops immediately begins creating **Draft Commits** — recording your prompt, the session start, and every subsequent action:

```bash
bitloops checkpoints status
```

```
Capture:     enabled
Agent:       claude-code
Session:     active (started 30s ago, 1 Draft Commit)
Checkpoints: 8 total (committed)
```

## Step 2: The Agent Works

Claude Code reads your codebase, installs the package, creates the middleware, and applies it. During this process, Bitloops silently records each Draft Commit:

- Your initial prompt and Claude's planning response
- Each file read (Claude examined `src/app.ts`, `package.json`, existing middleware)
- The shell command `npm install express-rate-limit`
- Each file write with the reasoning for that specific change
- Alternatives Claude considered (e.g., per-route vs global limiting)

You don't see any of this happening — your workflow is unchanged.

## Step 3: Review and Commit

You review Claude's changes and commit:

```bash
git add -A
git commit -m "feat: add rate limiting middleware"
```

```
[main f4e5d6c] feat: add rate limiting middleware
 3 files changed, 45 insertions(+), 2 deletions(-)
 create mode 100644 src/middleware/rate-limit.ts
```

At this moment, Bitloops converts all Draft Commits into a **Committed Checkpoint** — a permanent, immutable record linked to commit `f4e5d6c`.

## Step 4: See What Was Captured

### Full reasoning trace

```bash
bitloops explain
```

```
Session Summary
───────────────
Agent:    Claude Code
Model:    claude-sonnet-4
Duration: 2m 48s
Prompt:   "Add rate limiting middleware to the API..."

Reasoning:
  The agent analyzed the Express app structure and identified
  src/app.ts as the entry point where middleware is registered.
  It created a new rate-limit middleware using express-rate-limit,
  configured with 100 req/15min, and applied it globally via
  app.use() before route registration. Health check routes
  (/health, /ready) were excluded using a skip function.

  Alternatives considered:
  - Per-route rate limiting (rejected: user requested global)
  - Custom rate limiter (rejected: express-rate-limit is standard)

Modified files:
  src/middleware/rate-limit.ts  (+32, new file)
  src/app.ts                   (+8 -1)
  package.json                 (+5 -1)
```

### Check the blast radius

Before this change, you might want to know what it affects:

```bash
bitloops devql query "artefacts(symbol_fqn:'app::middleware')->deps(direction:'in', kind:'references')"
```

This shows every part of the codebase that depends on the middleware chain — useful for understanding the impact.

## Step 5: The Code Review

When a teammate reviews commit `f4e5d6c`, they pull the branch and run:

```bash
bitloops explain
```

Now the review includes not just _what_ changed, but:
- **What the developer asked** — the original prompt
- **How the AI reasoned** — why it chose global middleware over per-route
- **What it considered** — alternatives explored and rejected
- **What it examined** — which files it read to understand the codebase
- **Which model** — the specific AI model used

This transforms code review from "does this diff look correct?" to "do I agree with the reasoning and approach?"

## Step 6: Update the Knowledge Graph

Index the new code for future sessions:

```bash
bitloops devql tasks enqueue --kind ingest
```

```
✔ Scanning repository...
✔ Parsed 143 artefacts (1 new, 2 updated)
✔ Mapped 91 relationships (2 new)
✔ Knowledge graph updated
```

The next time any AI agent works on this codebase, it can query for the rate limiting middleware directly — understanding what it does, where it's applied, and why it was implemented this way.

## The Compound Effect

Without Bitloops, this session would be invisible. The commit message says "add rate limiting" but doesn't capture the reasoning, alternatives, or decision process.

With Bitloops, every AI-assisted commit carries its full context. Over weeks and months, this compounds:

- **Month 1:** 20 Committed Checkpoints with reasoning traces
- **Month 3:** 80+ checkpoints, knowledge from linked GitHub issues, semantic understanding of key modules
- **Month 6:** A rich institutional knowledge base that makes every new AI session dramatically more informed

The AI agent working on your codebase in month 6 has access to every decision, every rejected alternative, and every external context from the previous five months. It operates with the accumulated understanding of a senior engineer, not the blank slate of a first-day hire.
