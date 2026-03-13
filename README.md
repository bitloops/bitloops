<div align="center">
  <h1>Bitloops</h1>
  <p><strong>Git captures what changed. Bitloops captures why.</strong></p>
  <p>
    The open-source intelligence layer for AI-native development. Bitloops
    captures the full developer-agent conversation around each commit, builds
    structured repository memory, and keeps that knowledge local to your repo.
  </p>
  <p>
    <a href="https://bitloops.com">Website</a>
    ·
    <a href="#why-bitloops">Why Bitloops</a>
    ·
    <a href="#installation">Installation</a>
    ·
    <a href="#getting-started">Getting Started</a>
    ·
    <a href="#what-is-devql">DevQL</a>
    ·
    <a href="#faqs">FAQs</a>
  </p>
  <p>
    <img alt="License: Apache 2.0" src="https://img.shields.io/badge/License-Apache%202.0-black" />
    <img alt="Open source" src="https://img.shields.io/badge/Open%20Source-Yes-black" />
    <img alt="Local first" src="https://img.shields.io/badge/Data-Local%20First-black" />
    <img alt="Agent agnostic" src="https://img.shields.io/badge/Agents-Agent--Agnostic-black" />
  </p>
</div>

AI coding agents generate code quickly, but teams still lose the reasoning
behind changes. Sessions restart from zero, context drifts across tools, and
reviewers are left with diffs that do not explain how a decision was made.

Bitloops closes that loop. It gives supported agents shared repository memory,
links work back to commits, and makes AI-assisted development more reviewable
instead of more opaque.

## Why Bitloops

| Without Bitloops | With Bitloops |
| --- | --- |
| Each session starts from zero | Sessions can draw from shared repository memory |
| Agents build siloed, tool-specific context | Supported agents read from the same knowledge store |
| Reviewers see diffs with no reasoning trail | Commits are linked to the developer-agent conversation behind them |
| Teams re-explain architecture over and over | Important decisions remain available for the next session |

## What Bitloops Does

### Shared memory for coding agents

Bitloops gives supported tools the same repository-scoped knowledge layer, so
context becomes shared infrastructure instead of tribal knowledge.

### Git-linked reasoning capture

Bitloops records the developer-agent workflow around each commit so teams can
trace how a change was produced, not just what landed in the diff.

### Structured context injection

Instead of forcing every session to search the codebase from scratch, Bitloops
assembles targeted structural and historical signal around the task.

### Local observability

Bitloops includes a local dashboard so teams can inspect AI-assisted activity
without sending code or commit history to a cloud service.

## Installation

### Native Install (Recommended)

macOS, Linux, WSL:

```bash
curl -fsSL https://bitloops.com/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://bitloops.com/install.ps1 | iex
```

Windows CMD:

```cmd
curl -fsSL https://bitloops.com/install.cmd -o install.cmd && install.cmd && del install.cmd
```

### Homebrew (macOS/Linux)

```bash
brew install bitloops/tap/bitloops
```

## Getting Started

From within the repo you are currently working on:

1. Enable Bitloops for the repository:

   ```bash
   bitloops enable
   ```

   This configures Bitloops settings and installs the required git hooks.

2. Initialize your agent integrations:

   ```bash
   bitloops init
   ```

   Select your agents, or use `Ctrl+A` to select them all. If you already know
   the target agent, you can also run `bitloops init --agent <name>`.

3. Work as usual and commit normally. Bitloops will capture the relevant
   developer-agent context around those changes.

## Dashboard

To view your checkpoints, run the following command again from within the root
of your repo:

```bash
bitloops dashboard
```

## How It Works

1. Bitloops installs locally as a CLI.
2. `bitloops enable` configures repository settings and git hooks.
3. `bitloops init` connects supported agents inside the repository.
4. Agents request context from Bitloops instead of rebuilding everything from
   scratch.
5. Bitloops stores workflow metadata, structured context, and commit-linked
   history locally.

## Supported Agents

- [x] Claude Code
- [x] Codex CLI (currently supports `SessionStart` and `Stop` hooks only; richer hook parity will follow as Codex expands hook coverage)
- [x] Cursor
- [x] Gemini CLI
- [x] OpenCode
- [ ] GitHub Copilot (Coming soon)

## What is DevQL?

DevQL is a query language created to offer you and your AI agents valuable and
targeted insights regarding your codebases within milliseconds.

[Read more here](./DevQL-Getting_Started.md)

## FAQs

### Do you need access to my codebase?

No. None of your code is sent to our servers. Your data is stored in your git
repo (`bitloops/checkpoints/v1`) as well as your databases.

### Is this totally free for real?

You bet.

### What kind of databases do I need?

DevQL now uses a provider model:
- Relational backend: `sqlite` or `postgres`
- Events backend: `duckdb` or `clickhouse`

Current runtime adapters are `sqlite`/`postgres` for relational and
`duckdb`/`clickhouse` for events (default relational backend: `sqlite`,
default events backend: `duckdb`). Legacy `postgres_dsn` / `clickhouse_*` and
`BITLOOPS_DEVQL_*` settings remain supported for backward compatibility.

### Why do you use telemetry and why should I opt-in?

Telemetry data help us understand which features users are using the most and
help us guide our development. The telemetry data are not connected to specific
users and are analyzed and considered in aggregate.

## License

Apache 2.0.
