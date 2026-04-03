# QAT as Integration Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

## Summary

The QAT (Quality Acceptance Test) suite is currently compiled into the production binary behind a `#[cfg(feature = "qat")]` feature gate and invoked via a `qat` CLI subcommand. This means test-only code (Cucumber harness, step definitions, helpers) ships inside `src/` and bloats the production artifact when the feature is enabled.

This plan moves the entire QAT module out of `src/qat/` and into `tests/qat_support/` + `tests/qat_acceptance.rs`, making it a standard Cargo integration test. The result:

- **Production binary is clean** — zero QAT code, no feature flag, no `qat` subcommand.
- **Tests exercise the real binary** — resolved via `CARGO_BIN_EXE_bitloops` (local) or `BITLOOPS_QAT_BINARY` env var (CI).
- **Standard invocation** — `cargo test --test qat_acceptance qat_smoke -- --ignored` instead of `cargo run --features qat -- qat --smoke`.
- **Default `cargo test` is unaffected** — all QAT tests are `#[ignore]` so they only run when explicitly requested.

The migration is mechanical: copy 14 files, rewrite 3 `crate::` imports to `bitloops::`, rewrite 4 `crate::qat::` paths to `crate::qat_support::`, restructure `runner.rs` to drop the CLI args wrapper, then delete the old module and all feature-flag references.

**Goal:** Move QAT from a feature-gated module inside `src/` to a standard Cargo integration test under `tests/`, so the production binary contains zero QAT code and the test harness exercises the real shipped artifact.

**Architecture:** The entire `src/qat/` module moves to `tests/qat_support/` (helpers, steps, world) plus `tests/qat_acceptance.rs` (entry point). All `crate::` imports become `bitloops::` imports. The `qat` Cargo feature flag and all `#[cfg(feature = "qat")]` guards are removed from `src/`. The binary path is resolved via `BITLOOPS_QAT_BINARY` env var (CI) or `CARGO_BIN_EXE_bitloops` (local). The CLI `qat` subcommand is removed.

**Tech Stack:** Rust, Cucumber 0.22.1, Cargo integration tests, `CARGO_BIN_EXE_bitloops`

---

## Execution plan — phases and parallelization

This section maps the implementation into phases with explicit dependency ordering. Tasks marked **⚡ PARALLEL** can be dispatched as concurrent sub-agents within their phase.

### Phase 1: Scaffold the new test module (Task 1 + Task 2)

These two tasks produce the new `tests/` code. Task 2 depends on Task 1's directory structure existing, but most file-creation work within Task 1 is independent.

| # | Work item | Depends on | Agent |
|---|-----------|------------|-------|
| 1.1 | Create directory structure (`tests/qat_support/helpers/`, `tests/qat_support/steps/`) | — | Setup |
| 1.2 | Create `tests/qat_support/mod.rs` | 1.1 | ⚡ Agent A |
| 1.3 | Copy unchanged files: `world.rs`, `helpers/core.rs`, `helpers/deps_and_testlens.rs`, `helpers/semantic_clones.rs`, `helpers/knowledge.rs`, `helpers/internals.rs`, `helpers/tests.rs` | 1.1 | ⚡ Agent A |
| 1.4 | Create adapted `tests/qat_support/helpers/mod.rs` (`crate::` → `bitloops::`) | 1.1 | ⚡ Agent B |
| 1.5 | Create adapted `tests/qat_support/runner.rs` (remove `clap::Args`, accept `binary_path`) | 1.1 | ⚡ Agent B |
| 1.6 | Create adapted `tests/qat_support/steps/mod.rs` (`crate::qat::` → `crate::qat_support::`) | 1.1 | ⚡ Agent C |
| 1.7 | Create adapted `tests/qat_support/steps/common.rs` | 1.1 | ⚡ Agent C |
| 1.8 | Create adapted `tests/qat_support/steps/given.rs` | 1.1 | ⚡ Agent C |
| 1.9 | Create adapted `tests/qat_support/steps/then.rs` | 1.1 | ⚡ Agent C |
| 1.10 | Create `tests/qat_acceptance.rs` (entry point with 3 `#[ignore]` test fns) | 1.2–1.9 | Sequential |

**Parallelization:** After 1.1, items 1.2–1.9 split into 3 parallel agents:
- **Agent A** — `mod.rs` + all unchanged file copies (trivial, fast)
- **Agent B** — adapted `helpers/mod.rs` + `runner.rs` (import rewriting + structural changes)
- **Agent C** — all 4 steps files (import rewriting, same pattern)

