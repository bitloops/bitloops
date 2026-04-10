//! Tests for DevQL sync: workspace inspection, SQLite schema, content cache, extraction,
//! materialization, and `execute_sync` behaviour.
//!
//! Submodules live under `tests/sync_tests/`; paths are explicit because the parent `devql`
//! module loads this file via `#[path = ".../sync_tests.rs"]`, which anchors child `mod`
//! resolution to `tests/` rather than this file's subfolder.

#[path = "sync_tests/fixtures.rs"]
mod fixtures;

#[path = "sync_tests/content_cache.rs"]
mod content_cache;
#[path = "sync_tests/execute_sync_cache_branches.rs"]
mod execute_sync_cache_branches;
#[path = "sync_tests/execute_sync_core.rs"]
mod execute_sync_core;
#[path = "sync_tests/execute_sync_modes.rs"]
mod execute_sync_modes;
#[path = "sync_tests/execute_sync_worktree.rs"]
mod execute_sync_worktree;
#[path = "sync_tests/extraction_typescript.rs"]
mod extraction_typescript;
#[path = "sync_tests/materialize_current.rs"]
mod materialize_current;
#[path = "sync_tests/repo_sync_state_writes.rs"]
mod repo_sync_state_writes;
#[path = "sync_tests/sqlite_relational_schema.rs"]
mod sqlite_relational_schema;
#[path = "sync_tests/workspace_inspect.rs"]
mod workspace_inspect;
