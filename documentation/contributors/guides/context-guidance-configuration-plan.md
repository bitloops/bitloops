# Context Guidance Configuration Plan

## Goal

Add first-class CLI configuration support for the context guidance capability so it can be selected the same way summaries and code embeddings are selected:

- interactively in terminal flows
- non-interactively through flags
- written into `bitloops.toml`
- respected by daemon/runtime configuration

The manually working target configuration is:

```toml
[context_guidance.inference]
guidance_generation = "guidance_llm"

[inference.profiles.guidance_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "bitloops_platform_chat"
model = "ministral-3-3b-instruct"
api_key = "${BITLOOPS_PLATFORM_GATEWAY_TOKEN}"
temperature = "0.1"
max_output_tokens = 4096
```

## Current State

Context guidance already has most runtime plumbing. The missing work is primarily CLI setup, docs, and tests.

Existing support:

- `bitloops/src/config/types.rs` defines:
  - `ContextGuidanceInferenceBindings`
  - `ContextGuidanceConfig`
  - `guidance_generation`
- `bitloops/src/config/resolve.rs` already resolves:
  - `[context_guidance.inference].guidance_generation`
  - `BITLOOPS_CONTEXT_GUIDANCE_GUIDANCE_GENERATION`
  - empty, `off`, and `disabled` values as no binding
- `bitloops/src/config/daemon_config/file.rs` accepts `context_guidance` as a top-level daemon config section.
- `bitloops/src/config/unified_consumer_tests.rs` has a unified config test for `context_guidance` plus inference profile wiring.
- `bitloops/src/capability_packs/context_guidance/descriptor.rs` declares:
  - `CONTEXT_GUIDANCE_TEXT_GENERATION_SLOT = "guidance_generation"`
  - a text generation inference slot
- `bitloops/src/host/capability_host/runtime_contexts/local_resources.rs` includes context guidance when building slot bindings.
- `bitloops/src/capability_packs/context_guidance/health.rs` validates the text generation slot.
- `bitloops/src/capability_packs/context_guidance/ingesters/history.rs` and `knowledge.rs` already use text generation at runtime.
- `bitloops/src/daemon/enrichment/workplane/job_claim.rs` can claim generic text-generation jobs.
- `bitloops/src/daemon/enrichment_tests.rs` already covers generic context guidance text-generation job claiming and the unconfigured case.

Known gap:

- `documentation/docs/reference/configuration.md` omits `context_guidance` from the documented accepted top-level daemon sections, even though the parser accepts it.

## Missing Pieces

- No CLI setup writer for `[context_guidance.inference]`.
- No prompt flow for selecting context guidance generation.
- No `bitloops init` flags for context guidance.
- No `bitloops enable` flags for context guidance.
- No `context_guidance_generation_configured` helper.
- No CLI tests for parsing, prompt behavior, or TOML writing.
- No user-facing docs for context guidance setup flags or manual TOML.

## Recommended Design

Treat context guidance as another text-generation setup target, parallel to summaries.

Summaries currently bind:

```toml
[semantic_clones.inference]
summary_generation = "..."
```

Context guidance should bind:

```toml
[context_guidance.inference]
guidance_generation = "..."
```

Embeddings remain a separate inference task family. Context guidance should reuse summary-style text-generation profile creation, not embedding setup internals.

## Implementation Plan

### 1. Add Context Guidance Setup Primitives

Create a new setup module, likely:

```text
bitloops/src/cli/inference/setup/guidance.rs
```

Export it from the existing setup module.

Suggested constants:

```rust
const DEFAULT_PLATFORM_GUIDANCE_PROFILE_NAME: &str = "guidance_llm";
const DEFAULT_LOCAL_GUIDANCE_PROFILE_NAME: &str = "guidance_local";
const DEFAULT_GUIDANCE_MAX_OUTPUT_TOKENS: i64 = 4096;
```

Suggested functions:

```rust
pub fn context_guidance_generation_configured(repo_root: &Path) -> bool;

pub fn write_platform_context_guidance_profile(
    repo_root: &Path,
    gateway_url_override: Option<&str>,
    api_key_env: &str,
) -> Result<()>;

pub fn write_local_context_guidance_profile(
    repo_root: &Path,
    model_name: &str,
) -> Result<()>;

pub async fn configure_cloud_context_guidance(...) -> Result<()>;

pub async fn configure_local_context_guidance(...) -> Result<()>;
```

The configured check should mirror `summary_generation_configured`, but without the summary-mode guard. It should require:

- `[context_guidance.inference].guidance_generation` exists
- the referenced inference profile exists
- profile task is `text_generation`
- required profile fields are present for the selected driver/runtime

### 2. Share Text-Generation Profile Writing