Then 1.10 (entry point) is written after all modules exist.

**Gate:** `cargo test --test qat_acceptance --no-run` must compile before proceeding.

---

### Phase 2: Compile-check and smoke-test the new integration test

This phase validates the new test code works before touching production code.

| # | Work item | Depends on | Agent |
|---|-----------|------------|-------|
| 2.1 | Run `cargo test --test qat_acceptance --no-run` — must compile | Phase 1 | Sequential |
| 2.2 | Run `cargo test --test qat_acceptance qat_smoke -- --ignored` — 2 scenarios must pass | 2.1 | Sequential |
| 2.3 | Commit all new test files | 2.2 | Sequential |

**Parallelization:** None — these are sequential verification gates.

**Gate:** Smoke suite passes before proceeding to remove old code.

---

### Phase 3: Remove QAT from the production crate (Task 3)

Remove all QAT code and feature-flag references from `src/`, `Cargo.toml`, and CLI. The individual edits are independent of each other but must all land before verification.

| # | Work item | Depends on | Agent |
|---|-----------|------------|-------|
| 3.1 | Delete `src/qat/` directory (14 files) | Phase 2 | ⚡ Agent D |
| 3.2 | Remove `#[cfg(feature = "qat")] pub mod qat;` from `src/lib.rs` | Phase 2 | ⚡ Agent E |
| 3.3 | Remove `#[cfg(feature = "qat")] pub use bitloops::qat;` from `src/main.rs` | Phase 2 | ⚡ Agent E |
| 3.4 | Remove `Qat` variant + dispatch arm from `src/cli.rs` (2 blocks) | Phase 2 | ⚡ Agent E |
| 3.5 | Remove `"qat"` command name mapping from `src/cli/root.rs` | Phase 2 | ⚡ Agent E |
| 3.6 | Remove `qat = []` feature line from `Cargo.toml` | Phase 2 | ⚡ Agent E |
| 3.7 | Run `cargo check` — production crate compiles cleanly | 3.1–3.6 | Sequential |
| 3.8 | Run `cargo test --test qat_acceptance --no-run` — integration test still compiles | 3.7 | Sequential |
| 3.9 | Commit removals | 3.8 | Sequential |

**Parallelization:** Items 3.1–3.6 split into 2 parallel agents:
- **Agent D** — delete `src/qat/` directory
- **Agent E** — all 5 source-file edits (`lib.rs`, `main.rs`, `cli.rs`, `cli/root.rs`, `Cargo.toml`)

These are small edits, so a single agent for all 5 files is efficient. The directory deletion is separate to avoid race conditions.

**Gate:** Both `cargo check` and `cargo test --test qat_acceptance --no-run` must pass.

---

### Phase 4: Update documentation (Task 4)

| # | Work item | Depends on | Agent |
|---|-----------|------------|-------|
| 4.1 | Replace `TESTING.md` with new table (add QAT commands, `BITLOOPS_QAT_BINARY` docs) | Phase 3 | ⚡ Agent F |
| 4.2 | Update `qat/README.md` — replace all old invocation commands with `cargo test` equivalents | Phase 3 | ⚡ Agent G |
| 4.3 | Commit documentation changes | 4.1–4.2 | Sequential |

**Parallelization:** `TESTING.md` and `qat/README.md` are independent — 2 parallel agents.

---

### Phase 5: End-to-end verification (Task 5)

| # | Work item | Depends on | Agent |
|---|-----------|------------|-------|
| 5.1 | `cargo check` — clean production build | Phase 4 | Sequential |
| 5.2 | `cargo test --test qat_acceptance --no-run` — integration test compiles | 5.1 | Sequential |
| 5.3 | `cargo test --test qat_acceptance qat_smoke -- --ignored` — smoke passes | 5.2 | Sequential |
| 5.4 | `cargo test --test qat_acceptance qat_devql -- --ignored` — DevQL passes | 5.3 | Sequential |
| 5.5 | `cargo test` (default) — verify QAT tests are NOT run (all `#[ignore]`) | 5.4 | Sequential |
| 5.6 | Final fixup commit if needed | 5.5 | Sequential |

**Parallelization:** None — this is a sequential verification chain. Each step confirms correctness before the next.

---

### Parallelization summary

