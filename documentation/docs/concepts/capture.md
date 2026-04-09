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
- it may also resolve telemetry consent if the existing daemon config needs it

After that, use capture toggles:

```bash
bitloops enable
bitloops enable --install-embeddings
bitloops daemon enable
bitloops disable
```

These commands edit `[capture].enabled` in the nearest discovered project policy. Installed hooks stay in place and no-op while capture is disabled.

`bitloops daemon enable` is an alias to the same implementation. `--install-embeddings` also lets `enable` configure the default local embeddings profile in the effective daemon config and run the existing runtime warm/bootstrap path. In an interactive terminal, plain `bitloops enable` asks about that setup when embeddings are not already configured and defaults to `Yes`.

If the global daemon config already exists but telemetry consent is unresolved, interactive `bitloops enable` can ask before it edits project policy. In non-interactive mode you must pass an explicit telemetry flag.

Use `bitloops uninstall` if you want to remove Bitloops hook integration itself.

## Policy

Shared capture policy lives in `.bitloops.toml`:

```toml
[capture]
enabled = true
strategy = "manual-commit"
```

Local overrides live in `.bitloops.local.toml`, which can also stand on its own without a sibling shared file.

For DevQL indexing scope, capture now also honours:

- `[scope].exclude` inline glob patterns
- `[scope].exclude_from` files (for example `.gitignore` or `config/devql.ignore`)

`exclude_from` files are not generated automatically. Create any ignore-pattern file under the repo-policy root and reference it from `exclude_from`. Use one glob pattern per line, with optional `#` comments.

These rules are evaluated before sync/ingest/watch path discovery. Missing or unreadable `exclude_from` files fail the indexing run before processing starts.

Non-language files that pass plain-text guardrails (UTF-8, non-binary, bounded size) are indexed as file-level `plain_text` artefacts.

## What Capture Does Not Configure

Capture policy does not define:

- store backends
- daemon runtime paths
- credentials
- dashboard bundle locations

Those remain daemon concerns.

The optional embeddings install path is the exception: it updates the effective daemon config, not repo policy, and only adds the default local profile when no active embedding profile is already configured.