Avoid duplicating summary setup code. Extract a low-level helper that writes a text-generation binding for any capability section.

Suggested helper shape:

```rust
fn update_text_generation_binding(
    doc: &mut DocumentMut,
    capability_table: &str,
    inference_key: &str,
    profile_name: &str,
) {
    let capability = ensure_table(doc, capability_table);
    let inference = ensure_child_table(capability, "inference");
    inference[inference_key] = Item::Value(profile_name.into());
}
```

Summary setup would call:

```rust
update_text_generation_binding(
    &mut doc,
    "semantic_clones",
    "summary_generation",
    profile_name,
);
```

Context guidance setup would call:

```rust
update_text_generation_binding(
    &mut doc,
    "context_guidance",
    "guidance_generation",
    profile_name,
);
```

Keep existing public summary setup functions as wrappers so call sites do not need broad churn.

The context guidance cloud profile should match the manual config:

```toml
[inference.profiles.guidance_llm]
task = "text_generation"
runtime = "bitloops_inference"
driver = "bitloops_platform_chat"
model = "ministral-3-3b-instruct"
api_key = "${BITLOOPS_PLATFORM_GATEWAY_TOKEN}"
temperature = "0.1"
max_output_tokens = 4096
```

If a gateway URL override is supplied, write the same field name used by summary cloud profiles.

The local profile should use the existing local chat driver pattern used by summaries:

```toml
[inference.profiles.guidance_local]
task = "text_generation"
runtime = "bitloops_inference"
driver = "ollama_chat"
model = "<selected-ollama-model>"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 4096
```

Use the same profile name collision behavior as summaries. If a non-managed `guidance_llm` or `guidance_local` already exists, preserve it and choose a suffixed profile name.

### 3. Add Interactive Prompt

Add a context-specific terminal prompt modeled on the summary prompt.

Suggested title:

```text
Configure context guidance
```

Suggested intro:

```text
Context guidance distills captured sessions and linked knowledge into repo-specific guidance facts.
```

Options:

```text
Skip for now (recommended)
Bitloops Cloud (limited availability)
Local (Ollama)
```

Behavior:

- Default selection should be skip.
- Bitloops Cloud should use `guidance_llm`.
- Local should use `guidance_local` and prompt for or auto-detect an Ollama model the same way summaries do.
- Preserve text-input fallback for non-interactive or limited terminal environments.

### 4. Add `bitloops init` Flags

Extend `bitloops/src/cli/init/args.rs`.

Suggested flags:

```text
--context-guidance-runtime <platform|local>
--no-context-guidance
--context-guidance-gateway-url <url>
--context-guidance-api-key-env <env>
```

Defaults:

```text
--context-guidance-api-key-env BITLOOPS_PLATFORM_GATEWAY_TOKEN
```

Validation:

- `--no-context-guidance` conflicts with `--context-guidance-runtime`.
- `--no-context-guidance` conflicts with `--context-guidance-gateway-url`.
- `--no-context-guidance` conflicts with `--context-guidance-api-key-env`.
- `--context-guidance-gateway-url` only applies to platform/cloud runtime.
- `--context-guidance-api-key-env` only applies to platform/cloud runtime.

Workflow changes in `bitloops/src/cli/init/workflow.rs`:

- Plain `bitloops init` should keep current behavior and skip optional setup unless explicitly requested.
- `bitloops init --install-default-daemon` should prompt for context guidance if it is not already configured and the user did not explicitly skip or select it.
- Explicit `--context-guidance-runtime platform` should write the cloud profile without prompting.
- Explicit `--context-guidance-runtime local` should configure local Ollama guidance without prompting except for required model selection if not supplied by existing local selection mechanics.
- If cloud embeddings, summaries, and context guidance are all requested, combine them into the existing login-required path so the user logs in once.

Initial v1 recommendation:

- Configure context guidance synchronously before daemon bootstrap progress, because there is no separate repo-wide context guidance backfill lane equivalent to summary bootstrap.

Possible future v2:

- Generalize `SummaryBootstrap` into a `TextGenerationBootstrap` with a target enum such as `SemanticSummaries` and `ContextGuidance`.
- Only do this if product requirements need daemon-owned progress for initial context guidance generation.

### 5. Add `bitloops enable` Flags

Extend `bitloops/src/cli/enable.rs`.

Suggested flags:

```text
--install-context-guidance
--context-guidance-runtime <platform|local>
--context-guidance-gateway-url <url>
--context-guidance-api-key-env <env>
```

Validation:

- Context guidance setup flags should require `--capture`, matching the current pattern for enable-time optional setup.
- `--context-guidance-runtime` should default to platform when `--install-context-guidance` is present and no runtime is supplied, if that matches the current summaries behavior.
- Gateway URL and API key env should only apply to platform/cloud runtime.