```
Phase 1 (scaffold)
  1.1 (dirs)
    ├── Agent A: 1.2, 1.3   (mod.rs + unchanged copies)
    ├── Agent B: 1.4, 1.5   (adapted helpers/mod.rs + runner.rs)
    └── Agent C: 1.6–1.9    (adapted steps/*)
  1.10 (entry point)         — after A+B+C complete

Phase 2 (verify new tests)   — sequential gate

Phase 3 (remove old code)
    ├── Agent D: 3.1         (delete src/qat/)
    └── Agent E: 3.2–3.6    (edit 5 source files)
  3.7–3.9                    — sequential gate

Phase 4 (docs)
    ├── Agent F: 4.1         (TESTING.md)
    └── Agent G: 4.2         (qat/README.md)
  4.3                        — sequential gate

Phase 5 (E2E verify)         — fully sequential
```

**Total parallel agent slots:** 3 (Phase 1) + 2 (Phase 3) + 2 (Phase 4) = **7 agent dispatches across 3 parallel windows**

---

## Import rewriting reference

Every `crate::` and `super::` path in the QAT module must be rewritten when moving to `tests/`. Here is the complete mapping:

| File | Old import | New import |
|------|-----------|------------|
| `helpers/mod.rs` | `use super::world::QatWorld` | `use super::world::QatWorld` (unchanged — still relative) |
| `helpers/mod.rs` | `use crate::adapters::agents::AGENT_NAME_CLAUDE_CODE` | `use bitloops::adapters::agents::AGENT_NAME_CLAUDE_CODE` |
| `helpers/mod.rs` | `use crate::host::checkpoints::session::create_session_backend_or_local` | `use bitloops::host::checkpoints::session::create_session_backend_or_local` |
| `helpers/mod.rs` | `use crate::host::checkpoints::strategy::manual_commit::{...}` | `use bitloops::host::checkpoints::strategy::manual_commit::{...}` |
| `steps/common.rs` | `use crate::qat::world::QatWorld` | `use crate::qat_support::world::QatWorld` |
| `steps/given.rs` | `use crate::qat::helpers` | `use crate::qat_support::helpers` |
| `steps/given.rs` | `use crate::qat::world::QatWorld` | `use crate::qat_support::world::QatWorld` |
| `steps/then.rs` | `use crate::qat::helpers` | `use crate::qat_support::helpers` |
| `steps/then.rs` | `use crate::qat::world::QatWorld` | `use crate::qat_support::world::QatWorld` |
| `steps/mod.rs` | `use crate::qat::world::QatWorld` | `use crate::qat_support::world::QatWorld` |
| `runner.rs` | `use super::helpers::sanitize_name` | `use super::helpers::sanitize_name` (unchanged — still relative) |
| `runner.rs` | `use super::steps` | `use super::steps` (unchanged) |
| `runner.rs` | `use super::world::{QatRunConfig, QatWorld}` | `use super::world::{QatRunConfig, QatWorld}` (unchanged) |
| `helpers/tests.rs` | `use super::*` | `use super::*` (unchanged) |

**Note on `crate::` in integration tests:** In Cargo integration tests, `crate` refers to the test crate itself (not the library). So `crate::qat_support::...` resolves to the test's own `qat_support` module.

## File structure

