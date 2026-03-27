---
sidebar_position: 1
title: CLI Commands
---

# CLI Commands

Complete reference for all Bitloops CLI commands.

## Global Options

```bash
bitloops --version            # Show version
bitloops --connection-status  # Check store connectivity
bitloops help                 # Show available commands
```

**Example:**

```bash
bitloops --connection-status
```

```
Relational (SQLite): ✔ connected
Event (DuckDB):      ✔ connected
Blob (local):        ✔ available
```

---

## Setup & Lifecycle

### `init`

Initialize Bitloops for an AI agent.

```bash
bitloops init [--agent <name>] [--force] [--telemetry <true|false>]
```

| Flag | Description |
|------|-------------|
| `--agent <name>` | `claude-code`, `cursor`, `copilot`, `codex`, `gemini`, `opencode` |
| `--force` | Reinstall hooks even if already configured |
| `--telemetry` | Enable/disable anonymous telemetry |

If `--agent` is omitted, Bitloops attempts to detect the installed agent.

```bash
bitloops init --agent claude-code
```

```
✔ Detected git repository at /Users/you/project
✔ Stores initialized (SQLite + DuckDB)
✔ Claude Code hooks installed
✔ Agent ready: claude-code
```

### `enable`

Start capturing sessions and checkpoints.

```bash
bitloops enable [--local] [--project]
```

| Flag | Description |
|------|-------------|
| `--local` | Personal only (gitignored `settings.local.json`) |
| `--project` | Team-shared (`settings.json`, committed to git) |

### `disable`

Stop capturing. Does not delete existing data.

```bash
bitloops disable
```

---

## Session & Checkpoint Commands

### `status`

Show current state.

```bash
bitloops status
```

```
Capture:     enabled
Agent:       claude-code
Session:     idle (last session: 5m ago)
Checkpoints: 12 total
```

### `explain`

Show reasoning from the most recent AI session.

```bash
bitloops explain
```

### `rewind`

Interactively browse past checkpoints.

```bash
bitloops rewind
```

### `resume`

Switch between branch-specific sessions.

```bash
bitloops resume
```

### `reset`

Clear current session state without deleting data.

```bash
bitloops reset
```

### `clean`

Remove orphaned data.

```bash
bitloops clean
```

### `doctor`

Diagnose common issues (stuck sessions, missing hooks, store problems).

```bash
bitloops doctor
```

```
✔ Git repository detected
✔ .bitloops/ directory exists
✔ Hooks installed for: claude-code
✔ Stores reachable
✔ No stuck sessions
```

---

## DevQL Commands

### `devql init`

Create DevQL schema for the configured relational and events backends.

```bash
bitloops devql init
```

### `devql ingest`

Ingest checkpoints, events, artefacts, and related enrichments into the configured stores.

```bash
bitloops devql ingest [--init <true|false>] [--max-checkpoints <number>]
```

| Flag | Description |
|------|-------------|
| `--init` | Bootstrap schema before ingesting. Defaults to `true`. |
| `--max-checkpoints` | Limit how many checkpoints are processed. Defaults to `500`. |

**Examples:**

```bash
bitloops devql ingest

bitloops devql ingest --init=false --max-checkpoints 200
```

### `devql query`

Execute a DevQL query against the in-process GraphQL schema.

```bash
bitloops devql query [--graphql] [--compact] "<query>"
```

`bitloops devql query` supports two input modes:

- DevQL DSL when the query contains `->`
- Raw GraphQL otherwise

`--graphql` remains available as an explicit raw-GraphQL override. `--compact` emits compact JSON.

**Examples:**

```bash
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->limit(10)'
bitloops devql query '{ repo(name: "bitloops") { artefacts(first: 5) { edges { node { path symbolFqn canonicalKind } } } } }'
bitloops devql query --graphql --compact '{ health { relational { backend connected } } }'
```

### `devql connection-status`

Check configured backend connectivity for DevQL.

```bash
bitloops devql connection-status
```

This is the command form of the global `bitloops --connection-status` check.

### `devql packs`

Inspect registered capability packs, readiness, migrations, and optional health information.

```bash
bitloops devql packs [--json] [--with-health] [--apply-migrations] [--with-extensions]
```

| Flag | Description |
|------|-------------|
| `--json` | Emit JSON instead of human-readable output |
| `--with-health` | Run pack health checks where available |
| `--apply-migrations` | Apply registered pack migrations before reporting |
| `--with-extensions` | Include Core extension-host language-pack and capability metadata |

### `devql knowledge add`

Add a repository-scoped external knowledge source by URL.

```bash
bitloops devql knowledge add <url> [--commit <sha-or-ref>]
```

### `devql knowledge associate`

Associate an existing knowledge item with a typed Bitloops target.

```bash
bitloops devql knowledge associate <source-ref> --to <target-ref>
```

Example:

```bash
bitloops devql knowledge associate 'knowledge:item-1' --to 'commit:abc123'
```

### `devql knowledge refresh`

Refresh an existing knowledge source and create a new immutable version when the content changes.

```bash
bitloops devql knowledge refresh <knowledge-ref>
```

### `devql knowledge versions`

List immutable document versions for a knowledge item.

```bash
bitloops devql knowledge versions <knowledge-ref>
```

---

## Dashboard

### `dashboard`

Start the local web dashboard.

```bash
bitloops dashboard [--port <number>] [--host <hostname>]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `5667` | Port for the server |
| `--host` | `localhost` | Hostname to bind to |

---

## Other Commands

### `testlens`

Analyze test coverage and map tests to artefacts.

```bash
bitloops testlens
```

### `completion`

Generate shell completions.

```bash
bitloops completion bash
bitloops completion zsh
bitloops completion fish
bitloops completion powershell
```

### `hooks`

Internal command used by agent hook scripts. Not for direct use.

```bash
bitloops hooks <agent-name> <event>
```
