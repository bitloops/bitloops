---
sidebar_position: 2
title: Uninstalling Bitloops
---

# Uninstalling Bitloops

`bitloops disable` turns capture off in the nearest discovered project policy while leaving hooks installed.

`bitloops uninstall` removes Bitloops-managed artefacts from your machine and, for repo-local targets, from known repositories or Bitloops projects.

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

Remove repo-local Bitloops policy from all known projects:

```bash
bitloops uninstall --repo-config
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
| `--full` | All targets below, including repository-local cleanup |
| `--binaries` | Recognised `bitloops` binaries |
| `--service` | The global daemon service plus daemon state metadata |
| `--data` | Platform data directory plus repo-local `.bitloops/` data |
| `--caching` | Platform cache directory |
| `--config` | Platform config directory plus TLS artefacts in `~/.bitloops/certs` |
| `--agent-hooks` | Supported agent hooks plus Bitloops-managed repo-local agent guidance and matching exclude entries |
| `--repo-config` | Repo-local `.bitloops.toml`, `.bitloops.local.toml`, and the managed `.bitloops.local.toml` exclude entry |
| `--git-hooks` | Git hooks installed by Bitloops |
| `--shell` | Managed shell completion integration |
| `--only-current-project` | Restrict repo-local uninstall targets to the current repository or project |
| `--force` | Skip the confirmation prompt |

## Repo-Local Scope

By default, `--agent-hooks`, `--repo-config`, and `--git-hooks` operate on all known repositories or Bitloops projects. Bitloops builds that list from the daemon repo registry and also includes the current repository when it can resolve one.

`--only-current-project` is valid only with `--agent-hooks`, `--repo-config`, and/or `--git-hooks`.

If you only want to pause capture without removing hooks, use `bitloops disable`.

## What `--full` Means

`--full` removes:

- recognised binaries
- shell integration managed by Bitloops
- the global daemon service
- global config, data, cache, and state directories
- TLS artefacts in `~/.bitloops/certs`
- Bitloops hook integration in known repositories
- Bitloops-managed repo-local agent guidance and matching exclude entries
- repo-local `.bitloops.toml` and `.bitloops.local.toml` policy files in cleaned projects
- the managed `.bitloops.local.toml` `.git/info/exclude` entry in cleaned projects
- repo-local `.bitloops/` data directories in known repositories

## What Bitloops Does Not Remove

Bitloops only removes artefacts it manages. It does not remove:

- non-Bitloops entries in `.git/info/exclude`
- unrelated user shell configuration
- non-Bitloops entries in agent config files

## Permissions And Failures

If a binary or service artefact lives somewhere your current user cannot modify, Bitloops reports that failure instead of silently skipping it.

Shell cleanup removes the managed completion integration and any other installer-managed edits that Bitloops can identify confidently. If you added your own PATH entries manually, you may still need to remove those yourself.
