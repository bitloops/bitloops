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

Create database schema.

```bash
bitloops devql init
```

### `devql ingest`

Parse source files and populate the knowledge graph.

```bash
bitloops devql ingest [--knowledge-url <url>]
```

| Flag | Description |
|------|-------------|
| `--knowledge-url` | Ingest a specific external resource (GitHub issue, Jira ticket, Confluence page) |

**Examples:**

```bash
# Ingest the codebase
bitloops devql ingest

# Ingest a GitHub issue
bitloops devql ingest --knowledge-url https://github.com/org/repo/issues/123

# Ingest a Jira ticket
bitloops devql ingest --knowledge-url https://your-org.atlassian.net/browse/PROJ-456
```

### `devql query`

Query the knowledge graph.

```bash
bitloops devql query "<query>"
```

```bash
bitloops devql query "artefacts(language='rust')"
bitloops devql query "checkpoints"
bitloops devql query "chat_history"
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