| Action | File | Responsibility |
|--------|------|---------------|
| **Create** | `tests/qat_acceptance.rs` | Integration test entry point — 3 test functions + runner logic |
| **Create** | `tests/qat_support/mod.rs` | Module root (replaces `src/qat/mod.rs`) |
| **Create** | `tests/qat_support/world.rs` | Copied from `src/qat/world.rs` (no changes needed) |
| **Create** | `tests/qat_support/runner.rs` | Adapted from `src/qat/runner.rs` — remove `clap::Args`, accept binary path |
| **Create** | `tests/qat_support/helpers/mod.rs` | Adapted from `src/qat/helpers/mod.rs` — `crate::` → `bitloops::` |
| **Create** | `tests/qat_support/helpers/core.rs` | Copied from `src/qat/helpers/core.rs` (no changes — no crate imports) |
| **Create** | `tests/qat_support/helpers/deps_and_testlens.rs` | Copied (no changes) |
| **Create** | `tests/qat_support/helpers/semantic_clones.rs` | Copied (no changes) |
| **Create** | `tests/qat_support/helpers/knowledge.rs` | Copied (no changes) |
| **Create** | `tests/qat_support/helpers/internals.rs` | Copied (no changes) |
| **Create** | `tests/qat_support/helpers/tests.rs` | Copied (no changes) |
| **Create** | `tests/qat_support/steps/mod.rs` | Adapted — `crate::qat::` → `crate::qat_support::` |
| **Create** | `tests/qat_support/steps/common.rs` | Adapted — `crate::qat::` → `crate::qat_support::` |
| **Create** | `tests/qat_support/steps/given.rs` | Adapted — `crate::qat::` → `crate::qat_support::` |
| **Create** | `tests/qat_support/steps/then.rs` | Adapted — `crate::qat::` → `crate::qat_support::` |
| **Delete** | `src/qat/` (entire directory) | Remove from production crate |
| **Modify** | `src/lib.rs:14-15` | Remove `#[cfg(feature = "qat")] pub mod qat;` |
| **Modify** | `src/main.rs:13-14` | Remove `#[cfg(feature = "qat")] pub use bitloops::qat;` |
| **Modify** | `src/cli.rs:117-119` | Remove `#[cfg(feature = "qat")] Qat(...)` variant |
| **Modify** | `src/cli.rs:207-208` | Remove `#[cfg(feature = "qat")] Commands::Qat(args) => ...` |
| **Modify** | `src/cli/root.rs:237-238` | Remove `#[cfg(feature = "qat")] ... "qat"` |
| **Modify** | `Cargo.toml:10` | Remove `qat = []` feature |
| **Modify** | `TESTING.md` | Document new QAT test commands |

---

### Task 1: Create the test module structure and copy files

Move the QAT module from `src/qat/` to `tests/qat_support/` with all necessary import rewrites.

**Files:**
- Create: `tests/qat_support/mod.rs`
- Create: `tests/qat_support/world.rs` (copy from `src/qat/world.rs` — no changes)
- Create: `tests/qat_support/runner.rs` (adapted from `src/qat/runner.rs`)
- Create: `tests/qat_support/helpers/mod.rs` (adapted — `crate::` → `bitloops::`)
- Create: `tests/qat_support/helpers/core.rs` (copy — no changes)
- Create: `tests/qat_support/helpers/deps_and_testlens.rs` (copy — no changes)
- Create: `tests/qat_support/helpers/semantic_clones.rs` (copy — no changes)
- Create: `tests/qat_support/helpers/knowledge.rs` (copy — no changes)
- Create: `tests/qat_support/helpers/internals.rs` (copy — no changes)
- Create: `tests/qat_support/helpers/tests.rs` (copy — no changes)
- Create: `tests/qat_support/steps/mod.rs` (adapted)
- Create: `tests/qat_support/steps/common.rs` (adapted)
- Create: `tests/qat_support/steps/given.rs` (adapted)
- Create: `tests/qat_support/steps/then.rs` (adapted)

- [x] **Step 1: Create directory structure**

```bash
mkdir -p tests/qat_support/helpers tests/qat_support/steps
```

- [x] **Step 2: Copy files that need NO changes**

These files have no `crate::` imports — only `super::` or no module imports at all:

```bash
cp src/qat/world.rs tests/qat_support/world.rs
cp src/qat/helpers/core.rs tests/qat_support/helpers/core.rs
cp src/qat/helpers/deps_and_testlens.rs tests/qat_support/helpers/deps_and_testlens.rs
cp src/qat/helpers/semantic_clones.rs tests/qat_support/helpers/semantic_clones.rs
cp src/qat/helpers/knowledge.rs tests/qat_support/helpers/knowledge.rs
cp src/qat/helpers/internals.rs tests/qat_support/helpers/internals.rs
cp src/qat/helpers/tests.rs tests/qat_support/helpers/tests.rs
```

- [x] **Step 3: Create `tests/qat_support/mod.rs`**

```rust
pub mod helpers;
pub mod runner;
pub mod steps;
pub mod world;
```

Note: no re-exports needed — the entry point (`qat_acceptance.rs`) will call `runner::run_suite()` directly.

- [x] **Step 4: Create `tests/qat_support/helpers/mod.rs`**

Copy from `src/qat/helpers/mod.rs` and change the 3 `crate::` imports to `bitloops::`:

