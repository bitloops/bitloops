---
sidebar_position: 3
title: Environment Variables
---

# Environment Variables

Bitloops prefers daemon config and repo policy over environment variables, but a small number of variables are still supported for runtime control and temporary overrides.

## General

| Variable | Meaning |
| --- | --- |
| `BITLOOPS_TELEMETRY_OPTOUT` | Disables telemetry dispatch at runtime. It does not answer the CLI consent prompt or rewrite stored daemon-config consent. |
| `BITLOOPS_DISABLE_VERSION_CHECK` | Skips update checks |
| `BITLOOPS_LOG_LEVEL` | Sets the log level for both the daemon log (`daemon.log`) and the telemetry file logger |
| `ACCESSIBLE` | Uses simpler terminal prompts for accessibility workflows |

## DevQL Semantic Overrides

These override semantic settings resolved from daemon config:

| Variable | Meaning |
| --- | --- |
| `BITLOOPS_DEVQL_SEMANTIC_PROVIDER` | Semantic provider override |
| `BITLOOPS_DEVQL_SEMANTIC_MODEL` | Semantic model override |
| `BITLOOPS_DEVQL_SEMANTIC_API_KEY` | Semantic API key override |
| `BITLOOPS_DEVQL_SEMANTIC_BASE_URL` | Semantic base URL override |

## DevQL Embedding Overrides

| Variable | Meaning |
| --- | --- |
| `BITLOOPS_DEVQL_EMBEDDING_PROVIDER` | Embedding provider override |
| `BITLOOPS_DEVQL_EMBEDDING_MODEL` | Embedding model override |
| `BITLOOPS_DEVQL_EMBEDDING_API_KEY` | Embedding API key override |

## Watcher Overrides

| Variable | Meaning |
| --- | --- |
| `BITLOOPS_DEVQL_WATCH_DEBOUNCE_MS` | Overrides repo-policy debounce |
| `BITLOOPS_DEVQL_WATCH_POLL_FALLBACK_MS` | Overrides repo-policy polling fallback |
| `BITLOOPS_DISABLE_WATCHER_AUTOSTART` | Prevents watcher auto-start for supported commands |

## Dashboard Bundle Overrides

These are mainly useful in development or CI:

| Variable | Meaning |
| --- | --- |
| `BITLOOPS_DASHBOARD_CDN_BASE_URL` | Overrides the dashboard CDN base URL |
| `BITLOOPS_DASHBOARD_MANIFEST_URL` | Overrides the dashboard manifest URL |

## Interpolation In Daemon Config

Daemon config values may reference environment variables with `${VAR_NAME}`:

```toml
[knowledge.providers.github]
token = "${GITHUB_TOKEN}"
```

Use this for secrets and per-machine credentials. Repo policy files should not contain secrets.
