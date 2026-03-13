  <div align="center">
  <img src="assets/bitloops-logo_320x132.png" alt="Bitloops logo" width="360" height="148" />
  <h1>Git captures what changed. Bitloops captures why.</h1>
  <p>
    The open-source intelligence layer for AI-driven software development. Bitloops 
    captures the full developer–AI conversation for every commit and builds a 
    structured semantic model of your codebase that you and your agents can query.
  </p>
  <p>
    <a href="https://bitloops.com">Website</a>
    ·
    <a href="https://github.com/bitloops/bitloops">GitHub</a>
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
  <p style="margin:4px 0;">
    <a href="https://github.com/bitloops/bitloops/network/">
      <img src="https://img.shields.io/github/forks/bitloops/bitloops.svg?style=social&label=Fork&maxAge=2592000&colorA=7404e4&colorB=000000" alt="GitHub forks" />
    </a>
    <a href="https://github.com/bitloops/bitloops/stargazers/">
      <img src="https://img.shields.io/github/stars/bitloops/bitloops.svg?style=social&label=Star&maxAge=2592000&colorA=7404e4&colorB=000000" alt="GitHub stars" />
    </a>
    <a href="https://github.com/bitloops/bitloops/commit/">
      <img src="https://badgen.net/github/commits/bitloops/bitloops?color=7404e4" alt="GitHub commits" />
    </a>
    <a href="https://github.com/bitloops/bitloops/tags/">
      <img src="https://badgen.net/github/tag/bitloops/bitloops?color=7404e4" alt="GitHub tag" />
    </a>
    <a href="https://github.com/bitloops/bitloops/releases">
      <img src="https://img.shields.io/github/downloads/bitloops/bitloops/total.svg?color=7404e4" alt="Downloads" />
    </a>
    <a href="https://github.com/bitloops/bitloops/blob/main/LICENSE">
      <img src="https://img.shields.io/github/license/bitloops/bitloops?colorA=7404e4&colorB=000000" alt="License" />
    </a>
    <a href="https://github.com/bitloops/bitloops/graphs/contributors">
      <img src="https://img.shields.io/github/contributors/bitloops/bitloops?colorA=7404e4&colorB=000000" alt="Contributors" />
    </a>
    <a href="https://github.com/bitloops/bitloops/graphs/contributors">
      <img alt="Local first" src="https://img.shields.io/badge/Data-Local%20First-7404e4" />
    </a>
    <a href="https://github.com/bitloops/bitloops/graphs/contributors">
      <img alt="Agent agnostic" src="https://img.shields.io/badge/Agents-Agent--Agnostic-7404e4" />
    </a>
  </div>

## About Bitloops

AI coding agents generate code quickly, but teams still lose the reasoning
behind changes. Sessions restart from zero, context drifts across tools, and
reviewers are left with diffs that do not explain how a decision was made.

Bitloops closes that loop. It gives supported agents shared repository memory,
links work back to commits, and makes AI-assisted development more reviewable
instead of more opaque.

### Why Bitloops

<table style="width:100%; border-collapse:collapse; background:#f3f1ff; border:2px solid #7404e4;">
  <thead>
    <tr>
      <th style="text-align:left; padding:12px; background:#7404e4; color:#ffffff;">Without Bitloops</th>
      <th style="text-align:left; padding:12px; background:#7404e4; color:#ffffff;">With Bitloops</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td style="padding:12px; border-top:1px solid #d0c0ff; background:#fdfcff;">Each session starts from zero</td>
      <td style="padding:12px; border-top:1px solid #d0c0ff; background:#eef2ff;">Sessions can draw from shared repository memory</td>
    </tr>
    <tr>
      <td style="padding:12px; border-top:1px solid #d0c0ff; background:#fdfcff;">Agents build siloed, tool-specific context</td>
      <td style="padding:12px; border-top:1px solid #d0c0ff; background:#eef2ff;">Supported agents read from the same knowledge store</td>
    </tr>
    <tr>
      <td style="padding:12px; border-top:1px solid #d0c0ff; background:#fdfcff;">Reviewers see diffs with no reasoning trail</td>
      <td style="padding:12px; border-top:1px solid #d0c0ff; background:#eef2ff;">Commits are linked to the developer-agent conversation behind them</td>
    </tr>
    <tr>
      <td style="padding:12px; border-top:1px solid #d0c0ff; background:#fdfcff;">Teams re-explain architecture over and over</td>
      <td style="padding:12px; border-top:1px solid #d0c0ff; background:#eef2ff;">Important decisions remain available for the next session</td>
    </tr>
  </tbody>
</table>

### What Bitloops Does

- **Shared memory for coding agents:** Bitloops gives supported tools the same repository-scoped knowledge layer, so
context becomes shared infrastructure instead of tribal knowledge.

- **Git-linked reasoning capture:** Bitloops records the developer-agent workflow around each commit so teams can
trace how a change was produced, not just what landed in the diff.

- **Structured context injection:** Instead of forcing every session to search the codebase from scratch, Bitloops
assembles targeted structural and historical signal around the task.

- **Local observability:** Bitloops includes a local dashboard so teams can inspect AI-assisted activity
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

1. Initialize your agent integrations:

   ```bash
   bitloops init
   ```

   Select your agents, or use `Ctrl+A` to select them all.

2. To start recording Checkpoints run:

   ```bash
   bitloops enable
   ```

3. Work as usual and commit normally. Bitloops will capture the relevant
   developer-agent context around those changes.

## Dashboard

To view your checkpoints, run the following command again from within the root of your repo:

```bash
bitloops dashboard
```


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
repo (`bitloops/checkpoints/v1`) as well as your DBs.

### Is this totally free for real?

You bet!

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

## Community & Support

### Contributing

We welcome contributions from the community! Your input helps make Bitloops better
for everyone. See [CONTRIBUTING.md](./CONTRIBUTING.md) to get started.

### Code of Conduct

We're committed to fostering an inclusive and respectful community. Read our
[CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) for guidelines.

## License

Apache 2.0.
