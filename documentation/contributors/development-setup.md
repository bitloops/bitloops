---
sidebar_position: 3
title: Development Setup
---

# Development Setup

Get the Bitloops codebase building and tests passing on your machine.

## Prerequisites

- **Rust** — install via [rustup](https://rustup.rs/)
- **Git** — you probably have this already

That's it. SQLite and DuckDB are bundled — no external databases needed.

## Clone and Build

```bash
# Fork the repo on GitHub, then:
git clone https://github.com/YOUR_USERNAME/bitloops.git
cd bitloops

# Check everything compiles
cargo check

# Build
cargo build

# Run
cargo run -- --version
```

## Local checks (optional)

There are no required git hooks. From the repository root you can run the same checks as CI (for PRs into `develop`):

```bash
bash scripts/check-dev.sh           # file-size, fmt, clippy
bash scripts/check-dev.sh --test   # + full tests
bash scripts/check-dev.sh --full   # + coverage baseline
```

If an older setup pointed `core.hooksPath` here, run `bash scripts/setup-hooks.sh` once to clear it.

## Running Tests

From `bitloops/`, the usual full run is:

```bash
./scripts/test-summary.sh
```

That runs `cargo test --no-fail-fast` and prints combined `test result:` lines at the end. Cargo also defines optional aliases in `.cargo/config.toml` (`test-core`, `test-cli`, `test-integration`, `test-all`). Aliases only work when that config is loaded (run from `bitloops/`); from the repo root use `cargo test --manifest-path bitloops/Cargo.toml --no-fail-fast`, not `cargo test-all --manifest-path …`.

For coverage in one go (llvm-cov + summary tables):

```bash
./scripts/test-summary.sh --coverage
```

For HTML/LCOV artifacts (not the baseline gate):

```bash
./scripts/test-coverage.sh baseline
```

## Test Coverage

We use `cargo-llvm-cov` for coverage. Install it:

```bash
cargo install cargo-llvm-cov
```

The project maintains a coverage baseline in `.coverage-baseline.jsonl` (under `bitloops/`). CI runs that check on pull requests to `develop` **informationally** (merge is not blocked by it). To enforce the 5% tolerance locally before merging, use `bash scripts/check-dev.sh --full`.

## Quick Reference

| Task | Command |
|------|---------|
| Check compiles | `cargo check` |
| Build | `cargo build` |
| Run locally | `cargo run -- <command>` |
| All tests | `./scripts/test-summary.sh` or `cargo test --no-fail-fast` |
| Format code | `cargo fmt` |
| Lint | `cargo clippy` |
| Coverage report | `./scripts/test-summary.sh --coverage` |
