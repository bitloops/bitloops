---
title: Semantic + Embeddings Quickstart
---

# Semantic + Embeddings Quickstart

This guide shows the quickest way to try:

- semantic summaries through a configured LLM provider
- local embeddings through the standalone `bitloops-embeddings` runtime
- the daemon-owned enrichment queue and health checks

Run the repo-scoped commands below from inside a Git repository or Bitloops project, typically after `bitloops init`.

## What You Need

- `bitloops`
- `bitloops-embeddings`
- a semantic provider API key if you want LLM semantic summaries

In a packaged release, both CLIs should be installed together. In a source checkout, build and install both:

```bash
cargo build
cargo dev-install
cargo install --path bitloops-embeddings --force
```

On macOS, prefer `cargo dev-install` over plain `cargo install --path bitloops --force` so the
DuckDB runtime is staged correctly for the installed `bitloops` binary.

## Fastest Setup Paths

Bitloops can now set up the default local embeddings profile for you without manual `config.toml` edits.

If you are bootstrapping a repo and want `init` to install the default daemon first:

```bash
bitloops init --install-default-daemon --sync=true
```

When embeddings are not already configured, `bitloops init --install-default-daemon`:

- bootstraps the default daemon config if needed
- adds the default local embeddings profile to the effective daemon config
- runs the existing runtime warm/bootstrap path before any init-triggered sync

If the repo is already initialised and you just want to add embeddings:

```bash
bitloops enable --install-embeddings
bitloops daemon enable --install-embeddings
```

Interactive `bitloops enable` also asks whether to install embeddings when they are not already configured. The prompt uses `[Y/n]`, so pressing `Enter` accepts the recommended setup.

## Config Location And Targeting

Semantic and embeddings runtime settings live in the Bitloops daemon config.

Typical macOS location, for example:

```text
~/Library/Application Support/bitloops/config.toml
```

If you have not bootstrapped the daemon config yet:

```bash
bitloops start --create-default-config
```

Repo-scoped embeddings setup uses the effective daemon config in this order:

1. `BITLOOPS_DAEMON_CONFIG_PATH_OVERRIDE`
2. the nearest repo `config.toml`
3. the default global daemon config

That same resolved path is used for both config mutation and runtime bootstrap.

## Default Local Embeddings Config

When Bitloops auto-configures embeddings, it writes the minimum profile required for the default local setup:

```toml
[semantic_clones]
embedding_profile = "local"

[embeddings.profiles.local]
kind = "local_fastembed"
```

Notes:

- `local` is the default auto-created profile name.
- `local_fastembed` is the default auto-created profile kind.
- If an active embedding profile already exists, Bitloops does not overwrite it.
- If that existing active profile is local, Bitloops still runs the normal warm/bootstrap path for it.
- If that existing active profile is hosted or otherwise non-local, Bitloops treats embeddings as already enabled and skips local runtime bootstrap.

## Optional Semantic Summaries

If you also want semantic summaries, add semantic provider settings to the daemon config:

```toml
[semantic]
provider = "openai"
model = "gpt-5.4-mini"
api_key = "${OPENAI_API_KEY}"
```

Notes:

- `kind = "local_fastembed"` uses the local embeddings runtime with the default local model settings.
- For platform-specific config paths, use the configuration reference alongside your OS defaults.

## Warm The Local Model

Run:

```bash
bitloops embeddings pull local
```

What this does:

- starts the standalone embeddings runtime
- validates the selected profile
- warms the local model cache
- downloads the model if it is missing

This is the best first check that `bitloops-embeddings` is installed and reachable.

`bitloops enable --install-embeddings` and `bitloops init --install-default-daemon` reuse this same warm/bootstrap path automatically; `bitloops embeddings pull local` remains useful when you want to rerun it explicitly.

## Verify Health

Run:

```bash
bitloops devql packs --with-health
```

Healthy output should show:

- semantic summary provider ready
- embedding profile resolved
- runtime command available
- runtime handshake succeeded

If embeddings are disabled, health should report that explicitly instead of failing.

## Run Ingest

Run:

```bash
bitloops devql ingest
```

Then inspect the shared enrichment queue:

```bash
bitloops daemon enrichments status
```

That queue covers:

- semantic summary upgrades
- embeddings
- clone-edge rebuilds

On a healthy path, pending jobs should go down over time and failed jobs should stay at `0`.

## Query The Result

The easiest user-facing proof that embeddings were written and used is a clone query:

```bash
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->clones(min_score:0.6)->limit(5)'
```

If you need the low-level clone payload (including artefact ids and line ranges) for debugging, add `raw:true`:

```bash
bitloops devql query 'repo("bitloops")->artefacts(kind:"function")->clones(min_score:0.6,raw:true)->limit(5)'
```

## Useful Commands

General profile inspection:

```bash
bitloops embeddings doctor
```

Clear a local model cache:

```bash
bitloops embeddings clear-cache local
```

Pause and resume background enrichment work:

```bash
bitloops daemon enrichments pause
bitloops daemon enrichments resume
bitloops daemon enrichments retry-failed
```

## Troubleshooting

### `bitloops-embeddings` not found

Make sure the companion runtime is installed and on `PATH`:

```bash
which bitloops-embeddings
```

### Runtime handshake timeout on first local run

If the first local model startup is slow, raise the daemon timeouts:

```toml
[embeddings.runtime]
startup_timeout_secs = 120
request_timeout_secs = 120
```

### Enable or init succeeded, but embeddings setup failed

If Bitloops reports that core `enable` or `init` succeeded but embeddings setup failed, the command already rolled back only the new embeddings-related daemon-config changes from that invocation.

After fixing the local runtime, rerun one of:

```bash
bitloops enable --install-embeddings
bitloops daemon enable --install-embeddings
bitloops embeddings pull local
```

### Semantic summaries stay on deterministic fallback

That means semantic is enabled, but no semantic provider is configured, or the provider is disabled or unavailable. `bitloops devql packs --with-health` should show this clearly.