```rust
use super::world::QatWorld;
use bitloops::adapters::agents::AGENT_NAME_CLAUDE_CODE;
use bitloops::host::checkpoints::session::create_session_backend_or_local;
use bitloops::host::checkpoints::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_committed,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration as StdDuration, Instant};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};
use uuid::Uuid;

pub const BITLOOPS_REPO_NAME: &str = "bitloops";
const DEFAULT_CLAUDE_CODE_COMMAND: &str =
    "claude --model haiku --permission-mode bypassPermissions -p";
const FIRST_CLAUDE_PROMPT: &str =
    "Remove the Vite example code from the project and replace it with a simple hello world page";
const SECOND_CLAUDE_PROMPT: &str = "Change the hello world color to blue";
const COMMAND_TIMEOUT_ENV: &str = "BITLOOPS_QAT_COMMAND_TIMEOUT_SECS";
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 180;
const CLAUDE_TIMEOUT_ENV: &str = "BITLOOPS_QAT_CLAUDE_TIMEOUT_SECS";
const DEFAULT_CLAUDE_TIMEOUT_SECS: u64 = 30;
const CLAUDE_AUTH_TIMEOUT_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS";
const DEFAULT_CLAUDE_AUTH_TIMEOUT_SECS: u64 = 300;
const CLAUDE_AUTH_STATUS_COMMAND_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_STATUS_CMD";
const DEFAULT_CLAUDE_AUTH_STATUS_COMMAND: &str = "claude auth status --json";
const CLAUDE_AUTH_LOGIN_COMMAND_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_LOGIN_CMD";
const DEFAULT_CLAUDE_AUTH_LOGIN_COMMAND: &str = "claude auth login --claudeai";
const CLAUDE_FALLBACK_MARKER: &str = ".qat-claude-fallback";
const SEMANTIC_CLONES_FALLBACK_MARKER: &str = ".qat-semantic-clones-fallback";
const KNOWLEDGE_FALLBACK_MARKER: &str = ".qat-knowledge-fallback";

#[derive(Debug, Serialize)]
struct RunMetadata<'a> {
    scenario_name: &'a str,
    scenario_slug: &'a str,
    flow_name: &'a str,
    run_dir: String,
    repo_dir: String,
    terminal_log: String,
    binary_path: String,
    created_at: String,
}

include!("core.rs");
include!("deps_and_testlens.rs");
include!("semantic_clones.rs");
include!("knowledge.rs");
include!("internals.rs");

#[cfg(test)]
mod tests;
```

- [x] **Step 5: Create `tests/qat_support/runner.rs`**

Adapted from `src/qat/runner.rs`. Remove `clap::Args` (not needed — the test entry point constructs args directly). Accept `binary_path` as a parameter. Remove the `run()` wrapper (no CLI subcommand).

```rust
use super::helpers::sanitize_name;
use super::steps;
use super::world::{QatRunConfig, QatWorld};
use anyhow::{Context, Result, bail};
use cucumber::{World as _, writer::Stats as _};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

/// Which QAT suite to run.
pub enum Suite {
    Smoke,
    Devql,
    ClaudeCode,
}

/// Run a QAT suite against the given `binary_path`.
pub async fn run_suite(binary_path: PathBuf, suite: Suite) -> Result<()> {
    let max_concurrent = resolve_max_concurrent_scenarios();
    let runs_root = resolve_runs_root()?;
    let suite_root = create_suite_root(&runs_root)?;
    let feature_path = suite_feature_path(&suite);

    fs::write(
        runs_root.join(".last-run"),
        format!("{}\n", suite_root.display()),
    )
    .with_context(|| format!("writing latest qat pointer in {}", runs_root.display()))?;

    println!(
        "Running Bitloops QAT features from {}",
        feature_path.display()
    );
    println!("Artifacts will be written to {}", suite_root.display());

    let config = Arc::new(QatRunConfig {
        binary_path,
        suite_root: suite_root.clone(),
    });

    let before_config = Arc::clone(&config);
    let result = QatWorld::cucumber()
        .steps(steps::collection())
        .max_concurrent_scenarios(max_concurrent)
        .before(move |_, _, scenario, world| {
            let config = Arc::clone(&before_config);
            Box::pin(async move {
                let slug = sanitize_name(&scenario.name);
                world.prepare(config, &scenario.name, slug);
            })
        })
        .fail_on_skipped()
        .with_default_cli()
        .run(feature_path)
        .await;

    if result.execution_has_failed() || result.parsing_errors() != 0 {
        bail!(
            "bitloops qat reported failures (parsing_errors={}, skipped_steps={})\nartifacts: {}",
            result.parsing_errors(),
            result.skipped_steps(),
            suite_root.display()
        );
    }

    println!("Bitloops QAT completed successfully.");
    println!("Artifacts: {}", suite_root.display());
    Ok(())
}

fn resolve_runs_root() -> Result<PathBuf> {
    Ok(env::current_dir()
        .context("resolving current directory for qat runs dir")?
        .join("target")
        .join("qat-runs"))
}

fn create_suite_root(runs_root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(runs_root)
        .with_context(|| format!("creating qat runs root {}", runs_root.display()))?;
    let timestamp = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("formatting qat suite timestamp")?
        .replace(':', "-");
    let suite_dir = runs_root.join(format!(
        "{}-{}",
        timestamp,
        &Uuid::new_v4().simple().to_string()[..8]
    ));
    fs::create_dir_all(&suite_dir)
        .with_context(|| format!("creating qat suite dir {}", suite_dir.display()))?;
    Ok(suite_dir)
}

fn feature_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("qat")
        .join("features")
}

fn suite_feature_path(suite: &Suite) -> PathBuf {
    let root = feature_root();
    match suite {
        Suite::Smoke => root.join("smoke"),
        Suite::Devql => root.join("devql"),
        Suite::ClaudeCode => root.join("claude-code"),
    }
}

fn resolve_max_concurrent_scenarios() -> usize {
    env::var("BITLOOPS_QAT_MAX_CONCURRENT_SCENARIOS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1)
}
```

