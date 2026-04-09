---
sidebar_position: 1
title: CLI Commands
---

# CLI Commands

Bitloops has a thin CLI plus a single global user-level daemon service, `com.bitloops.daemon`.

For breaking changes from the older command model, see the [upgrade note](./upgrading-to-the-daemon-architecture.md).

## Global Options

```bash
bitloops --version
bitloops --version --check
bitloops version --check
bitloops --connection-status
bitloops help
bitloops help devql
```

## Initial Setup

### `bitloops init`

Bootstraps the current project or subproject.

```bash
bitloops init
bitloops init --sync=true
bitloops init --sync=false
bitloops init --install-default-daemon
bitloops init --install-default-daemon --sync=true
```

Notes:

- Run `bitloops start` first when the daemon is already configured.
- Use `bitloops init --install-default-daemon` on a fresh machine when you want `init` to bootstrap the default daemon service before continuing.
- When `--install-default-daemon` is used and embeddings are not configured yet, `init` also applies the default local embeddings setup before any init-triggered sync runs.
- `init` treats the current working directory as the Bitloops project root.
- `init` creates or updates `.bitloops.local.toml`.
- `.bitloops.local.toml` is added to `.git/info/exclude`.
- `init` installs git hooks plus the selected agent hooks.
- `init` replaces `[agents].supported` with the current selection on rerun.
- Plain `init` does not change embeddings config.
- If embeddings are already configured, `init --install-default-daemon` leaves the active profile in place. Active local profiles may still be warmed; hosted or other non-local profiles are treated as already enabled.
- `init` can queue an initial DevQL current-state sync after hook setup.
- With `--install-default-daemon`, embeddings setup runs before any init-triggered sync so that sync can include embeddings immediately.
- `--sync=true` queues that sync and follows it to completion.
- `--sync=false` skips the initial sync explicitly.
- If `--sync` is omitted in an interactive terminal, `init` asks whether you want to sync the codebase after hooks are installed.
- In non-interactive mode, `init` requires `--sync=true` or `--sync=false`.
- `init` still does not run DevQL ingest. The `--skip-baseline` flag is accepted for compatibility only.
- Use `--agent <name>` to pin the supported agent set.
- `init` accepts `--telemetry`, `--telemetry=false`, and `--no-telemetry`.
- First-run telemetry consent belongs to `bitloops start` when the default daemon config is created for the first time.
- `init` only prompts for telemetry when the daemon config already existed and consent later became unresolved, for example after a CLI upgrade cleared a previous opt-out.
- In non-interactive mode, unresolved telemetry consent requires an explicit telemetry flag.
- If `init` newly adds embeddings config and the runtime bootstrap fails, Bitloops reverts only those embeddings-related daemon-config changes, keeps the rest of init intact, and exits non-zero.

### `bitloops enable`

Enables capture in the nearest discovered project policy.

```bash
bitloops enable
bitloops enable --install-embeddings
bitloops daemon enable
bitloops daemon enable --install-embeddings
```

Notes:

- `enable` edits the nearest discovered `.bitloops.local.toml` or `.bitloops.toml` in place.
- `enable` only toggles `[capture].enabled = true`.
- Installed hooks stay in place and resume capturing without reinstallation.
- `bitloops daemon enable` is an alias to the same implementation and keeps the same telemetry and repo-policy behaviour.
- `--install-embeddings` is an explicit non-interactive opt-in to configure embeddings in the effective daemon config and then run the existing runtime warm/bootstrap path.
- In an interactive terminal, when `--install-embeddings` is absent and embeddings are not already configured, `enable` asks whether to install embeddings and includes them in sync. The prompt defaults to `Yes` with `[Y/n]`; blank input, `y`, and `yes` all opt in.
- If an active embedding profile already exists, `enable` skips daemon-config mutation. Active `local_fastembed` profiles still use the existing warm/bootstrap path; hosted or other non-local profiles are treated as already enabled and do not trigger local runtime bootstrap.
- Embeddings setup targets the effective daemon config in this order: `BITLOOPS_DAEMON_CONFIG_PATH_OVERRIDE`, the nearest repo `config.toml`, then the default global config.
- If no project config is found before the enclosing `.git` root, Bitloops tells you to run `bitloops init`.
- `enable` accepts `--telemetry`, `--telemetry=false`, and `--no-telemetry`.
- `enable` only prompts for telemetry when the daemon config already existed and consent is unresolved.
- In non-interactive mode, unresolved telemetry consent requires an explicit telemetry flag and Bitloops fails before editing project policy.
- If `enable` newly adds embeddings config and the runtime bootstrap fails, Bitloops reverts only those embeddings-related daemon-config changes, leaves capture enabled, and exits non-zero.

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

