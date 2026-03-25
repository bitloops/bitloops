---
sidebar_position: 4
title: Submitting Changes
---

# Submitting Changes

How to get your code into Bitloops.

## Branch and Build

```bash
# Create a branch from main
git checkout -b your-feature main

# Make your changes, then verify (from bitloops/)
cd bitloops
cargo check
./scripts/test-summary.sh
cargo fmt
cargo clippy
```

Keep changes focused. One PR per concern — don't mix a bug fix with a refactor.

## Submit a PR

1. Push your branch to your fork
2. Open a PR against `main`
3. Fill in the PR template — describe what you changed and why
4. CI will run automatically (builds across 6 platforms)
5. A maintainer will review your code

### PR Tips

- **Update tests** if you changed behavior
- **Don't commit secrets** — use environment variable interpolation in config
- **Follow existing patterns** — look at how similar code is structured in the repo
- **Small PRs get reviewed faster** — break large changes into smaller pieces if you can

## Commit Messages

Be clear about what changed and why. We don't enforce a strict format, but good commit messages help everyone.

```
feat: add support for OpenCode agent hooks

Implements the adapter for OpenCode following the existing
agent adapter pattern in src/adapters/agents/.
```

## Release Process

Releases are handled by maintainers. The flow is:

1. PRs are merged to `main` with CI passing
2. Version is bumped in `bitloops/Cargo.toml`
3. `./scripts/release.sh` creates and pushes a release tag
4. GitHub Actions builds binaries for all 6 platforms (macOS ARM64/x86, Linux, Windows ARM64/x86)
5. Binaries are published to GitHub Releases
6. Homebrew tap is updated

You don't need to worry about releases — just get your PR merged and we'll handle the rest.

## Got Questions?

- Open a [GitHub Discussion](https://github.com/bitloops/bitloops/discussions)
- Join [Discord](https://discord.com/invite/vj8EdZx8gK)
- Ask in your PR — we're happy to help
