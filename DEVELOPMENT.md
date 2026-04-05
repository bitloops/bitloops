# Building & Distributing the CLI

## Build locally

Run from the repository root:

```bash
# One-time local setup for build-time dashboard URL embedding
cp bitloops/config/dashboard_urls.template.json bitloops/config/dashboard_urls.json

# Fast type-check loop (default local path)
cargo dev-check

# Dev build → bitloops/target/debug/bitloops
cargo dev-build

# Install/update the local `bitloops` binary in Cargo bin dir
cargo dev-install

# Release build with bundled DuckDB (for offline/exotic targets)
cargo build --manifest-path bitloops/Cargo.toml --release --features duckdb-bundled

# Run without a separate build step
cargo run --manifest-path bitloops/Cargo.toml --no-default-features -- status
cargo run --manifest-path bitloops/Cargo.toml --no-default-features -- --help
```

Dashboard bundle URLs are embedded at build time from `bitloops/config/dashboard_urls.json`.
If this file is missing or invalid, `cargo build`/`cargo check` will fail with a
clear remediation message.

### DuckDB defaults

Local Cargo aliases now default to the fast non-bundled DuckDB path and set
`DUCKDB_DOWNLOAD_LIB=1` automatically.

For supported hosts (`linux-gnu` amd64/arm64, macOS universal, Windows MSVC),
this uses official prebuilt `libduckdb` binaries:

```bash
cargo dev-check
```

To use a **local** unpack instead of download, set **`DUCKDB_LIB_DIR`** (and **`DUCKDB_INCLUDE_DIR`** if headers are not beside the lib) and build with **`--no-default-features`**.

On macOS, `cargo dev-install` signs the installed binary automatically. By default it uses ad-hoc signing (no secrets required). To use a keychain identity instead, set `BITLOOPS_CODESIGN_IDENTITY` in your shell profile.

For offline/exotic targets use bundled mode explicitly:

```bash
cargo dev-check-bundled
cargo dev-build-bundled
```

**Unsupported** for prebuilts: **linux-musl** and other triples without an official `libduckdb-*.zip` — use bundled mode for those targets (for example `cargo build --manifest-path bitloops/Cargo.toml --release --target x86_64-unknown-linux-musl --features duckdb-bundled`).

## Local checks (optional)

There are no repo-enforced git hooks. To match what runs on pull requests to `develop`, run from the repo root:

```bash
bash scripts/check-dev.sh           # file-size, fmt --check, clippy
bash scripts/check-dev.sh --test   # also ./scripts/test-summary.sh (full suite + combined summaries)
bash scripts/check-dev.sh --full   # also coverage baseline check
```

If you previously pointed `core.hooksPath` at this repository, run `bash scripts/setup-hooks.sh` once to clear it.

## Testing

Run from the repository root:

```bash
# Fast default loop after edits
cargo dev-check
cargo dev-test-core

# If CLI behaviour changed
cargo dev-test-cli

# Fast default test lane (no slow e2e/integration suites; binaries are pre-signed on macOS)
cargo dev-test-fast

# Slow lane only (feature-gated heavy suites)
cargo dev-test-slow

# Full lane before handoff/merge (fast + slow)
cargo dev-test-full

# Tests with coverage (single llvm-cov run) + coverage summary tables
cargo dev-coverage
cargo dev-coverage-metrics

# Coverage with both LCOV and HTML from one instrumented run
cargo dev-coverage-all

# HTML + LCOV reports (separate from the baseline gate)
cargo dev-coverage-html

# Coverage setup (once)
brew install cargo-llvm-cov  # macOS (Linux: `apt install llvm`)

# If preview error, do
rustup component add llvm-tools-preview

# Local compare against default policy thresholds (80/75 with 0.05 tolerance)
cargo dev-coverage-compare

# Open HTML coverage report
open bitloops/target/llvm-cov-html/html/index.html
```

Test type notes:

- `core` tests are Rust library tests (`--lib`).
- `cli` tests are Rust binary tests for `bitloops` (`--bin bitloops`).
- `integration` tests are explicit test targets in `Cargo.toml`.
- slow end-to-end/integration suites are gated behind `--features slow-tests`.

Coverage outputs:

- HTML: `bitloops/target/llvm-cov-html/html/index.html`
- LCOV: `bitloops/target/llvm-cov.info`

Coverage gate policy:

- On pull requests to `develop`, CI enforces coverage for **non-draft** PRs.
- Metrics: lines and functions.
- Rule: `current >= baseline - 0.05` for both metrics (0.05 percentage-point tolerance).
- Baseline source: GitHub repository variables (`BITLOOPS_COV_BASELINE_LINES_PCT`, `BITLOOPS_COV_BASELINE_FUNCTIONS_PCT`) refreshed on push to `develop`.
- Fallback when metadata is missing: lines `80.00%`, functions `75.00%`.