Workflow:

- When capture is enabled from disabled, prompt for summaries as today.
- Then prompt for context guidance if it is not configured.
- If `--install-context-guidance` is supplied, configure it directly and skip the prompt.
- If context guidance is already configured, do not overwrite it unless an explicit overwrite flag already exists for comparable setup flows.

### 6. Update Documentation

Update:

```text
documentation/docs/reference/configuration.md
```

Add:

- `context_guidance` to accepted top-level daemon config sections.
- Manual TOML example for cloud context guidance.
- Manual TOML example for local context guidance.
- New `bitloops init` flags.
- New `bitloops enable` flags.
- `BITLOOPS_CONTEXT_GUIDANCE_GUIDANCE_GENERATION` in the environment variable section if this page documents env overrides.

Also update any CLI guide that currently documents summary or embedding setup flags.

### 7. Add Tests

Add focused tests only. Do not run the entire suite unless explicitly requested.

Suggested setup tests in `bitloops/src/cli/inference/tests.rs`:

- platform context guidance writes `[context_guidance.inference] guidance_generation = "guidance_llm"`
- platform context guidance writes `max_output_tokens = 4096`
- platform context guidance writes the expected model and API key env placeholder
- local context guidance writes `driver = "ollama_chat"` and selected model
- `context_guidance_generation_configured` returns false for missing binding
- `context_guidance_generation_configured` returns false for missing profile
- `context_guidance_generation_configured` returns false for non-text-generation profile
- existing non-managed `guidance_llm` is preserved and a suffixed profile name is selected

Suggested init tests in `bitloops/src/cli/init/tests.rs`:

- parse `--context-guidance-runtime platform`
- parse `--context-guidance-runtime local`
- reject `--no-context-guidance --context-guidance-runtime platform`
- reject gateway URL with local runtime
- default prompt path skips context guidance
- explicit platform runtime writes context guidance config
- `--install-default-daemon` prompts when guidance is unconfigured
- cloud embeddings plus summaries plus context guidance trigger a single login path

Suggested enable tests in `bitloops/src/cli/enable_tests.rs`:

- parse `--install-context-guidance`
- reject context guidance setup flags without `--capture`
- capture enable prompts for context guidance when unconfigured
- capture enable does not prompt when context guidance is already configured
- explicit context guidance install writes the expected config

Runtime tests already exist for:

- context guidance slot binding
- health validation
- generic text-generation job claiming
- pending job behavior when generation is unconfigured

Keep those as safety net rather than duplicating runtime behavior in CLI tests.

## Suggested Verification Commands

For the implementation phase, prefer targeted tests:

```bash
cargo nextest run --manifest-path bitloops/Cargo.toml -E 'test(/context_guidance/) | test(/summary_setup/) | test(/init_.*context_guidance/) | test(/enable_.*context_guidance/)'
```

If CLI parsing or setup flow changes broadly, also run the repo's CLI-specific alias if available.

Do not run the full test suite unless requested.

## Open Decisions

1. Flag naming: use `platform` for consistency with existing embedding runtime flags, while user-facing prompt text should say `Bitloops Cloud`.
2. Cloud API key env: default to `BITLOOPS_PLATFORM_GATEWAY_TOKEN`, but allow an override if summaries already support that pattern.
3. Local model default: reuse summary local model discovery and selection instead of introducing a context-guidance-specific default.
4. Daemon bootstrap: start with synchronous config writing only. Add daemon-owned context guidance bootstrap later only if the product needs visible initial generation progress.

## Minimal Code Change Set

Expected files to touch:

```text
bitloops/src/cli/inference/setup.rs
bitloops/src/cli/inference/setup/guidance.rs
bitloops/src/cli/inference/setup/summaries.rs
bitloops/src/cli/inference/tests.rs
bitloops/src/cli/init/args.rs
bitloops/src/cli/init/workflow.rs
bitloops/src/cli/init/tests.rs
bitloops/src/cli/enable.rs
bitloops/src/cli/enable_tests.rs
documentation/docs/reference/configuration.md
```

Potential files if helper types are centralized:

```text
bitloops/src/cli/inference/setup/text_generation.rs
bitloops/src/cli/inference/setup/prompts.rs
```

## Acceptance Criteria

- Users can configure cloud context guidance during `bitloops init --install-default-daemon`.
- Users can configure local context guidance during `bitloops init --install-default-daemon`.
- Users can configure context guidance non-interactively through init flags.
- Users can configure context guidance when enabling capture through `bitloops enable --capture`.
- Existing summary and embedding setup behavior remains unchanged.
- Generated TOML matches the runtime-resolved config shape already supported by the daemon.
- Existing user-authored inference profiles are not overwritten accidentally.
- Documentation includes both flags and manual TOML configuration.