- [x] **Step 6: Create `tests/qat_support/steps/mod.rs`**

Copy from `src/qat/steps/mod.rs`, replacing `crate::qat::` with `crate::qat_support::`:

The file starts with:
```rust
use crate::qat_support::world::QatWorld;
use cucumber::step::Collection;
```

The rest of the file (the `collection()` function with all `.given()` and `.then()` registrations) stays exactly the same.

- [x] **Step 7: Create `tests/qat_support/steps/common.rs`**

```rust
use crate::qat_support::world::QatWorld;
use cucumber::codegen::LocalBoxFuture;
use regex::Regex;

pub(super) fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|err| panic!("invalid step regex `{pattern}`: {err}"))
}

pub(super) fn step_fn(
    f: for<'a> fn(&'a mut QatWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()>,
) -> for<'a> fn(&'a mut QatWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()> {
    f
}

pub(super) fn run_step(step_name: &str, result: anyhow::Result<()>) {
    if let Err(err) = result {
        panic!("{step_name} failed: {err:#}");
    }
}
```

- [x] **Step 8: Create `tests/qat_support/steps/given.rs`**

Copy from `src/qat/steps/given.rs`, replacing the first two imports:

```rust
use crate::qat_support::helpers;
use crate::qat_support::world::QatWorld;
use cucumber::codegen::LocalBoxFuture;

use super::common::run_step;
```

The rest of the file (all `pub(super) fn given_*` functions) stays exactly the same.

- [x] **Step 9: Create `tests/qat_support/steps/then.rs`**

Copy from `src/qat/steps/then.rs`, replacing the first two imports:

```rust
use crate::qat_support::helpers;
use crate::qat_support::world::QatWorld;
use cucumber::codegen::LocalBoxFuture;

use super::common::run_step;
```

The rest of the file stays exactly the same.

- [x] **Step 10: Commit**

```bash
git add tests/qat_support/ tests/qat_acceptance.rs
git commit -m "feat: move QAT module to tests/qat_support as integration test"
```

---

### Task 2: Create the integration test entry point

**Files:**
- Create: `tests/qat_acceptance.rs`

- [x] **Step 1: Create `tests/qat_acceptance.rs`**

```rust
//! QAT acceptance tests — run the Bitloops QAT Cucumber suites as standard
//! Cargo integration tests.
//!
//! The binary under test is resolved via:
//! 1. `BITLOOPS_QAT_BINARY` env var (CI: point at a default-feature release build)
//! 2. `CARGO_BIN_EXE_bitloops` (local dev: the binary built alongside this test)
//!
//! # Running
//!
//! ```bash
//! # All three suites:
//! cargo test --test qat_acceptance -- --ignored
//!
//! # Just smoke:
//! cargo test --test qat_acceptance qat_smoke -- --ignored
//!
//! # Against a separate release build (CI):
//! BITLOOPS_QAT_BINARY=target/release/bitloops \
//!   cargo test --test qat_acceptance -- --ignored
//! ```

mod qat_support;

use std::path::PathBuf;

use qat_support::runner::{self, Suite};

