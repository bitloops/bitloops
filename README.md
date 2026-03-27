<div align="center">
  <img src="assets/bitloops-logo_320x132.png" alt="Bitloops logo" width="360" height="148" />
  <h1>Give your AI agents high-signal context in milliseconds.</h1>
<p>
  <strong>Bitloops continuously models your codebase and development history so agents can retrieve
  architecture, decisions, and intent instantly — instead of crawling your repositories.</strong>
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
    <a href="https://github.com/bitloops/bitloops/blob/Repo_Documentation_Improvement/DevQL-Getting_Started.md">DevQL</a>
    ·
    <a href="#faqs">FAQs</a>
  </p>
  <p style="margin:8px 0;">
  [![GitHub stars](https://img.shields.io/github/stars/bitloops/bitloops?style=for-the-badge&logo=github&color=181717)](https://github.com/bitloops/bitloops/stargazers)
  [![GitHub forks](https://img.shields.io/github/forks/bitloops/bitloops?style=for-the-badge&logo=github&color=181717)](https://github.com/bitloops/bitloops/network)
  [![GitHub contributors](https://img.shields.io/github/contributors/bitloops/bitloops?style=for-the-badge&logo=github&color=181717)](https://github.com/bitloops/bitloops/graphs/contributors)
  [![License](https://img.shields.io/badge/license-Apache%202.0-blue?style=for-the-badge&logo=apache&logoColor=white)](LICENSE)
  </p>
  <p style="margin:8px 0;">
  [![Downloads](https://img.shields.io/github/downloads/bitloops/bitloops/total?style=for-the-badge&logo=github&color=6b7280)](https://github.com/bitloops/bitloops/releases)
  [![Last commit](https://img.shields.io/github/last-commit/bitloops/bitloops/main?style=for-the-badge&logo=github&color=6b7280)](https://github.com/bitloops/bitloops/commits)
  [![Local First](https://img.shields.io/badge/Data-Local%20First-7404e4?style=for-the-badge)](https://github.com/bitloops/bitloops)
  [![Agent Agnostic](https://img.shields.io/badge/Agents-Agent%20Agnostic-7404e4?style=for-the-badge)](https://github.com/bitloops/bitloops)
  </p>
</div>

<br>

## About Bitloops

Bitloops is a memory and context layer for AI coding agents.

AI agents can generate code quickly, but the reasoning behind their changes is
often lost. Sessions restart from zero, context drifts across tools, and
reviewers are left with diffs that do not explain how a decision was made.

Bitloops captures and structures agent reasoning alongside your repository. It
links conversations, decisions, and generated code back to commits so teams can
understand, review, and improve AI-assisted development.

Instead of sending large amounts of repository context to the model, Bitloops
retrieves the most relevant code, architecture, and prior reasoning. This gives
agents higher-quality context, reduces token usage, and improves the likelihood
of correct results on the first attempt.

---

## Why Bitloops

Bitloops introduces three core capabilities:

- **Repository memory for AI agents**
- **Targeted context retrieval for codebases**
- **Traceable AI reasoning through Git commits**

Together, these allow agents to work with better context while teams retain
visibility and governance over AI-generated changes.

|                                                              |                                                               |
| ------------------------------------------------------------ | ------------------------------------------------------------- |
| **Without Bitloops**                                         | **With Bitloops**                                             |
| Each session starts from zero                                | Sessions build on shared repository memory                    |
| Agents send large amounts of repository context to the model | Only relevant code, architecture, and reasoning are retrieved |
| High token usage with noisy context                          | Fewer tokens with higher-quality context                      |
| Reviewers see diffs with no reasoning trail                  | Commits link to the developer–AI conversation                 |
| AI reasoning disappears between sessions                     | Agent reasoning remains searchable                            |
| Limited governance over AI-generated changes                 | Teams can review and audit AI reasoning through commits       |
| Multiple iterations to reach a correct result                | Higher chance of getting it right the first time              |

---

## What Bitloops does

Bitloops adds structured memory, context retrieval, and reasoning traceability
to AI-assisted development.

- **Shared memory for coding agents:** Supported tools access the same repository-scoped knowledge layer, turning
  context into shared infrastructure instead of siloed sessions.

- **Git-linked reasoning capture:** Bitloops records the developer–agent workflow around each commit so teams can
  trace how a change was produced, not just what appears in the diff.

- **Targeted context retrieval:** Instead of forcing every session to search the codebase from scratch, Bitloops
  retrieves the most relevant structural and historical signals for the task.

- **Local observability:** A local dashboard lets teams inspect AI-assisted activity without sending code
  or commit history to a cloud service.

- **External knowledge integration:** Connect GitHub issues, pull requests, Jira tickets, and Confluence pages
  directly to your repository context.

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
- [x] Codex (currently supports `SessionStart` and `Stop` hooks only; richer hook parity will follow as Codex expands hook coverage)
- [x] Cursor
- [x] Gemini
- [x] Copilot
- [x] OpenCode

## What is DevQL?

DevQL is a typed GraphQL interface for querying artefacts, checkpoints, dependencies, and knowledge — available as a CLI DSL, raw GraphQL, or dashboard endpoint.

[Read more here](./DevQL-Getting_Started.md)

## External Knowledge

Bitloops can ingest repository-scoped external knowledge by URL:

```bash
bitloops devql knowledge add https://github.com/bitloops/bitloops/issues/42
bitloops devql knowledge add https://bitloops.atlassian.net/browse/CLI-1370 --commit <sha>
```

Supported sources:

- GitHub issues
- GitHub pull requests
- Jira issues
- Confluence pages

For this flow, SQLite stores repository-scoped identity and relations, DuckDB stores version metadata, and the full payload content is stored in the configured blob backend.

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

We welcome:

- bug reports
- documentation improvements
- performance improvements
- new integrations with AI coding tools
- architectural ideas and discussions

If you are unsure where to start, open an issue and we will help guide you.

### Code of Conduct

We're committed to fostering an inclusive and respectful community. Read our
[CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) for guidelines.

## License

Apache 2.0.
