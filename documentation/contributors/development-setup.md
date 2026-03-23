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

## Set Up Git Hooks

We have pre-commit hooks for formatting and linting:

```bash
./scripts/setup-hooks.sh
```

This installs:
- **Pre-commit** — runs `cargo fmt` and `cargo clippy`
- **Pre-push** — runs coverage checks against the baseline

## Running Tests

We use four test aliases to keep things organized:

| Command | What It Runs |
|---------|-------------|
| `cargo test-core` | Library crate unit tests |
| `cargo test-cli` | CLI/binary crate tests |
| `cargo test-integration` | Integration tests in `tests/` |
| `cargo test-all` | Everything |

For a quick summary with coverage:

```bash
./scripts/test-summary.sh --coverage
```

## Test Coverage

We use `cargo-llvm-cov` for coverage. Install it:

```bash
cargo install cargo-llvm-cov
```

The project maintains a coverage baseline in `.coverage-baseline.jsonl`. The pre-push hook checks that coverage doesn't regress beyond a 5% tolerance.

## Quick Reference

| Task | Command |
|------|---------|
| Check compiles | `cargo check` |
| Build | `cargo build` |
| Run locally | `cargo run -- <command>` |
| All tests | `cargo test-all` |
| Format code | `cargo fmt` |
| Lint | `cargo clippy` |
| Coverage report | `./scripts/test-summary.sh --coverage` |
