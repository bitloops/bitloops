# `include!` macro cleanup — technical debt and remediation plan

> **Status:** Archived technical-debt plan. This note is kept outside the active docs surfaces and is not part of the contributor docs site.

## What is `include!` and why is it a problem?

Rust's `include!("file.rs")` macro inserts the contents of another file **verbatim** into the current file at compile time — as if you copy-pasted the text. This means the included file shares the parent's entire scope: all `use` imports, all type definitions, all private functions. Nothing is explicitly declared or bounded.

The idiomatic Rust alternative is the **module system** (`mod foo;`), where each file is a self-contained module with:
- Its own imports (`use` statements)
- Explicit visibility (`pub`, `pub(crate)`, private by default)
- A clear contract with its parent (only `pub` items are accessible outside)

### Why the codebase uses `include!`

The pattern emerged as a way to split large files (800+ lines) into smaller physical files without changing the module structure. It's convenient because:
- No `pub` annotations needed — everything stays in the same scope
- No `use` imports needed — the parent's imports are automatically available
- No refactoring of call sites — functions/types remain in the same module path

### Why it's technical debt

1. **Implicit coupling**: Every included file silently depends on whatever the parent imports. Add or remove a `use` in the parent and an included file can break or gain unintended access. There's no compiler-enforced contract between files.

2. **No encapsulation**: All items in included files are visible to all other included files in the same parent, even if they shouldn't be. A helper function meant for one subsystem is callable by every other subsystem in the same include group.

3. **IDE/tooling confusion**: Editor features like "go to definition", "find references", and auto-import work poorly with `include!` because the file isn't a real module. Rust-analyzer can struggle with scope resolution.

4. **Flat namespace pollution**: 28 files included into `host/devql.rs` means hundreds of functions, types, and constants all share one namespace. Name collisions are possible and the mental model of "what's in scope" is unclear.

5. **Testing boundaries**: All included code is in the same module, making it impossible to unit test subsystems in isolation without the full parent scope.

## Current scope

The codebase has **90+ `include!` directives** across these areas:

| Area | Includes | Nature |
|------|----------|--------|
| `host/devql.rs` | 28 | Ingestion engine, query parser, executor, DB utils — the largest group |
| `host/checkpoints/strategy/` | 27 | Manual commit strategy impl + tests (nested 3-4 levels deep) |
| `host/devql/ingestion/schema.rs` | 8 | DDL schema strings for multiple backends |
| `host/devql/query/executor.rs` | 6 | Query executor subsystems |
| `host/devql/tests/devql_tests.rs` | 10 | BDD test harness |
| `capability_packs/semantic_clones/scoring.rs` | 4 | Clone scoring algorithms |
| `cli/explain.rs` | 5 | Explain command (nested) |
| `host/extension_host/host.rs` | 2 | `impl` blocks for CoreExtensionHost |
| Others | ~5 | Agent runtime helpers, build-generated code |

## Ideal solution

Convert each `include!("foo.rs")` into a proper `mod foo;` declaration. For each converted file:

### Step 1: Add explicit imports

The included file currently relies on the parent's `use` statements. After conversion, each file must declare its own:

```rust
// Before (included file — no imports, shares parent scope):
fn process_artefact(batch: &ProductionIngestionBatch) -> Result<()> {
    // ProductionIngestionBatch and Result are from the parent's scope
}

// After (proper module):
use anyhow::Result;
use crate::models::ProductionIngestionBatch;

pub(super) fn process_artefact(batch: &ProductionIngestionBatch) -> Result<()> {
    // Explicit imports, explicit visibility
}
```

For initial conversion, `use super::*;` is an acceptable shortcut that preserves the "everything from parent" semantics while making the dependency explicit. Over time, these should be narrowed to specific imports.

### Step 2: Add visibility annotations

Items that need to be accessible outside the new module must be marked `pub(super)` (visible to parent) or `pub(crate)` (visible crate-wide):

```rust
// Before: implicitly visible to all siblings in the include group
fn helper() { ... }

// After: explicitly scoped
pub(super) fn helper() { ... }
```

### Step 3: Re-export from parent if needed

If external code references items by the parent's module path, add re-exports:

```rust
// In parent module:
mod ingestion;
pub use ingestion::process_artefact;  // preserves the external API
```

### Step 4: Update the parent

Replace `include!("foo.rs")` with `mod foo;` and add any needed re-exports.

## Exceptions

A few `include!` usages are legitimate and should remain:

- **Build-generated code**: `include!(concat!(env!("OUT_DIR"), "/dashboard_env.rs"))` — standard Rust pattern for build script output
- **`impl` blocks split across files**: When a struct has a very large `impl` block and it's split purely for readability (e.g., `extension_host/host.rs` splitting `impl CoreExtensionHost`), `include!` is the only option since Rust doesn't allow `impl` blocks in child modules for types defined in the parent (orphan rules)
- **Test schema includes**: Where a test needs to inline SQL schema definitions from another module without creating a dependency

## Priority order for conversion

1. **`capability_packs/semantic_clones/scoring.rs`** (4 includes) — smallest production code group, clean boundaries
2. **`cli/explain.rs`** (5 includes, nested) — presentation layer, isolated
3. **`host/hooks/runtime/agent_runtime.rs`** (1 include) — single file
4. **`host/devql/query/executor.rs`** (6 includes) — query subsystem
5. **`host/devql/ingestion/schema.rs`** (8 includes) — schema definitions
6. **`host/devql.rs`** (28 includes) — largest group, most complex
7. **`host/checkpoints/strategy/`** (27 includes) — deeply nested, most risk
8. **Test files** (10 includes in devql_tests) — lower priority since test scope sharing is less harmful

## Estimated effort

Each include conversion requires:
- Reading the file to understand what scope it uses from the parent
- Adding `use` imports (or `use super::*;` as shortcut)
- Adding `pub(super)` / `pub(crate)` to items accessed by siblings or parent
- Updating the parent to declare `mod` and add re-exports
- Running `cargo check` and fixing compilation errors iteratively

For the full 90+ conversions: significant effort spread across multiple PRs. Each group (scoring, explain, executor, etc.) should be its own PR for reviewability.
