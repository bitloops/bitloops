# Building & Distributing the CLI

## Build locally

Run from the `bitloops_cli/` directory:

```bash
# One-time local setup for build-time dashboard URL embedding
cp config/dashboard_urls.template.json config/dashboard_urls.json

# Type-check only (fast, like tsc --noEmit)
cargo check

# Dev build → target/debug/bitloops
cargo build

# Release build (optimised) → target/release/bitloops
cargo build --release

# Run without a separate build step
cargo run -- status
cargo run -- --help
```

Dashboard bundle URLs are embedded at build time from `config/dashboard_urls.json`.
If this file is missing or invalid, `cargo build`/`cargo check` will fail with a
clear remediation message.

## Local git hooks (one-time setup)

Run once from the repo root:

```bash
bash scripts/setup-hooks.sh
```

This configures git to use the versioned hooks in `.githooks/`:

- `pre-commit`: Rust file-size check, `cargo fmt`, `cargo clippy`
- `pre-push`: strict coverage non-regression check via `./bitloops_cli/scripts/coverage-baseline-check.sh check`

## Testing

Run from the `bitloops_cli/` directory:

```bash
# Test type aliases
# - core: lib crate tests (domain/application/core logic)
cargo test-core

# - cli: bin crate tests (CLI adapter/command wiring)
cargo test-cli

# - integration: tests/*.rs (includes e2e-style scenarios)
cargo test-integration

# - all: full suite
cargo test-all

# Run tests and print one combined summary block at the end
./scripts/test-summary.sh

# Run tests with coverage (single run) and print coverage summary at the end
./scripts/test-summary.sh --coverage
# (prints overall lines/functions/branches plus lowest-coverage files)

# Coverage setup (once)
brew install cargo-llvm-cov  # macOS (Linux: `apt install llvm`)

# If preview error, do
rustup component add llvm-tools-preview

# Coverage baseline gate (lines + functions, strict no-regression)
# - check: fail if current coverage < baseline - 0.05 for either metric
./scripts/coverage-baseline-check.sh check

# - update: append a new baseline entry intentionally (JSONL history)
./scripts/coverage-baseline-check.sh update

# Coverage baseline (HTML + LCOV, default cargo threading)
cargo test-coverage

# Open HTML coverage report
open target/llvm-cov-html/html/index.html
```

Test type notes:

- `core` tests are Rust library tests (`--lib`).
- `cli` tests are Rust binary tests for `bitloops` (`--bin bitloops`).
- `integration` tests are separate test targets under `tests/*.rs`; this includes end-to-end style scenarios in this repo.

Coverage outputs:

- HTML: `target/llvm-cov-html/html/index.html`
- LCOV: `target/llvm-cov.info`
- Baseline file: `.coverage-baseline.jsonl` (inside `bitloops_cli/`)

Coverage gate policy:

- Enforced in local `pre-push`.
- Metrics: lines and functions.
- Rule: `current >= baseline - 0.05` for both metrics (0.05 percentage-point tolerance).
- Baseline source on check: latest JSONL record (`tail -n 1`).

When baseline changes are intentional:

- Run `./scripts/coverage-baseline-check.sh update` from `bitloops_cli/`.
- Commit the appended baseline history entries with your code changes.
- If baseline decreases, include a short justification in the PR description.

---

## Releasing

### 1. Bump the version

Edit `Cargo.toml`:

```toml
[package]
version = "0.1.0"   # ← change this
```

### 2. Run the release script

From the repo root:

```bash
./scripts/release.sh
```

This will:

- Read the version from `Cargo.toml`
- Commit the version bump
- Create and push the git tag (`v0.1.0`)
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
  version "0.1.0"
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
| Build        | `cargo build --release`                                |
| Release      | bump `Cargo.toml` version → `./scripts/release.sh`     |
| CI workflow  | `.github/workflows/release.yml` (triggers on `v*` tag) |
| curl install | `install.sh` in repo root                              |
| Homebrew     | `bitloops/homebrew-tap` → `Formula/bitloops.rb`        |

## Maintainer Notes

Dashboard bundle URL settings are now build-time embedded:

- Input file: `config/dashboard_urls.json` (gitignored)
- Template: `config/dashboard_urls.template.json`
- Build generator: `build.rs`
- Runtime consumer: `src/server/dashboard/bundle.rs`

Optional emergency runtime overrides are still supported:

- `BITLOOPS_DASHBOARD_MANIFEST_URL`
- `BITLOOPS_DASHBOARD_CDN_BASE_URL`
