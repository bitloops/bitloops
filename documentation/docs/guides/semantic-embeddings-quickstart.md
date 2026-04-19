---
title: Semantic + Embeddings Quickstart
---

# Semantic + Embeddings Quickstart

This guide shows the quickest way to try:

- semantic summaries through a configured text-generation inference profile
- local embeddings through the standalone `bitloops-local-embeddings` binary over stdio IPC
- hosted embeddings through the standalone `bitloops-platform-embeddings` binary over stdio IPC
- the daemon-owned enrichment queue and health checks

Run the repo-scoped commands below from inside a Git repository or Bitloops project, typically after `bitloops init`.

## What You Need

- `bitloops`
- either the Bitloops-managed embeddings install flow or a manually installed `bitloops-local-embeddings` / `bitloops-platform-embeddings` binary
- a text-generation provider API key if you want LLM semantic summaries

In a source checkout, build and install `bitloops`. For the default local Bitloops-managed runtime, explicit setup flows such as `bitloops init --install-default-daemon`, `bitloops enable --install-embeddings`, and `bitloops embeddings install` can install the standalone `bitloops-local-embeddings` binary for you. If you are wiring a custom runtime manually, install the matching standalone binary from the `bitloops/bitloops-embeddings` GitHub releases for your platform:

```bash
cargo build
cargo dev-install
```

On macOS, prefer `cargo dev-install` over plain `cargo install --path bitloops --force` so the
DuckDB runtime is staged correctly for the installed `bitloops` binary.

No Python interpreter is required for the embeddings runtime binary.

## Fastest Setup Paths

Bitloops can now set up embeddings for you without manual `config.toml` edits.

If you are bootstrapping a repo and want `init` to install the default daemon first:

```bash
bitloops init --install-default-daemon --sync=true
```

When embeddings are not already configured, `bitloops init --install-default-daemon`:

- bootstraps the default daemon config if needed
- in interactive terminals, asks whether to use Bitloops cloud, the local runtime, or skip embeddings for now
- recommends Bitloops cloud in that prompt
- configures the selected embeddings runtime before init-triggered sync, except for the local managed runtime bootstrap which still downloads and warms asynchronously

If the repo is already initialised and you just want to add embeddings:

```bash
bitloops enable --install-embeddings
bitloops daemon enable --install-embeddings
```

Interactive `bitloops enable` also asks whether to install embeddings when they are not already configured. The prompt uses `[Y/n]`, so pressing `Enter` accepts the recommended setup.

If you want the hosted gateway runtime instead of the default local runtime:

```bash
bitloops embeddings install --runtime platform --gateway-url https://gateway.example/v1/embeddings
```

`bitloops init` and `bitloops enable` accept the same hosted mode through `--embeddings-runtime platform`. Add `--embeddings-gateway-url https://gateway.example/v1/embeddings` or set `BITLOOPS_PLATFORM_GATEWAY_URL` only when you want to override the platform default endpoint. The managed platform runtime reads its bearer token from `BITLOOPS_PLATFORM_GATEWAY_TOKEN` by default; override that with `--embeddings-api-key-env`.

## Config Location And Targeting

Inference runtimes, profiles, and semantic-clones slot bindings live in the Bitloops daemon config.

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
[semantic_clones.inference]
code_embeddings = "local_code"
summary_embeddings = "local_code"

