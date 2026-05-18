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
| `BITLOOPS_LOCAL_EMBEDDINGS_VERSION_OVERRIDE` | Overrides the managed `bitloops-local-embeddings` release tag the CLI installs. When unset, the CLI resolves the latest release from GitHub on the first managed local install. |
| `BITLOOPS_PLATFORM_EMBEDDINGS_VERSION_OVERRIDE` | Overrides the managed `bitloops-platform-embeddings` release tag the CLI installs. When unset, the CLI resolves the latest release from GitHub on the first managed platform install. |
| `BITLOOPS_INFERENCE_VERSION_OVERRIDE` | Overrides the managed `bitloops-inference` release tag the CLI installs. When unset, the CLI resolves the latest release from GitHub on the first managed install. |
| `BITLOOPS_DISABLE_VERSION_CHECK` | Skips update checks |
| `BITLOOPS_LOG_LEVEL` | Sets the log level for both the daemon log (`daemon.log`) and the telemetry file logger |
| `BITLOOPS_WORKOS_CLIENT_ID` | Overrides the built-in WorkOS AuthKit client id used by `bitloops login` |
| `BITLOOPS_WORKOS_BASE_URL` | Overrides the built-in WorkOS API base URL used by CLI auth. This is mainly useful for development and tests |
| `ACCESSIBLE` | Uses simpler terminal prompts for accessibility workflows |

## Inference Configuration

Configure inference runtimes and profiles through the daemon config, and repo semantic embedding intent through project policy:

- daemon `[semantic_clones]` for capability policy such as `summary_mode`
- daemon `[semantic_clones.inference]` for daemon-owned slot bindings such as `summary_generation`
- repo `.bitloops.local.toml` or `.bitloops.toml` `[semantic_clones]` for `embedding_mode`
- repo `[semantic_clones.inference]` for `code_embeddings` and `summary_embeddings` profile bindings
- `[inference.runtimes.<name>]` for executable-backed runtimes such as the standalone `bitloops-local-embeddings`, `bitloops-platform-embeddings`, and `bitloops-inference` binaries
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
