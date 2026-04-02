# Semantic + Embeddings Quickstart

This guide shows the quickest way to try:

- semantic summaries through an OpenAI-compatible provider
- local embeddings through the standalone `bitloops-embeddings` runtime
- the daemon-owned enrichment queue and health checks

## What You Need

- `bitloops`
- `bitloops-embeddings`
- a semantic provider API key if you want LLM semantic summaries

In a packaged release, both CLIs should be installed together. In a source checkout, build and install both:

```bash
cargo build
cargo install --path bitloops --force
cargo install --path bitloops-embeddings --force
```

## Config Location

Semantic and embeddings runtime settings live in the Bitloops daemon config.

Typical macOS path:

```text
~/Library/Application Support/bitloops/config.toml
```

If you have not bootstrapped the daemon config yet:

```bash
bitloops daemon start --create-default-config
```

## Local Embeddings + OpenAI Semantic

Add this to the daemon config:

```toml
[semantic]
provider = "openai"
model = "gpt-5.4-mini"
api_key = "${OPENAI_API_KEY}"

[semantic_clones]
embedding_profile = "local"

[embeddings.profiles.local]
kind = "local_fastembed"
```

Notes:

- `embedding_profile` is just a profile name. `local` is a convention, not a reserved keyword.
- `kind = "local_fastembed"` uses the local embeddings runtime with the Jina model by default.

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
bitloops devql query 'artefacts(kind:"function")->clones(relation_kind:"similar_implementation",min_score:0.6)->limit(5)'
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

### Semantic summaries stay on deterministic fallback

That means semantic is enabled, but no semantic provider is configured, or the provider is disabled or unavailable. `bitloops devql packs --with-health` should show this clearly.
