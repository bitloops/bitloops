---
sidebar_position: 1
title: Capture
---

# Capture

Capture is the repo-side part of Bitloops. Hooks and the slim CLI collect context locally, apply repo policy, and send the resulting events to the daemon.

## Enabling Capture

```bash
bitloops enable
```

This installs:

- git hooks
- supported agent hooks for the current repository

Disable them again with:

```bash
bitloops disable
```

## Policy

Shared capture policy lives in `.bitloops.toml`:

```toml
[capture]
enabled = true
strategy = "manual-commit"
```

Local overrides live in `.bitloops.local.toml`.

## What Capture Does Not Configure

Capture policy does not define:

- store backends
- daemon runtime paths
- credentials
- dashboard bundle locations

Those remain daemon concerns.
