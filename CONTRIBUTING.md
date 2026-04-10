<!-- Update the GitHub links in this file if the public repository slug changes. -->

# Contributing to Bitloops

Thanks for your interest in contributing to Bitloops.

This file is the short repo-level entrypoint. The detailed contributor
documentation lives in the docs site:

- [Contributor docs home](https://bitloops.com/docs/contributors)
- [Development setup](https://bitloops.com/docs/contributors/development-setup)
- [Submitting changes](https://bitloops.com/docs/contributors/submitting-changes)
- [Architecture overview](https://bitloops.com/docs/contributors/architecture)

## Pull request policy

- Branch from `develop`
- Open pull requests against `develop`
- Keep changes focused and reviewable
- Update docs and tests when behavior or workflows change

Typical setup:

```shell
git clone https://github.com/<your-github-username>/bitloops.git
cd bitloops
git checkout develop
git pull origin develop
git checkout -b feature/short-description
```

## Before opening a PR

- Search existing issues and pull requests first
- Open an issue early for larger changes so scope can be aligned
- Run the relevant checks for the area you changed
- Describe what changed, why it changed, and how you verified it
- Call out any privacy, telemetry, hook, storage, or security impact clearly

## More ways to contribute

- Report bugs or rough edges
- Improve docs, onboarding, or examples
- Submit code, tests, or refactors
- Review pull requests and help reproduce issues
- Share feedback from real AI-assisted development workflows

## Security, conduct, and CLA

- Follow the [Code of Conduct](CODE_OF_CONDUCT.md)
- Report vulnerabilities privately using [SECURITY.md](SECURITY.md)
- By submitting a pull request, you agree to the [CLA](CLA.md)

## Helpful links

- [Bitloops docs](https://bitloops.com/docs)
- [Issue tracker](https://github.com/bitloops/bitloops/issues)
- [Discussions](https://github.com/bitloops/bitloops/discussions)
- [Contributors](CONTRIBUTORS.md)
