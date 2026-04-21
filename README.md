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
    <a href="#what-is-bitloops">What is Bitloops</a>
    ·
    <a href="#installation">Installation</a>
    ·
    <a href="#getting-started">Getting Started</a>
    ·
    <a href="https://github.com/bitloops/bitloops/blob/Repo_Documentation_Improvement/DevQL-Getting_Started.md">DevQL</a>
    ·
    <a href="#faqs">FAQs</a>
     ·
    <a href="https://bitloops.com/docs/">Docs</a>
  </p>

[![Fork](https://img.shields.io/github/forks/bitloops/bitloops?style=flat-square&label=Fork)](https://github.com/bitloops/bitloops/network/) [![Star](https://img.shields.io/github/stars/bitloops/bitloops?style=flat-square&label=Star)](https://github.com/bitloops/bitloops/stargazers/) [![Commits](https://badgen.net/github/commits/bitloops/bitloops?color=6b7280)](https://github.com/bitloops/bitloops/commits/) [![Version](https://img.shields.io/github/v/tag/bitloops/bitloops?style=flat-square&color=7404e4)](https://github.com/bitloops/bitloops/tags/) [![Downloads](https://img.shields.io/github/downloads/bitloops/bitloops/total?style=flat-square&color=6b7280)](https://github.com/bitloops/bitloops/releases) [![License](https://img.shields.io/github/license/bitloops/bitloops?style=flat-square&color=111827)](https://github.com/bitloops/bitloops/blob/main/LICENSE) [![Contributors](https://img.shields.io/github/contributors/bitloops/bitloops?style=flat-square&color=6b7280)](https://github.com/bitloops/bitloops/graphs/contributors) [![Local First](https://img.shields.io/badge/Data-Local%20First-7404e4?style=flat-square)](https://github.com/bitloops/bitloops) [![Agent Agnostic](https://img.shields.io/badge/Agents-Agent%20Agnostic-7404e4?style=flat-square)](https://github.com/bitloops/bitloops)
</div>
    
> [!WARNING]
> **Project status: Alpha / Work in Progress**
>
> Bitloops is under active development in the open. It is **not production-ready** yet.
>
> You should currently expect:
> - breaking changes between releases
> - incomplete or evolving documentation
> - rough edges in onboarding and agent integrations
> - limited support for some environments and workflows
>
> We welcome early adopters, testers, and contributors who are comfortable trying experimental developer tooling and sharing feedback.

## What is Bitloops?

AI agents don’t understand your codebase.  
They brute-force it.

They:
- run `grep`, `ls`, `cat`  
- chain tool calls  
- pull too much context  
- burn tokens figuring out what matters  

Bitloops replaces that.

It builds a **local, queryable model of your repository** so agents can fetch what they need directly—dependencies, files, tests, and prior context—in a single call.

Powered by **DevQL**, a GraphQL-based query layer for your codebase.

It also captures every agent interaction as **checkpoints**—giving you a trace of:
- tool usage  
- decisions  
- reasoning  
- and how changes evolved  

So you don’t just get results—you can inspect how they were produced.

---


### Without vs With Bitloops

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

1. The fastest way to get started is to run this from inside the git repository or subproject you want to capture:

   ```bash
   bitloops init --install-default-daemon
   ```

   This bootstraps the default daemon service if needed, creates or updates `.bitloops.local.toml`, and installs Bitloops hooks for the current project.

2. Toggle capture later if needed:

   ```bash
   bitloops enable
   bitloops disable
   ```

   `bitloops disable` removes Bitloops-managed agent prompt surfaces for the agents recorded in the repo policy.
   `bitloops enable` reinstalls those same managed surfaces and resumes capture.

3. Work as usual and commit normally. Bitloops will capture the relevant
   developer-agent context around those changes.

## Dashboard

To view your checkpoints, with the daemon running visit:
[localhost:5667](http://127.0.0.1:5667)


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
- [x] Codex
- [x] Cursor
- [x] Gemini
- [x] Copilot
- [x] OpenCode

## What is DevQL?

DevQL is a typed GraphQL interface for querying artefacts, checkpoints, dependencies, and knowledge — available as a CLI DSL, raw GraphQL, or dashboard endpoint.

[Read more here](./DevQL-Getting_Started.md)

To try OpenAI-backed semantic summaries together with the standalone local embeddings runtime, see [Semantic + Embeddings Quickstart](./docs/semantic-embeddings-quickstart.md).


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
