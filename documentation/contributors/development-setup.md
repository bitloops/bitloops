---
sidebar_position: 3
title: Development Setup
---

# Development Setup

Get the Bitloops codebase building and tests passing on your machine.

## Prerequisites

- **Rust** — install via [rustup](https://rustup.rs/)
- **Git** — you probably have this already

That's it. No external databases are required for local development.

## Clone and Build

```bash
# Fork the repo on GitHub, then:
git clone https://github.com/YOUR_USERNAME/bitloops.git
cd bitloops

# One-time setup for build-time dashboard URLs
cp bitloops/config/dashboard_urls.template.json bitloops/config/dashboard_urls.json

# Fast local checks/builds
cargo dev-check
cargo dev-build

# Run
cargo run --manifest-path bitloops/Cargo.toml --no-default-features -- --version
```

The Cargo `dev-*` aliases are the primary local interface for contributors and agents.

## Local checks (optional)

There are no required git hooks. From the repository root you can run the same checks as CI (for PRs into `develop`):

```bash
bash scripts/check-dev.sh           # file-size, fmt, clippy
bash scripts/check-dev.sh --test   # + full tests
bash scripts/check-dev.sh --full   # + coverage baseline
```

If an older setup pointed `core.hooksPath` here, run `bash scripts/setup-hooks.sh` once to clear it.

## Running Tests

From the repository root, use the Cargo lanes:

```bash
# Fast default lane
cargo dev-check
cargo dev-test-core
cargo dev-test-fast

# CLI lane (when command output/parsing changes)
cargo dev-test-cli

# Slow e2e/integration lane (feature-gated)
cargo dev-test-slow

# Full lane before handoff/merge
cargo dev-test-full

# Coverage (single run)
cargo dev-coverage
```

`duckdb-bundled` is now opt-in. Use `cargo dev-check-bundled`/`cargo dev-build-bundled` when you need bundled DuckDB (for example offline or unsupported targets).

## Test Coverage

We use `cargo-llvm-cov` for coverage. Install it:

```bash
cargo install cargo-llvm-cov
```

The project maintains a coverage baseline in `.coverage-baseline.jsonl` (under `bitloops/`). CI runs that check on pull requests to `develop` **informationally** (merge is not blocked by it). To enforce the 5% tolerance locally before merging, use `bash scripts/check-dev.sh --full`.

Shell helpers in `bitloops/scripts/` are kept for CI/back-compat purposes, but local workflows should use Cargo `dev-*` commands.

## Quick Reference

| Task | Command |
|------|---------|
| Check compiles | `cargo dev-check` |
| Build | `cargo dev-build` |
| Run locally | `cargo run --manifest-path bitloops/Cargo.toml --no-default-features -- <command>` |
| Fast tests | `cargo dev-test-fast` |
| Slow tests | `cargo dev-test-slow` |
| Full tests | `cargo dev-test-full` |
| Format code | `cargo fmt` |
| Lint | `cargo clippy` |
| Coverage report | `cargo dev-coverage` |
