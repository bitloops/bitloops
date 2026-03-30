---
sidebar_position: 1
title: Capture
---

# Capture

Capture is the repo-side part of Bitloops. Hooks and the slim CLI collect context locally, apply repo policy, and send the resulting events to the daemon.

## Enabling Capture

```bash
bitloops init
```

Project bootstrap installs:

- git hooks
- supported agent hooks for the current project

After that, use capture toggles:

```bash
bitloops enable
bitloops disable
```

These commands edit `[capture].enabled` in the nearest discovered project policy. Installed hooks stay in place and no-op while capture is disabled.

Use `bitloops uninstall` if you want to remove Bitloops hook integration itself.

## Policy

Shared capture policy lives in `.bitloops.toml`:

```toml
[capture]
enabled = true
strategy = "manual-commit"
```

Local overrides live in `.bitloops.local.toml`, which can also stand on its own without a sibling shared file.

## What Capture Does Not Configure

Capture policy does not define:

- store backends
- daemon runtime paths
- credentials
- dashboard bundle locations

Those remain daemon concerns.
