---
sidebar_position: 1
title: CLI Commands
---

# CLI Commands

Bitloops now has a thin CLI plus a single global user-level daemon service, `com.bitloops.daemon`.

For breaking changes from the older command model, see the [upgrade note](./upgrading-to-the-daemon-architecture.md).

## Global Options

```bash
bitloops --version
bitloops --version --check
bitloops --connection-status
bitloops help
```

## Initial Setup

### `bitloops init`

Bootstraps the current project or subproject.

```bash
bitloops init
```

Notes:

- Run `bitloops start` first. `init` requires a running daemon.
- `init` treats the current working directory as the Bitloops project root.
- `init` creates or updates `.bitloops.local.toml`.
- `.bitloops.local.toml` is added to `.git/info/exclude`.
- `init` installs git hooks plus the selected agent hooks.
- `init` replaces `[agents].supported` with the current selection on rerun.
- `init` triggers daemon-backed schema initialisation and the baseline sync into `artefacts_current`.
- Use `--agent <name>` to pin the supported agent set or `--skip-baseline` when you want hooks and config without the initial baseline ingestion.

### `bitloops enable`

Enables capture in the nearest discovered project policy.

```bash
bitloops enable
```

Notes:

- `enable` edits the nearest discovered `.bitloops.local.toml` or `.bitloops.toml` in place.
- `enable` only toggles `[capture].enabled = true`.
- Installed hooks stay in place and resume capturing without reinstallation.
- If no project config is found before the enclosing `.git` root, Bitloops tells you to run `bitloops init`.

### `bitloops disable`

Disables capture in the nearest discovered project policy.

```bash
bitloops disable
```

Notes:

- `disable` only toggles `[capture].enabled = false`.
- Hooks and watchers remain installed and become no-ops while capture is disabled.
- Use `bitloops uninstall --agent-hooks --git-hooks` if you want to remove hooks themselves.

### `bitloops uninstall`

Removes Bitloops-managed artefacts from your machine and, for hook targets, from known repositories.

```bash
bitloops uninstall --full
bitloops uninstall --agent-hooks --git-hooks
bitloops uninstall --agent-hooks --git-hooks --only-current-project
bitloops uninstall --config --data --caching
```

Key flags:

| Flag | Meaning |
| --- | --- |
| `--full` | Remove all Bitloops-managed artefacts, including legacy locations |
| `--binaries` | Remove recognised `bitloops` binaries |
| `--service` | Remove the daemon service and daemon state metadata |
| `--data` | Remove global data and legacy repo-local `.bitloops/` data |
| `--caching` | Remove the global cache directory |
| `--config` | Remove the global config directory and legacy TLS artefacts |
| `--agent-hooks` | Remove supported agent hooks |
| `--git-hooks` | Remove Bitloops git hooks |
| `--shell` | Remove managed shell completion integration |
| `--only-current-project` | Limit hook removal to the current repository |
| `--force` | Skip confirmation |

Notes:

- No flags opens an interactive multi-select picker when running in a TTY.
- In non-interactive environments, you must pass explicit flags.
- `disable` is a capture toggle. Use `uninstall` for hook removal or machine-wide cleanup.
- See [Uninstalling Bitloops](./uninstall.md) for target-by-target behaviour and caveats.

## Daemon Lifecycle

The top-level lifecycle aliases are equivalent to `bitloops daemon ...`.

### `bitloops start`

Starts the Bitloops daemon.

```bash
bitloops start
bitloops daemon start
bitloops daemon start -d
bitloops daemon start --until-stopped
```

Key flags:

| Flag | Meaning |
| --- | --- |
| `-d`, `--detached` | Start the daemon in the background without installing an always-on service |
| `--until-stopped` | Install or refresh the global user service and start it |
| `--host` | Override the bind host |
| `--port` | Override the bind port |
| `--http` | Force local HTTP instead of HTTPS |
| `--recheck-local-dashboard-net` | Re-run local dashboard TLS and network checks |
| `--bundle-dir` | Override the dashboard bundle directory for this run |
| `--config` | Use an explicit daemon config file |

Notes:

- When you use the default daemon config path and it does not exist yet, `start` creates it automatically.
- When you pass `--config` and the file does not exist, `start` fails.

### `bitloops stop`

Stops the daemon. If the global service is installed, Bitloops stops that service-managed runtime.

```bash
bitloops stop
bitloops daemon stop
```

### `bitloops restart`

Restarts the daemon using the same targeting rules as `stop`.

```bash
bitloops restart
bitloops daemon restart
```

### `bitloops status`

Shows daemon status, URL, config path, PID, and supervisor information.

```bash
bitloops status
bitloops daemon status
```

Typical output:

```text
Bitloops daemon: running
Mode: always-on service
URL: https://127.0.0.1:5667
Config: /Users/alex/.config/bitloops/config.toml
PID: 12345
Supervisor service: com.bitloops.daemon (launchd, installed)
Supervisor state: running
```

If Bitloops finds legacy repo-local data such as old store directories, `status` also prints a warning that those paths are ignored unless explicitly configured.

## Dashboard

### `bitloops dashboard`

Opens the dashboard in your browser.

```bash
bitloops dashboard
```

Behaviour:

- If the daemon is already reachable, Bitloops opens the existing dashboard URL.
- If the global service is installed but not yet serving the current repo, Bitloops starts it and then opens the dashboard.
- Otherwise Bitloops prompts for foreground, detached, or always-on launch mode.

`dashboard` is now a launcher only. It is no longer the command that owns the server process.

## Capture And History

### `bitloops checkpoints status`

Shows repo-level capture status and the resolved thin-CLI policy.

```bash
bitloops checkpoints status
bitloops checkpoints status --detailed
```

The detailed view includes the discovered policy root and config fingerprint.

### Other capture commands

```bash
bitloops explain
bitloops rewind
bitloops resume <branch>
bitloops reset
bitloops clean
bitloops doctor
```

## DevQL

DevQL commands now talk to the local daemon over the existing HTTP and GraphQL surface.

### Common commands

```bash
bitloops devql init
bitloops devql ingest
bitloops devql query "files changed last 7 days"
bitloops devql knowledge ingest github
```

Highlights:

- `devql init` initialises the configured relational and event stores
- `devql ingest` sends ingestion work through the daemon
- `devql query` uses the local daemon transport rather than in-process GraphQL execution

## Completion

Generate shell completion scripts:

```bash
bitloops completion bash
bitloops completion zsh
bitloops completion fish
```