fn resolve_binary() -> PathBuf {
    if let Ok(p) = std::env::var("BITLOOPS_QAT_BINARY") {
        let path = PathBuf::from(p);
        assert!(
            path.exists(),
            "BITLOOPS_QAT_BINARY points to {}, which does not exist",
            path.display()
        );
        return path;
    }
    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT smoke suite; use `cargo test --test qat_acceptance qat_smoke -- --ignored`"]
async fn qat_smoke() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Smoke)
        .await
        .expect("QAT smoke suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT DevQL suite; use `cargo test --test qat_acceptance qat_devql -- --ignored`"]
async fn qat_devql() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::Devql)
        .await
        .expect("QAT DevQL suite failed");
}

#[tokio::test]
#[ignore = "slow E2E: runs QAT Claude Code suite; use `cargo test --test qat_acceptance qat_claude_code -- --ignored`"]
async fn qat_claude_code() {
    let binary = resolve_binary();
    runner::run_suite(binary, Suite::ClaudeCode)
        .await
        .expect("QAT Claude Code suite failed");
}
```

- [x] **Step 2: Verify compilation**

Run: `cargo test --test qat_acceptance --no-run`
Expected: Compiles successfully without running tests. No `--features qat` needed.

- [x] **Step 3: Run the smoke suite to verify it works**

Run: `cargo test --test qat_acceptance qat_smoke -- --ignored`
Expected: Smoke suite passes (2 scenarios).

- [x] **Step 4: Commit**

```bash
git add tests/qat_acceptance.rs
git commit -m "feat: add QAT integration test entry point"
```

---

### Task 3: Remove QAT from production crate

Remove all QAT code and feature flag references from `src/`, `Cargo.toml`, `main.rs`, `lib.rs`, and `cli.rs`.

**Files:**
- Delete: `src/qat/` (entire directory — 14 files)
- Modify: `src/lib.rs:14-15` — remove `#[cfg(feature = "qat")] pub mod qat;`
- Modify: `src/main.rs:13-14` — remove `#[cfg(feature = "qat")] pub use bitloops::qat;`
- Modify: `src/cli.rs:117-119` — remove `#[cfg(feature = "qat")] Qat(crate::qat::QatArgs),`
- Modify: `src/cli.rs:207-208` — remove `#[cfg(feature = "qat")] Commands::Qat(args) => crate::qat::run(args).await,`
- Modify: `src/cli/root.rs:237-238` — remove `#[cfg(feature = "qat")] ... "qat"`
- Modify: `Cargo.toml:10` — remove `qat = []` feature line

- [x] **Step 1: Delete `src/qat/` directory**

```bash
rm -rf src/qat/
```

- [x] **Step 2: Remove from `src/lib.rs`**

Remove these two lines:
```rust
#[cfg(feature = "qat")]
pub mod qat;
```

- [x] **Step 3: Remove from `src/main.rs`**

Remove these two lines:
```rust
#[cfg(feature = "qat")]
pub use bitloops::qat;
```

- [x] **Step 4: Remove from `src/cli.rs`**

Remove the command variant (around line 117):
```rust
    #[cfg(feature = "qat")]
    /// Run Bitloops quality acceptance tests from the integrated Rust BDD harness.
    Qat(crate::qat::QatArgs),
```

Remove the dispatch arm (around line 207):
```rust
        #[cfg(feature = "qat")]
        Commands::Qat(args) => crate::qat::run(args).await,
```

- [x] **Step 5: Remove from `src/cli/root.rs`**

Remove the command name mapping (around line 237):
```rust
        #[cfg(feature = "qat")]
        crate::cli::Commands::Qat(_) => "qat",
```

- [x] **Step 6: Remove `qat` feature from `Cargo.toml`**

Remove this line from `[features]`:
```toml
qat = []
```

- [x] **Step 7: Verify production crate compiles cleanly**

Run: `cargo check`
Expected: Compiles with no errors and no warnings about the removed QAT code.

- [x] **Step 8: Verify integration test still compiles**

Run: `cargo test --test qat_acceptance --no-run`
Expected: Compiles successfully. No `--features qat` flag needed.

- [x] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: remove QAT module and feature flag from production crate"
```

---

### Task 4: Update documentation

**Files:**
- Modify: `TESTING.md`
- Modify: `qat/README.md`

- [x] **Step 1: Replace `TESTING.md`**

```markdown
# Running Tests

