<div align="center">
  <h1>Bitloops</h1>
  <p><strong>The missing memory layer for AI-assisted software development.</strong></p>
  <p>
    Bitloops is the open-source intelligence layer for AI-assisted software
    development. It gives coding agents shared context, captures the full
    developer-agent conversation on every commit, and keeps that knowledge
    local to the repository.
  </p>
  <p>
    <a href="https://bitloops.com">Website</a>
    ·
    <a href="https://bitloops.com/docs">Docs</a>
    ·
    <a href="#quickstart">Quickstart</a>
    ·
    <a href="#how-it-works">How It Works</a>
    ·
    <a href="#roadmap">Roadmap</a>
  </p>
  <p>
    <img alt="License: Apache 2.0" src="https://img.shields.io/badge/License-Apache%202.0-black" />
    <img alt="Open source" src="https://img.shields.io/badge/Open%20Source-Yes-black" />
    <img alt="Local first" src="https://img.shields.io/badge/Data-Local%20First-black" />
    <img alt="Agent agnostic" src="https://img.shields.io/badge/Agents-Agent--Agnostic-black" />
  </p>
</div>

AI coding agents generate more code than teams can comfortably review, explain,
or govern. They rebuild context from scratch, lose decisions when the session
ends, and keep separate mental models across tools.

Bitloops closes that loop. It gives every supported agent the same
repository-scoped memory layer, writes reasoning back on every commit, and
makes AI-assisted work reviewable instead of opaque.

## Why Bitloops

Git captures what changed. Bitloops captures why.

The problem is not code generation anymore. The problem is coherence.

| Without Bitloops | With Bitloops |
| --- | --- |
| Each session starts from zero | Sessions can draw from shared repository memory |
| Agents build siloed, tool-specific context | Supported agents read from the same knowledge store |
| Multi-file changes rely on shallow pattern matching | Context is structured, ranked, and delivered deliberately |
| Reviewers see diffs with no reasoning trail | Commits are linked to the developer-agent conversation behind them |
| Teams re-explain architecture over and over | Important decisions stay available for the next session |

## What Bitloops Does

### 1. Shared memory for coding agents

Bitloops connects tools like Claude Code, Cursor, Gemini CLI, and other
tool-call-capable agents to the same repository-scoped knowledge store.

- Sessions stop resetting.
- Tools stop diverging.
- Context becomes shared infrastructure instead of tribal knowledge.

### 2. Git-linked reasoning capture

Bitloops records the developer-agent workflow around each commit so teams can
trace how a change was produced, not just what landed in the diff.

- Prompt to response to commit mapping
- Per-agent contribution history
- Reviewable checkpoints tied to real repository activity

### 3. Structured context injection

Instead of forcing every session to grep the codebase from scratch, Bitloops
assembles targeted structural and historical signal around the task.

- Less context thrash
- Lower token waste
- Better multi-file coherence

### 4. Local observability

Bitloops includes a local dashboard so teams can inspect AI-assisted activity
without sending code or commit history to a cloud service.

- AI session timeline
- Commit mapping
- Agent comparison
- Contribution metrics

## Quickstart

### Prerequisites

- A git repository
- A supported AI coding agent
- macOS, Linux, or another environment supported by the Bitloops CLI

### Step 1: Install Bitloops

Choose the install method that fits your workflow.

```bash
curl -sSL https://bitloops.com/install.sh | bash
```

```bash
brew install bitloops/tap/bitloops
```

```bash
cargo install bitloops
```

### Step 2: Enable Bitloops in your repository

Run Bitloops from inside the repository you want to work in.

```bash
cd your-project
bitloops enable
```

`bitloops enable` auto-detects supported assistants and prepares the local
Bitloops workflow for that repository.

### Step 3: Work as usual

Keep using your existing agent and developer workflow.

- Ask your agent to inspect or modify the codebase.
- Let Bitloops supply structured context in the background.
- Commit as usual while Bitloops links activity back to repository history.

### Step 4: Open the local dashboard

```bash
bitloops dashboard
```

This launches a local web server so you can inspect session history, commit
mapping, and agent activity directly on your machine.

## How It Works

1. Bitloops installs locally as a CLI.
2. `bitloops enable` connects supported agents inside a repository.
3. Agents request context from Bitloops instead of rebuilding everything from scratch.
4. Bitloops stores workflow metadata, context, and commit-linked history locally.
5. The next session starts with memory instead of guesswork.

## Works With Your Stack

| Category | Support |
| --- | --- |
| Agents | Claude Code, Cursor, Gemini CLI, OpenCode, and other compatible agent workflows |
| Models | Model-agnostic. Bitloops provides context and capture, not generation |
| Storage | Local repository-scoped knowledge store |
| Deployment | Local-first by default |
| Privacy | No code or commit history leaves your environment |

## Why Local-First Matters

Bitloops is infrastructure. It should not require you to hand your codebase to
another platform just to get reliable context and traceability.

- Runs locally
- Works alongside your existing tools
- Keeps repository memory on the machine
- Avoids vendor lock-in at the workflow layer

## Where Bitloops Fits

| Feature | Standalone agents | Bitloops |
| --- | --- | --- |
| Shared memory across tools | No | Yes |
| Git-linked AI session history | No | Yes |
| Local-first repository memory | No | Yes |
| Cross-agent context continuity | No | Yes |
| Vendor-neutral workflow layer | No | Yes |

## Who Bitloops Is For

- Developers tired of re-explaining the same codebase to agents every session
- Teams using more than one AI coding tool on the same repository
- Engineering leaders who need AI-assisted changes to stay reviewable
- Security-conscious organizations that want local-first infrastructure

## Roadmap

Bitloops already captures reasoning, links activity to commits, and provides a
shared memory layer across supported agents. The next stage expands the control
plane further.

- Constraints and validators for AI-generated code
- CI and policy enforcement
- Team-level dashboards
- Self-hosted database options
- Stronger repository-wide governance workflows

## License

Apache 2.0.

## Get Started

Install Bitloops, enable it in a real repository, and stop making every agent
session start from zero.

```bash
curl -sSL https://bitloops.com/install.sh | bash
```
