---
sidebar_position: 7
title: Using the Dashboard
---

# Using the Dashboard

The dashboard is the human-facing surface of Bitloops's accumulated intelligence. It provides visual access to sessions, checkpoints, the knowledge graph, and usage patterns.

## Starting the Dashboard

```bash
bitloops dashboard
```

```
✔ Dashboard server started
  → https://bitloops.local:5667
```

If local HTTPS is not set up yet, follow [Dashboard Local HTTPS Setup](/guides/dashboard-local-https-setup).

### Custom Port and Host

```bash
bitloops dashboard --port 8080
bitloops dashboard --host 0.0.0.0 --port 3000
bitloops dashboard --http --host 127.0.0.1
bitloops dashboard --recheck-local-dashboard-net
```

## DevQL GraphQL Endpoints

When the dashboard server is running, it also serves the DevQL GraphQL surface:

| Route               | Purpose                       |
| ------------------- | ----------------------------- |
| `/devql`            | GraphQL queries and mutations |
| `/devql/playground` | DevQL Explorer UI             |
| `/devql/sdl`        | Generated schema SDL          |
| `/devql/ws`         | Subscription transport        |

This is the same schema the CLI executes in-process for `bitloops devql query`, `bitloops devql init`, `bitloops devql ingest`, and the DevQL knowledge commands.

## Dashboard Views

### Checkpoints

Browse all Committed Checkpoints. Each shows:

- **Commit** — the linked git commit hash and message
- **Agent and model** — which AI agent and model produced this code
- **Reasoning summary** — structured summary of decisions and alternatives considered
- **Files modified** — list with diff stats
- **Timestamp** — when the checkpoint was created

Click any checkpoint to see the full session transcript and reasoning trace.

### Sessions

Individual AI agent sessions with full detail:

- **Full transcript** — every prompt, response, tool use, and decision
- **Symbol-level activity** — which artefacts were read, modified, or created
- **Duration and outcome** — how long and what was accomplished
- **Live Draft Commits** — for active sessions, see what's being captured in real time

### Artefacts

After running `bitloops devql ingest`:

- **All indexed code structures** — functions, structs, classes, modules with full definitions
- **Search and filtering** — by language, artefact kind, name, or file path
- **Dependency relationships** — how artefacts connect, with edge kinds visible
- **Domain heatmap** — which areas of the codebase see the most AI activity

### AI Usage Patterns

Aggregate views across sessions:

- **Agent and model usage** — which agents and models are being used and how often
- **Session frequency** — development velocity patterns
- **Review readiness** — which recent commits have full checkpoint coverage
- **Institutional knowledge health** — how much of the codebase has accumulated context

### Health Status

Real-time status of configured stores:

- Relational store connectivity and size
- Event store connectivity and size
- Blob store availability
- Knowledge source connection status

## Dashboard Configuration

```json title=".bitloops/config.json"
{
  "version": "1.0",
  "scope": "project",
  "settings": {
    "dashboard": {
      "local_dashboard": {
        "tls": true,
        "bitloops_local": true
      }
    }
  }
}
```

The dashboard is a bundled web application served by Bitloops's built-in HTTP server (Axum). It runs entirely locally — no external services involved.
