# Testing Guide

Run commands from the repository root.

## Default commands (Cargo aliases)

| Goal | Command |
| --- | --- |
| Fast compile check | `cargo dev-check` |
| Library tests only | `cargo dev-test-lib` |
| Fast lane tests | `cargo dev-test-fast` |
| Slow lane tests | `cargo dev-test-slow` |
| Full lane tests | `cargo dev-test-full` |
| Coverage (LCOV) | `cargo dev-coverage` |
| Coverage (LCOV + HTML, one instrumented run) | `cargo dev-coverage-all` |
| Coverage metrics (lines/functions) | `cargo dev-coverage-metrics` |
| Coverage compare (default 80/75 fallback policy) | `cargo dev-coverage-compare` |
| Coverage (HTML) | `cargo dev-coverage-html` |
| Install local CLI (signed on macOS) | `cargo dev-install` |
| File-size guardrail | `cargo dev-file-size` |
| Format (write fixes) | `cargo dev-fmt` |
| Format (check only) | `cargo dev-fmt-check` |
| Clippy (warnings denied) | `cargo dev-clippy` |
| One-command local gate | `cargo dev-loop` |

`cargo dev-loop` runs: `fmt` (write fixes) -> `clippy` -> fast tests -> file-size check.
`dev-test-*` aliases run with terse test output (`.` style) by default.
On macOS, `dev-test-*` and `dev-install` automatically sign produced binaries to reduce repeated policy validation overhead (`syspolicyd`).

## macOS code-signing for local development

By default, local commands use ad-hoc signing (`-`) which requires no secrets and works for all contributors.

Optional team setup for a real keychain identity:

```bash
# list available code-signing identities
security find-identity -v -p codesigning

# pick one identity and export it for your shell profile
export BITLOOPS_CODESIGN_IDENTITY="Developer ID Application: <Name> (<TEAMID>)"
```

Environment toggles:

- `BITLOOPS_CODESIGN=0`: disable local signing (not recommended on affected macOS hosts).
- `BITLOOPS_CODESIGN_IDENTITY=<identity>`: use a keychain identity instead of ad-hoc signing.
- `BITLOOPS_CODESIGN_VERIFY=0`: skip post-sign verification if needed for speed.

Team baseline recommendation:

- No shared secrets file for local development.
- Keep identity material in macOS Keychain.
- Use per-user shell env (`~/.zshrc`) for `BITLOOPS_CODESIGN_IDENTITY` only if a real identity is needed.

## Fast/slow lane policy

- Fast lane is the default loop and should stay cheap.
- Slow lane is opt-in via `--features slow-tests`.
- Full lane runs fast + slow and is for pre-handoff/pre-merge verification.

### Put a new test in slow lane if it does any of the following

- Spawns `bitloops` or other subprocess-heavy end-to-end flows.
- Uses `git` command flows as part of the scenario.
- Starts daemon/server processes or binds local ports.
- Requires isolated `HOME`/`XDG_*` environment simulation.
- Simulates full agent lifecycle/hook workflows.

### Keep a test in fast lane when it is

- Pure unit/library logic.
- Small, deterministic integration coverage without daemon/process orchestration.
- Local fixture/temp-dir based and quick to execute.

## Rules for writing new tests

1. Keep tests deterministic.
2. Do not depend on external network or remote services.
3. Use temp directories and explicit test-local state, never shared machine state.
4. Avoid hidden ordering assumptions between tests.
5. Keep assertions behaviour-focused and failure messages explicit.
6. Gate heavy tests behind `slow-tests` in `bitloops/Cargo.toml` `[[test]]` entries.

## Checklist before opening a PR

```bash
cargo dev-check
cargo dev-fmt-check
cargo dev-clippy
cargo dev-test-fast
cargo dev-file-size
```

If your change touches e2e/integration surfaces, also run:

```bash
cargo dev-test-slow
```

## Coverage

```bash
cargo dev-coverage
cargo dev-coverage-all
cargo dev-coverage-metrics
cargo dev-coverage-html
open bitloops/target/llvm-cov-html/html/index.html
```

PR coverage gate policy (`develop`, non-draft only):

- CI compares coverage against GitHub repository metadata baselines.
- If metadata is missing, CI falls back to `80.00%` lines and `75.00%` functions.
- Tolerance is `0.05` percentage points in both baseline and fallback modes.

## Install local binary

```bash
cargo dev-install
```
