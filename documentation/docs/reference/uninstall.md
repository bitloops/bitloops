---
sidebar_position: 2
title: Uninstalling Bitloops
---

# Uninstalling Bitloops

`bitloops disable` removes Bitloops hooks from the current repository.

`bitloops uninstall` removes Bitloops-managed artefacts from your machine and, for hook targets, from known repositories.

## Interactive And Non-Interactive Use

Run `bitloops uninstall` with no flags in an interactive terminal to open a multi-select picker.

In non-interactive environments, Bitloops requires explicit flags:

```bash
bitloops uninstall --full
bitloops uninstall --config --data --caching
```

## Common Commands

Remove everything Bitloops manages:

```bash
bitloops uninstall --full
```

Remove hook integration from all known repositories:

```bash
bitloops uninstall --agent-hooks --git-hooks
```

Remove hook integration only from the current repository:

```bash
bitloops uninstall --agent-hooks --git-hooks --only-current-project
```

Remove only global machine-scoped artefacts:

```bash
bitloops uninstall --config --data --caching --service --shell
```

## Flags

| Flag | Removes |
| --- | --- |
| `--full` | All targets below, including legacy cleanup |
| `--binaries` | Recognised `bitloops` binaries |
| `--service` | The global daemon service plus daemon state metadata |
| `--data` | Platform data directory plus legacy repo-local `.bitloops/` data |
| `--caching` | Platform cache directory |
| `--config` | Platform config directory plus legacy TLS artefacts in `~/.bitloops/certs` |
| `--agent-hooks` | Supported agent hooks |
| `--git-hooks` | Git hooks installed by Bitloops |
| `--shell` | Managed shell completion integration |
| `--only-current-project` | Restrict hook removal to the current repository |
| `--force` | Skip the confirmation prompt |

## Hook Scope

By default, `--agent-hooks` and `--git-hooks` operate on all known repositories. Bitloops builds that list from the daemon repo registry and also includes the current repository when it can resolve one.

`--only-current-project` is valid only with `--agent-hooks` and/or `--git-hooks`.

## What `--full` Means

`--full` removes:

- recognised binaries
- shell integration managed by Bitloops
- the global daemon service
- global config, data, cache, and state directories
- legacy global TLS artefacts
- Bitloops hook integration in known repositories
- legacy repo-local `.bitloops/` data directories in known repositories

## What Bitloops Does Not Remove

Bitloops only removes artefacts it manages. It does not remove:

- `.bitloops.toml`
- `.bitloops.local.toml`
- unrelated user shell configuration
- non-Bitloops entries in agent config files

## Permissions And Failures

If a binary or service artefact lives somewhere your current user cannot modify, Bitloops reports that failure instead of silently skipping it.

Shell cleanup removes the managed completion integration and any other installer-managed edits that Bitloops can identify confidently. If you added your own PATH entries manually, you may still need to remove those yourself.
