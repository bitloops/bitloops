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

[![Fork](https://img.shields.io/github/forks/bitloops/bitloops?style=flat-square&label=Fork)](https://github.com/bitloops/bitloops/network/) [![Star](https://img.shields.io/github/stars/bitloops/bitloops?style=flat-square&label=Star)](https://github.com/bitloops/bitloops/stargazers/) [![Commits](https://badgen.net/github/commits/bitloops/bitloops?color=6b7280)](https://github.com/bitloops/bitloops/commits/) [![Version](https://img.shields.io/github/v/tag/bitloops/bitloops?style=flat-square&color=7404e4)](https://github.com/bitloops/bitloops/tags/) [![Downloads](https://img.shields.io/github/downloads/bitloops/bitloops/total?style=flat-square&color=6b7280)](https://github.com/bitloops/bitloops/releases) [![License](https://img.shields.io/github/license/bitloops/bitloops?style=flat-square&color=111827)](https://github.com/bitloops/bitloops/blob/main/LICENSE) [![Contributors](https://img.shields.io/github/contributors/bitloops/bitloops?style=flat-square&color=6b7280)](https://github.com/bitloops/bitloops/graphs/contributors) [![Local First](https://img.shields.io/badge/Data-Local%20First-7404e4?style=flat-square)](https://github.com/bitloops/bitloops) [![Agent Agnostic](https://img.shields.io/badge/Agents-Agent%20Agnostic-7404e4?style=flat-square)](https://github.com/bitloops/bitloops)
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

The `bitloops-embeddings` binary is released separately in `bitloops/bitloops-embeddings`. Explicit embeddings setup flows such as `bitloops init --install-default-daemon`, `bitloops enable --install-embeddings`, and `bitloops embeddings install` can install the managed binary for you. If you are building from source or using a custom runtime, install that binary separately; no Python installation is required.

## Getting Started

1. The fastest way to get started is to run this from inside the git repository or subproject you want to capture:

   ```bash
   bitloops init --install-default-daemon
   ```

   This bootstraps the default daemon service if needed, creates or updates `.bitloops.local.toml`, and installs Bitloops hooks for the current project. Add `--sync=true` if you want the initial current-state sync to run immediately.

2. If you prefer to bootstrap the default daemon explicitly first, use:

   ```bash
   bitloops start --create-default-config
   bitloops init
   ```

   On a fresh machine, interactive `bitloops start` can also prompt to create the default daemon config. During that first bootstrap, Bitloops asks for telemetry consent unless you pass `--telemetry`, `--telemetry=false`, or `--no-telemetry`. If you need a custom daemon config, bootstrap that separately before running `bitloops init`.

3. Toggle capture later if needed:

   ```bash
   bitloops enable
   bitloops disable
   ```

   `bitloops disable` removes Bitloops-managed agent prompt surfaces for the agents recorded in the repo policy.
   `bitloops enable` reinstalls those same managed surfaces and resumes capture.

4. Work as usual and commit normally. Bitloops will capture the relevant
   developer-agent context around those changes.

## Dashboard

To view your checkpoints, run the following command again from within the root of your repo:

```bash
bitloops dashboard
```

To control the daemon directly:

```bash
bitloops start
bitloops daemon stop
bitloops status
bitloops daemon logs
bitloops checkpoints status
```

## Uninstall

Remove Bitloops-managed agent prompt surfaces from the current Bitloops project:

```bash
bitloops disable
```

Remove Bitloops-managed artefacts from your machine as well:

```bash
bitloops uninstall --full
```

## Supported Agents

- [x] Claude Code
- [x] Codex (supports `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, and `Stop`; Bitloops turn checkpoints come from the normal `UserPromptSubmit`/`Stop` lifecycle, while `PreToolUse`/`PostToolUse` are parsed but do not create separate tool checkpoints)
- [x] Cursor
- [x] Gemini
- [x] Copilot
- [x] OpenCode

Codex hooks require both `.codex/hooks.json` and `.codex/config.toml` with `[features].codex_hooks = true`.
Bitloops manages both for `bitloops init --agent codex`, but Codex only honors project-local `.codex/` config in trusted projects.

## What is DevQL?

DevQL is a typed GraphQL interface for querying artefacts, checkpoints, dependencies, and knowledge — available as a CLI DSL, raw GraphQL, or dashboard endpoint.

[Read more here](./DevQL-Getting_Started.md)

To try OpenAI-backed semantic summaries together with the standalone local embeddings runtime, see [Semantic + Embeddings Quickstart](./docs/semantic-embeddings-quickstart.md).

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

No. None of your code is sent to our servers. By default Bitloops keeps its
config, cache, state, and local data in platform app directories on your
machine, with store backends controlled by the daemon config.

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
users and are analysed and considered in aggregate. Bitloops asks for consent
when the default daemon config is first created, and later interactive `init`
or `enable` runs only ask again if consent becomes unresolved for an existing
daemon config.

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
