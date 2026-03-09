# Bitloops - Git captures what changed. Bitloops captures why

The open-source intelligence layer for AI-native development. Captures the full developer–AI conversation on every commit and builds a structured semantic model of your codebase that you and your agents can query.

## Installation

### Native Install (Recommended)

macOS, Linux, WSL:

```bash
curl -fsSL https://raw.githubusercontent.com/bitloops/bitloops/main/scripts/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/bitloops/bitloops/main/scripts/install.ps1 | iex
```

Windows CMD:

```cmd
curl -fsSL https://raw.githubusercontent.com/bitloops/bitloops/main/scripts/install.cmd -o install.cmd && install.cmd && del install.cmd
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
- [ ] Codex (Coming as soon as OpenAI adds hooks to Codex; they are working on it)
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

Bitloops works with Clickhouse (for events) and Postgresql (for codebase intelligence). You can install these for free locally via Docker Compose or natively.

### Why do you use telemetry and why should I opt-in?

Telemetry data help us understand which features users are using the most and help us guide our development. The telemetry data are not connected to specific users and are analysed and considered in aggregate.