| Goal                           | Command                                                              |
| ------------------------------ | -------------------------------------------------------------------- |
| Fast unit tests (no E2E)       | `cargo test`                                                         |
| Run only the Gherkin suite     | `cargo test --test testlens_gherkin -- --ignored`                    |
| Run all E2E / acceptance tests | `cargo test -- --ignored`                                            |
| Run everything including E2E   | `cargo test -- --include-ignored`                                    |
| QAT smoke suite                | `cargo test --test qat_acceptance qat_smoke -- --ignored`            |
| QAT DevQL suite                | `cargo test --test qat_acceptance qat_devql -- --ignored`            |
| QAT Claude Code suite          | `cargo test --test qat_acceptance qat_claude_code -- --ignored`      |
| QAT all suites                 | `cargo test --test qat_acceptance -- --ignored`                      |

## Testing a separate binary (CI)

To test a production binary built separately:

```bash
cargo build --release
BITLOOPS_QAT_BINARY=target/release/bitloops \
  cargo test --test qat_acceptance -- --ignored
```

The `BITLOOPS_QAT_BINARY` env var tells the test harness which binary to exercise.
When unset, it falls back to the binary built alongside the test (`CARGO_BIN_EXE_bitloops`).
```

- [x] **Step 2: Update `qat/README.md`**

Update the README to reflect that QAT is now invoked via `cargo test` instead of `cargo run --features qat -- qat`. Keep the artifact layout and environment variable documentation (those are unchanged). Replace the "How to run" sections:

Replace the command examples throughout the file. The key changes:

- `cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat` → `cargo test --test qat_acceptance qat_claude_code -- --ignored`
- `cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat --smoke` → `cargo test --test qat_acceptance qat_smoke -- --ignored`
- `cargo run --manifest-path bitloops/Cargo.toml --features qat -- qat --devql` → `cargo test --test qat_acceptance qat_devql -- --ignored`
- Remove `--feature` single-file option (not supported in the new entry point)
- Remove mentions of `--features qat` Cargo feature flag

- [x] **Step 3: Commit**

```bash
git add TESTING.md qat/README.md
git commit -m "docs: update QAT documentation for integration test invocation"
```

---

### Task 5: End-to-end verification

Verify everything works together.

**Files:**
- None (verification only)

- [x] **Step 1: Verify production build has no QAT code**

Run: `cargo check`
Expected: Compiles cleanly. No QAT module, no feature flag.

- [x] **Step 2: Verify integration test compiles**

Run: `cargo test --test qat_acceptance --no-run`
Expected: Compiles without `--features qat`.

- [x] **Step 3: Run the smoke suite**

Run: `cargo test --test qat_acceptance qat_smoke -- --ignored`
Expected: 2 scenarios pass, artifacts in `target/qat-runs/`.

- [ ] **Step 4: Run the DevQL suite**

Run: `cargo test --test qat_acceptance qat_devql -- --ignored`
Expected: 23 scenarios pass (or pass with fallback markers).

Observed on 2026-03-30: currently fails because `bitloops daemon start` does not become ready in this environment (`Error: Bitloops daemon did not become ready within 20 seconds`).

- [x] **Step 5: Verify `cargo test` (default) does NOT run QAT**

Run: `cargo test`
Expected: QAT tests are ignored (they have `#[ignore]`). Only fast unit tests run.

Observed on 2026-03-30: `cargo test --test qat_acceptance` confirms all three QAT entry points are `ignored` by default.

- [x] **Step 6: Final commit (if any fixups needed)**

If any adjustments were needed during verification, commit them.

---

## Summary of changes

| Change | Why |
|--------|-----|
| `src/qat/` → `tests/qat_support/` | QAT code is test infrastructure, not production code |
| `tests/qat_acceptance.rs` created | Standard `cargo test` entry point with `#[ignore]` |
| `qat` Cargo feature removed | No feature flag needed — tests are always compilable |
| CLI `qat` subcommand removed | No longer baked into the binary |
| `BITLOOPS_QAT_BINARY` env var | CI can point at any binary to test |
| `CARGO_BIN_EXE_bitloops` fallback | Local dev just runs `cargo test` |
| `crate::` → `bitloops::` in helpers | Access library types from integration test context |
| Store assertion supports repo-local and daemon-config stores | Compatible with current daemon-era `init/enable` behavior |
| DevQL init helper retries with daemon auto-start | Aligns QAT flow with daemon-required DevQL commands |

## What this does NOT change

- Feature files stay in `qat/features/` — unchanged
- Feature file content — unchanged
- Artifact output layout — unchanged (`target/qat-runs/`)
- Existing environment variables remain supported
- Fallback marker system — unchanged