`bitloops/scripts/*.sh` helpers remain in-repo for CI/back-compat usage and
report formatting, but local developer workflows should use the Cargo `dev-*`
commands above.

For a focused testing and quality-check command reference, see `TESTING.md`.

---

## Releasing

### 1. Bump the version

Edit `Cargo.toml`:

```toml
[package]
version = "0.0.1"   # ← change this
```

### 2. Run the release script

From the repo root:

```bash
./scripts/release.sh
```

This will:

- Read the version from `Cargo.toml`
- Commit the version bump
- Create and push the git tag (`v0.0.11`)
- GitHub Actions picks up the tag and builds all platform binaries automatically

Before release builds, generate the environment-specific dashboard config:

```bash
mkdir -p config
DASHBOARD_CDN_BASE_URL="https://cdn.example.com/bitloops-dashboard/" \
DASHBOARD_MANIFEST_URL="https://cdn.example.com/bitloops-dashboard/bundle_versions.json"

jq -n \
  --arg cdn "$DASHBOARD_CDN_BASE_URL" \
  --arg manifest "$DASHBOARD_MANIFEST_URL" \
  '{dashboard_cdn_base_url:$cdn,dashboard_manifest_url:$manifest}' \
  > config/dashboard_urls.json

# Build script validation runs during check/build
cargo check
```

### 3. GitHub Actions builds the binaries

`.github/workflows/release.yml` triggers on the tag and produces:

| File                                        | Platform              |
| ------------------------------------------- | --------------------- |
| `bitloops-aarch64-apple-darwin.tar.gz`      | macOS Apple Silicon   |
| `bitloops-x86_64-apple-darwin.tar.gz`       | macOS Intel           |
| `bitloops-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64 (static) |
| `bitloops-x86_64-pc-windows-msvc.zip`       | Windows               |

Watch the build: `https://github.com/<org>/<repo>/actions`

---

## Install via curl

Users can install with:

```bash
curl -fsSL https://raw.githubusercontent.com/bitloops/bitloops-cli/main/install.sh | sh
```

The `install.sh` script at the repo root detects the platform, downloads the matching binary from the latest GitHub Release, installs it to `~/.local/bin` by default, and then runs hidden post-install onboarding (`bitloops curl-bash-post-install`) to offer shell completion setup.

---

## Homebrew

### Setup (one-time)

1. Create a separate GitHub repo: `bitloops/homebrew-tap`
2. Add `Formula/bitloops.rb` to that repo (template below)

### Formula (`Formula/bitloops.rb`)

```ruby
class Bitloops < Formula
  desc "Bitloops CLI"
  homepage "https://github.com/bitloops/bitloops-cli"
  version "0.0.12"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/bitloops/bitloops-cli/releases/download/v#{version}/bitloops-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
    on_intel do
      url "https://github.com/bitloops/bitloops-cli/releases/download/v#{version}/bitloops-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256"
    end
  end

  on_linux do
    url "https://github.com/bitloops/bitloops-cli/releases/download/v#{version}/bitloops-x86_64-unknown-linux-musl.tar.gz"
    sha256 "REPLACE_WITH_SHA256"
  end

  def install
    bin.install "bitloops"
  end

  test do
    system "#{bin}/bitloops", "--version"
  end
end
```

Get the `sha256` for each tarball after a release:

```bash
curl -sL <url-to-tar.gz> | shasum -a 256
```

### User install

```bash
brew tap bitloops/tap
brew install bitloops
```

---

## Summary

| Task         | How                                                    |
| ------------ | ------------------------------------------------------ |
| Build        | `cargo dev-build`                                      |
| Install      | `cargo dev-install`                                    |
| Release      | bump `Cargo.toml` version → `./scripts/release.sh`     |
| CI workflow  | `.github/workflows/release.yml` (triggers on `v*` tag) |
| curl install | `install.sh` in repo root                              |
| Homebrew     | `bitloops/homebrew-tap` → `Formula/bitloops.rb`        |

## Maintainer Notes

Dashboard bundle URL settings are now build-time embedded:

- Input file: `config/dashboard_urls.json` (gitignored)
- Template: `config/dashboard_urls.template.json`
- Build generator: `build.rs`
- Runtime consumer: `src/api/bundle.rs`

Optional emergency runtime overrides are still supported:

- `BITLOOPS_DASHBOARD_MANIFEST_URL`
- `BITLOOPS_DASHBOARD_CDN_BASE_URL`