| Flag                     | Meaning                                                                   |
| ------------------------ | ------------------------------------------------------------------------- |
| `--full`                 | Remove all Bitloops-managed artefacts, including repository-local cleanup |
| `--binaries`             | Remove recognised `bitloops` binaries                                     |
| `--service`              | Remove the daemon service and daemon state metadata                       |
| `--data`                 | Remove global data and repo-local `.bitloops/` data                       |
| `--caching`              | Remove the global cache directory                                         |
| `--config`               | Remove the global config directory and TLS artefacts                      |
| `--agent-hooks`          | Remove supported agent hooks                                              |
| `--git-hooks`            | Remove Bitloops git hooks                                                 |
| `--shell`                | Remove managed shell completion integration                               |
| `--only-current-project` | Limit hook removal to the current repository                              |
| `--force`                | Skip confirmation                                                         |

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
bitloops start --create-default-config
bitloops start --config ./config.toml --bootstrap-local-stores
bitloops daemon start
bitloops daemon start -d
bitloops daemon start --until-stopped
```

Key flags:

| Flag                                                 | Meaning                                                                                                          |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `-d`, `--detached`                                   | Start the daemon in the background without installing an always-on service                                       |
| `--until-stopped`                                    | Install or refresh the global user service and start it                                                          |
| `--host`                                             | Override the bind host                                                                                           |
| `--port`                                             | Override the bind port                                                                                           |
| `--http`                                             | Force local HTTP instead of HTTPS                                                                                |
| `--recheck-local-dashboard-net`                      | Re-run local dashboard TLS and network checks                                                                    |
| `--bundle-dir`                                       | Override the dashboard bundle directory for this run                                                             |
| `--config`                                           | Use an explicit daemon config file                                                                               |
| `--create-default-config`                            | Create the default global daemon config plus local default store files before starting                           |
| `--bootstrap-local-stores`                           | Create the local SQLite, DuckDB, and blob-store artefacts required by the selected daemon config before starting |
| `--telemetry`, `--telemetry=false`, `--no-telemetry` | Set telemetry consent explicitly for this CLI version                                                            |

Notes:

- In interactive mode, plain `start` prompts to create the default daemon config when it is missing. Answering `yes` behaves the same as `--create-default-config`; answering `no` returns the usual missing-config error.
- On a fresh machine, `bitloops start --create-default-config` remains the explicit non-interactive bootstrap path for the default daemon config plus the default SQLite, DuckDB, and blob-store paths.
- When you pass `--config` and the file does not exist, `start` fails.
- `--create-default-config` only works with the default daemon config location. It cannot be combined with `--config`.
- `--bootstrap-local-stores` is the explicit bootstrap path for an existing custom config. It does not create the config file itself; it only creates the local file-backed store artefacts referenced by that config.
- `--bootstrap-local-stores` can be combined with `--config`, which makes it useful for repo-scoped test configs and other non-default daemon setups.
- When `start` creates the default daemon config and no explicit telemetry flag is present, interactive mode prompts for telemetry consent before the daemon continues.
- In non-interactive mode, creating the default daemon config requires an explicit telemetry flag.

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

Shows daemon status, URL, config path, log file path, PID, supervisor information, and sync queue summary.

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
Log file: /Users/alex/.local/state/bitloops/logs/daemon.log
PID: 12345
Supervisor service: com.bitloops.daemon (launchd, installed)
Supervisor state: running
```

When you run `status` inside a repository, it also reports the active or most recent sync task for that repo, including phase and progress when available.

### `bitloops daemon logs`

Prints the daemon log file as raw JSON lines.

```bash
bitloops daemon logs
bitloops daemon logs --tail 50
bitloops daemon logs --follow
bitloops daemon logs --path
```

Notes:

