---
sidebar_position: 4
title: Submitting Changes
---

# Submitting Changes

How to get your code into Bitloops.

## Branch and Build

```bash
# Sync from develop, then create your branch
git checkout develop
git pull origin develop
git checkout -b your-feature

# Make your changes, then verify (from bitloops/)
cd bitloops
cargo dev-check
cargo dev-test-fast
cargo dev-test-merge
cargo dev-fmt-check
cargo dev-clippy
```

If your change touches broad slow suites or post-merge flows, also run `cargo dev-test-full`.

Keep changes focused. One PR per concern; do not mix a bug fix with a refactor.

## Submit a PR

1. Push your branch to your fork
2. Open a PR against `develop`
3. Fill in the PR template; describe what you changed and why
4. CI will run automatically
5. Pull requests into `develop` block on file-size, formatting, Clippy, `cargo dev-test-fast`, and `cargo dev-test-merge`
6. Pushes to `develop` run `cargo dev-test-full` after merge, and pull requests into `main` include `cargo dev-test-full`
7. A maintainer will review your code

### PR Tips

- **Update tests** if you changed behavior
- **Don't commit secrets** - use environment variable interpolation in config
- **Follow existing patterns** - look at how similar code is structured in the repo
- **Small PRs get reviewed faster** - break large changes into smaller pieces if you can

## Commit Messages

Be clear about what changed and why. We don't enforce a strict format, but good commit messages help everyone.

```
feat: add support for OpenCode agent hooks

Implements the adapter for OpenCode following the existing
agent adapter pattern in src/adapters/agents/.
```

## Release Process

Releases are handled by maintainers. The flow is:

1. Contributor PRs are reviewed and merged to `develop`
2. Maintainers prepare releases from the integration branch
3. Version is bumped in `bitloops/Cargo.toml`
4. `./scripts/release.sh` creates and pushes a release tag
5. GitHub Actions builds and publishes release artifacts

You don't need to worry about releases; just get your PR merged and we'll handle the rest.

## Got Questions?

- Open a [GitHub Discussion](https://github.com/bitloops/bitloops/discussions)
- Join [Discord](https://discord.com/invite/vj8EdZx8gK)
- Ask in your PR - we're happy to help
