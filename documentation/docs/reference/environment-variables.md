---
sidebar_position: 3
title: Environment Variables
---

# Environment Variables

Bitloops prefers daemon config and repo policy over environment variables. The list below is the current documented, user-facing runtime surface. Test-only and build-time variables are intentionally omitted.

## General

| Variable | Meaning |
| --- | --- |
| `BITLOOPS_TELEMETRY_OPTOUT` | Disables telemetry dispatch at runtime. It does not answer the CLI consent prompt or rewrite stored daemon-config consent. |
| `BITLOOPS_DAEMON_CONFIG_PATH_OVERRIDE` | Forces repo-scoped commands to use the specified daemon `config.toml` path. This also controls which config `bitloops enable --install-embeddings`, `bitloops daemon enable --install-embeddings`, `bitloops inference install`, and `bitloops init --install-default-daemon` mutate and bootstrap. |
| `BITLOOPS_EMBEDDINGS_VERSION_OVERRIDE` | Overrides the managed `bitloops-embeddings` release tag the CLI installs. When unset, the CLI resolves the latest release from GitHub on the first managed install. |
| `BITLOOPS_INFERENCE_VERSION_OVERRIDE` | Overrides the managed `bitloops-inference` release tag the CLI installs. When unset, the CLI resolves the latest release from GitHub on the first managed install. |
| `BITLOOPS_DISABLE_VERSION_CHECK` | Skips update checks |
| `BITLOOPS_LOG_LEVEL` | Sets the log level for both the daemon log (`daemon.log`) and the telemetry file logger |
| `ACCESSIBLE` | Uses simpler terminal prompts for accessibility workflows |

## Inference Configuration

Configure embeddings and text generation through the daemon config:

- `[semantic_clones]` for capability policy such as `summary_mode` and `embedding_mode`
- `[semantic_clones.inference]` for slot bindings such as `summary_generation`, `code_embeddings`, and `summary_embeddings`
- `[inference.runtimes.<name>]` for executable-backed runtimes such as the standalone `bitloops-embeddings` and `bitloops-inference` binaries
- `[inference.profiles.<name>]` for embeddings and text-generation profiles

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

This interpolation also applies to inference profile values such as `[inference.profiles.summary_llm].api_key`.
