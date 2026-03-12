# Bitloops - Git captures what changed. Bitloops captures why

The open-source intelligence layer for AI-native development. Captures the full developer–AI conversation on every commit and builds a structured semantic model of your codebase that you and your agents can query.

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

From within the repo you are currently working on run:

`bitloops init`

Select your agents or ctrl+a to select them all.

To start recording Checkpoints run:

`bitloops enable`

That's all! Now all of your agent discussion are being saved on your git repo.

## Dashboard

To view your Checkpoints run the following command again from within the root of your repo:

`bitloops dashboard`

## Supported Agents

- [x] Claude Code
- [x] Codex CLI (currently supports `SessionStart` and `Stop` hooks only; richer hook parity will follow as Codex expands hook coverage)
- [x] Cursor
- [x] Gemini
- [ ] GitHub Copilot (Coming soon)
- [x] OpenCode

## What is DevQL?

DevQL is a query language created to offer you and your AI agents valuable and targeted insights regarding your codebases within milliseconds.

[Read more here](./DEVQL-Getting_Started.md)

## FAQs

### Do you need access to my codebase?

No! None of your code is sent to our servers. Your data is stored in your git repo (bitloops/checkpoints branch) as well as your DBs.

### Is this totally free for real?

You bet!

### What kind of databases do I need?

DevQL now uses a provider model:
- Relational backend: `sqlite` or `postgres`
- Events backend: `duckdb` or `clickhouse`

Current runtime adapters are `sqlite`/`postgres` for relational and `duckdb`/`clickhouse` for events (default relational backend: `sqlite`, default events backend: `duckdb`). Legacy `postgres_dsn` / `clickhouse_*` and `BITLOOPS_DEVQL_*` settings remain supported for backward compatibility.

### Why do you use telemetry and why should I opt-in?

Telemetry data help us understand which features users are using the most and help us guide our development. The telemetry data are not connected to specific users and are analysed and considered in aggregate.