- The default view prints the last 200 lines from `daemon.log`.
- `--follow` keeps streaming appended lines after the initial tail.
- `--path` prints the absolute log file path without reading the file.
- The daemon log lives under the Bitloops state directory at `logs/daemon.log`.

### `bitloops daemon enrichments`

Inspect or control the background enrichment queue:

```bash
bitloops daemon enrichments status
bitloops daemon enrichments pause
bitloops daemon enrichments pause --reason "maintenance"
bitloops daemon enrichments resume
bitloops daemon enrichments retry-failed
```

Use these commands when you want to inspect or control semantic-summary, embeddings, and clone rebuild work owned by the daemon.

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

These commands cover session inspection, resume/rewind workflows, shadow-state cleanup, and stuck-session recovery.

## DevQL

DevQL commands now talk to the local daemon over the existing HTTP and GraphQL surface.

### Schema, ingestion, and sync

```bash
bitloops devql init
bitloops devql ingest
bitloops devql sync
bitloops devql sync --status
bitloops devql sync --validate --status
bitloops devql projection checkpoint-file-snapshots --dry-run
```

Highlights:

- `devql init` explicitly ensures the configured relational and event schemas exist
- daemon startup owns the normal schema bootstrap path
- `devql ingest` performs ingestion only
- `devql sync` queues a sync task and returns immediately by default
- `devql sync --status` follows the queued task until it completes or fails
- `devql sync --validate` queues a read-only validation task instead of mutating current-state tables
- `bitloops status` and `bitloops daemon status` show global sync queue totals and the current repo sync task when you run them inside a repo

### Query and diagnostics

```bash
bitloops devql schema
bitloops devql schema --global
bitloops devql schema --human > bitloops/schema.slim.graphql
bitloops devql schema --global --human > bitloops/schema.graphql
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->limit(10)'
bitloops devql query '{ health { relational { backend connected } } }'
bitloops devql connection-status
bitloops devql packs --with-health
```

Highlights:

- `devql schema` is daemon-backed and fetches SDL from the running DevQL daemon
- `devql schema` without `--global` requires running the command from within a Git repository
- `devql schema --global` can be used outside a repository
- `devql schema` defaults to minified SDL so the output is easier to pass to LLMs and other prompt-driven tooling
- `devql schema --human` prints formatted SDL for review and checked-in schema snapshot export
- `devql query` treats input as DevQL DSL only when it contains `->`; otherwise it treats the input as raw GraphQL
- `devql query` is daemon-backed, not in-process
- that injected hook guidance is instruction-only; Bitloops does not run DevQL queries on the agent's behalf in the hook path
- GitHub currently documents Copilot CLI `sessionStart` output as ignored, so Bitloops can emit the session-start payload there without claiming the Copilot runtime will surface it to the model yet
- `devql packs --with-health` is the easiest way to inspect capability-pack and embeddings health

### Knowledge

```bash
bitloops devql knowledge add https://github.com/bitloops/bitloops/issues/42
bitloops devql knowledge add https://bitloops.atlassian.net/browse/CLI-1370 --commit <sha>
bitloops devql knowledge associate <knowledge_ref> --to commit:HEAD
bitloops devql knowledge refresh <knowledge_ref>
bitloops devql knowledge versions <knowledge_ref>
```

There is no `bitloops devql knowledge ingest` command in the current CLI.

## Test Harness

```bash
bitloops devql test-harness ingest-tests --commit <sha>
bitloops devql test-harness ingest-coverage --lcov coverage/lcov.info --commit <sha> --scope workspace
bitloops devql test-harness ingest-coverage-batch --manifest coverage/manifest.json --commit <sha>
bitloops devql test-harness ingest-results --jest-json reports/jest.json --commit <sha>
```

Use `devql test-harness` to ingest test-linkage, coverage, and results data for the test-harness capability pack. Schema initialisation is handled automatically by the daemon on `bitloops start`.

## Embeddings

```bash
bitloops embeddings pull local
bitloops embeddings doctor
bitloops embeddings clear-cache local
```

These commands inspect, warm, and clear configured embedding profiles from the current repo context.

## Completion

Generate shell completion scripts:

```bash
bitloops completion bash
bitloops completion zsh
bitloops completion fish
```

## Notes

- Hidden internal commands are intentionally omitted from this page.
- Use `bitloops help <command>` when you want the full flag surface for a specific command.
