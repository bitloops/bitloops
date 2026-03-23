---
sidebar_position: 2
title: "Capture: Git for AI"
---

# Capture: Git for AI

Git captures _what_ changed. Bitloops captures _why_ and _how_.

Every AI-assisted development session produces valuable reasoning: why this approach was chosen, what alternatives were considered, what constraints the agent operated under, what files it examined. Today, all of that disappears when the session ends.

Bitloops makes it permanent.

## How Capture Works

When you work with an AI agent, Bitloops hooks into the conversation and records everything in real time as **Draft Commits**:

| What's Captured | Why It Matters |
|----------------|---------------|
| **Every prompt you sent** | Reviewers see what was actually asked |
| **The agent's full response** | Including reasoning, not just code output |
| **Planning and decision-making** | Why this approach, not another |
| **Alternatives considered and rejected** | What the agent tried and discarded |
| **Constraints and rules followed** | Architecture rules, conventions, domain logic |
| **Files read and changed** | What context the agent used to make decisions |
| **Model identity** | Which AI model produced this output |
| **Tool use** | Shell commands, file operations, API calls |

Draft Commits update in real time. You can check what's being captured mid-session with `bitloops status`.

## From Draft to Permanent

When you `git commit`, Draft Commits are promoted to **Committed Checkpoints** — permanent, immutable, indexed records tied to that commit SHA.

```
Working with AI agent
        ↓
Draft Commits (live, temporary, updating in real time)
        ↓
git commit
        ↓
Committed Checkpoint (permanent, immutable, linked to commit)
```

Committed Checkpoints are stored in `.bitloops/checkpoints/` and designed to be committed to git. Push your branch, and the reasoning travels with it.

See [Checkpoints & Sessions](/concepts/checkpoints-and-sessions) for the full details.

## What This Enables

### Reviewable AI Commits

A PR diff tells you _what_ changed. Committed Checkpoints tell you _everything else_:

- What the developer asked the AI to do
- How the AI arrived at this implementation
- What it considered and rejected
- What files it read to understand the codebase
- The full reasoning chain from prompt to output

Code review goes from "does this diff look correct?" to "do I agree with the reasoning and approach?"

### Context That Survives

Captured sessions feed directly into the [Intelligence Layer](/concepts/intelligence-layer). The reasoning from today's session becomes context for tomorrow's.

- Why was this function refactored? Check the checkpoint.
- What was tried before and didn't work? It's in the history.
- What constraints should the next agent follow? Captured in the reasoning trace.

This happens automatically. No files to maintain, no memory to curate.

### Agent-Agnostic Recording

Bitloops uses an agent-agnostic hook processor. The core principle:

> Agent hook events are signals of potential mutation, not proof. The authoritative source is Git repository state.

When a hook fires, Bitloops:

1. **Fingerprints** the Git state — `sha256(HEAD + branch + working tree status)`
2. **Deduplicates** — if the fingerprint matches the last event, it's ignored
3. **Detects real change** — only records if the working tree actually changed or a new commit appeared

This means no false triggers from repeated tool invocations, reverted edits, or non-mutating commands. It works identically across Claude Code, Cursor, Copilot, Codex, Gemini — any agent.

### Traceability and Governance

For teams that need to know how AI is being used:

- Which agents and models are producing code
- How often AI-assisted commits happen
- What quality of reasoning is behind the output
- Full attribution of AI vs human-authored code

The [Dashboard](/guides/using-the-dashboard) surfaces all of this visually.

## The Compounding Effect

Every captured session makes the next one better. The reasoning from session 1 becomes context for session 50. Decisions accumulate. Institutional knowledge builds.

A codebase with 6 months of capture history is a fundamentally different experience for an AI agent than a fresh repo. The agent isn't starting from scratch — it's building on everything that came before.

That's what "Git for AI" means. Not just version control for code, but version control for reasoning.