[inference.runtimes.bitloops_local_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-local-embeddings/bitloops-local-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "bge-m3"
```

Notes:

- `local_code` is the default auto-created local embeddings profile name.
- `bitloops_embeddings_ipc` is the default auto-created local embeddings driver.
- `bge-m3` is the default auto-created local embeddings model.
- When Bitloops installs the managed runtime, it writes an absolute path under the Bitloops data directory, as shown above.
- Use `command = "bitloops-local-embeddings"` only when you are managing that standalone binary yourself on `PATH`.
- If an active embedding profile already exists, Bitloops does not overwrite it.
- If that existing active profile is local, Bitloops still runs the normal warm/bootstrap path for it.
- If that existing active profile is hosted or otherwise non-local, Bitloops treats embeddings as already enabled and skips local runtime bootstrap.

## Hosted Platform Embeddings Config

The hosted gateway path writes a separate runtime and profile:

```toml
[semantic_clones.inference]
code_embeddings = "platform_code"
summary_embeddings = "platform_code"

[inference.runtimes.bitloops_platform_embeddings]
command = "/Users/alex/Library/Application Support/bitloops/tools/bitloops-platform-embeddings/bitloops-platform-embeddings"
args = ["--gateway-url", "https://gateway.example/v1/embeddings", "--api-key-env", "BITLOOPS_PLATFORM_GATEWAY_TOKEN"]
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.platform_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_platform_embeddings"
model = "bge-m3"
```

The hosted runtime keeps the same daemon-facing IPC contract as the local runtime. Only the implementation behind the runtime command changes.

## Optional Semantic Summaries

If you also want semantic summaries, bind the `summary_generation` slot to a text-generation profile:

```toml
[semantic_clones.inference]
summary_generation = "summary_llm"

[inference.runtimes.bitloops_inference]
command = "bitloops-inference"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.summary_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "openai_chat_completions"
model = "gpt-5.4-mini"
api_key = "${OPENAI_API_KEY}"
base_url = "https://api.openai.com/v1/chat/completions"
temperature = "0.1"
max_output_tokens = 200
```

Notes:

- `summary_generation` is optional when `summary_mode = "auto"`. If it is unset or unavailable, Bitloops falls back to deterministic summaries.
- `task = "text_generation"` profiles must declare `runtime`, `temperature`, and `max_output_tokens`, and `driver` is interpreted by `bitloops-inference`.
- `bitloops inference install` installs or repairs the managed summary runtime. Interactive `bitloops enable` and `bitloops init --install-default-daemon` can bind summaries to a local Ollama model automatically when it is available, using `http://127.0.0.1:11434/api/chat`.
- `code_embeddings` and `summary_embeddings` can point at the same embeddings profile or at different ones.
- For platform-specific config paths, use the configuration reference alongside your OS defaults.

## Warm The Local Model

Run:

```bash
bitloops embeddings pull local_code
```

What this does:

- starts the standalone `bitloops-local-embeddings` binary through the host inference runtime
- validates the selected profile and runtime binding
- warms the local model cache
- downloads the model if it is missing

This is the best first check that the configured embeddings runtime command works.

`bitloops enable --install-embeddings` and `bitloops init --install-default-daemon` reuse this same warm/bootstrap path automatically; `bitloops embeddings pull local_code` remains useful when you want to rerun it explicitly.

## Verify Health

Run:

```bash
bitloops devql packs --with-health
```

Healthy output should show:

- summary generation slot resolved when configured
- code and summary embedding slots resolved when configured
- runtime command available
- IPC handshake succeeded

If embeddings are disabled, health should report that explicitly instead of failing.

## Run Ingest

Run:

```bash
bitloops devql tasks enqueue --kind ingest
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
bitloops embeddings clear-cache local_code
```

Pause and resume background enrichment work:

```bash
bitloops daemon enrichments pause
bitloops daemon enrichments resume
bitloops daemon enrichments retry-failed
```

## Troubleshooting

### `bitloops-local-embeddings` not found

If you are using the Bitloops-managed runtime, run:

```bash
bitloops embeddings install
bitloops embeddings doctor
```

If you are managing the runtime yourself, make sure the standalone binary is installed and on `PATH`:

```bash
which bitloops-local-embeddings
```

### Runtime handshake timeout on first local run

If the first local model startup is slow, raise the inference runtime timeouts:

```toml
[inference.runtimes.bitloops_local_embeddings]
startup_timeout_secs = 120
request_timeout_secs = 120
```

### Enable or init succeeded, but embeddings setup failed

If Bitloops reports that core `enable` or `init` succeeded but embeddings setup failed, the command already rolled back only the new embeddings-related daemon-config changes from that invocation.

After fixing the local runtime, rerun one of:

```bash
bitloops enable --install-embeddings
bitloops daemon enable --install-embeddings
bitloops embeddings pull local_code
```

### Semantic summaries stay on deterministic fallback

That means `summary_mode = "auto"` is enabled, but no `summary_generation` profile is bound, or the bound text-generation profile is unavailable. `bitloops devql packs --with-health` should show this clearly.
