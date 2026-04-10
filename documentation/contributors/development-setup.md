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

There are no required git hooks. From the repository root you can run the same checks as the blocking `develop` pull-request gate:

```bash
bash scripts/check-dev.sh           # file-size, fmt, clippy
bash scripts/check-dev.sh --test   # + merge lane (fast + curated slow smokes)
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

# Develop PR gate lane
cargo dev-test-merge

# CLI lane (when command output/parsing changes)
cargo dev-test-cli

# Slow e2e/integration lane (all feature-gated heavy suites only)
cargo dev-test-slow

# Full lane (post-merge `develop`, PRs to `main`, or explicit confidence run)
cargo dev-test-full

# Coverage (single run)
cargo dev-coverage
```

These test lanes are executed via `cargo-nextest`.

`duckdb-bundled` is now opt-in. Use `cargo dev-check-bundled`/`cargo dev-build-bundled` when you need bundled DuckDB (for example offline or unsupported targets).

## Test Coverage

We use `cargo-llvm-cov` for coverage. Install it:

```bash
cargo install cargo-llvm-cov
```

Install `cargo-nextest` as well. On macOS, prefer:

```bash
brew install cargo-nextest
```

For other platforms, use the official installation guide:
[https://nexte.st/docs/installation/](https://nexte.st/docs/installation/)

The project maintains a coverage baseline in `.coverage-baseline.jsonl` (under `bitloops/`). CI refreshes the baseline metadata on pushes to `develop`, and coverage stays separate from the blocking pull-request gates. To enforce the 5% tolerance locally before merging, use `bash scripts/check-dev.sh --full`.

If the repo gains doctests in future, keep those on `cargo test --doc`; `cargo-nextest` does not support doctests.

Shell helpers in `bitloops/scripts/` are kept for CI/back-compat purposes, but local workflows should use Cargo `dev-*` commands.

## Quick Reference

| Task | Command |
|------|---------|
| Check compiles | `cargo dev-check` |
| Build | `cargo dev-build` |
| Run locally | `cargo run --manifest-path bitloops/Cargo.toml --no-default-features -- <command>` |
| Fast tests | `cargo dev-test-fast` |
| Merge smoke lane | `cargo dev-test-merge` |
| Slow tests | `cargo dev-test-slow` |
| Full tests | `cargo dev-test-full` |
| Format code | `cargo fmt` |
| Lint | `cargo clippy` |
| Coverage report | `cargo dev-coverage` |
