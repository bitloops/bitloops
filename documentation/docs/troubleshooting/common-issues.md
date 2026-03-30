---
sidebar_position: 1
title: Common Issues
---

# Common Issues

## Hooks Are Not Firing

Checks:

1. Run `bitloops enable` in the repository.
2. Verify the repo is still a git repository.
3. Run `bitloops checkpoints status --detailed` to confirm the effective capture policy.

## The Dashboard Does Not Open

Checks:

1. Run `bitloops status`.
2. If needed, start the daemon manually with `bitloops start`.
3. Re-run `bitloops dashboard`.
4. If you use local HTTPS, try `bitloops daemon start --recheck-local-dashboard-net`.

## DevQL Cannot Reach Storage

Checks:

1. Run `bitloops --connection-status`.
2. Confirm the store paths or remote DSNs in the global daemon config.
3. Re-run `bitloops devql init` if the stores were recreated.

## Legacy Repo-Local Data Is Present

Bitloops now warns when it finds old repo-local data directories. Those paths are ignored unless you explicitly point the daemon config at them.

If you want to remove those old directories entirely, use `bitloops uninstall --data` or `bitloops uninstall --full`.

## Capture Seems Disabled Unexpectedly

Checks:

1. Inspect `.bitloops.toml`.
2. Inspect `.bitloops.local.toml`.
3. Run `bitloops checkpoints status --detailed` to confirm the active policy root and fingerprint.
