<!-- Update the GitHub links in this file if the public repository slug changes. -->

# Contributing to Bitloops

Thanks for your interest in contributing to Bitloops.

Bitloops is the local-first, open-source intelligence layer for AI-assisted
software development. Contributions to the CLI, dashboard, documentation,
integrations, examples, tests, and issue triage all help move the project
forward.

## Quick Links

- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)
- [Contributors](CONTRIBUTORS.md)
- [Bitloops Docs](https://bitloops.com/docs)
- [Issue Tracker](https://github.com/bitloops/bitloops/issues)

## Ways to Contribute

You can contribute to Bitloops in several ways:

- Report bugs, regressions, or rough edges in the workflow
- Propose features or product improvements
- Improve the README, docs, onboarding, or examples
- Submit code, tests, or refactors
- Review pull requests and help reproduce issues
- Share feedback from real AI-assisted development workflows

## Before You Start

- Search open issues and pull requests before starting work
- For larger changes, open an issue first so scope and direction can be aligned
- Keep contributions focused and reviewable
- Call out changes that affect privacy, local storage, telemetry, git hooks, or
  context capture and injection early in the discussion

## Development Setup

1. Fork the Bitloops repository.
2. Clone your fork locally.
3. Create a branch from `main` unless a maintainer asks you to target a
   different branch.
4. Review the README and docs for the area you plan to change.
5. Install the toolchain needed for that surface area.

For the CLI and core local workflow, start with Rust and Cargo. For docs,
examples, or other supporting assets, use the tooling already established in
the directories you touch.

```shell
git clone https://github.com/<your-github-username>/bitloops.git
cd bitloops
git checkout -b feature/short-description
```

## Making Changes

When preparing a contribution:

- Follow the existing project structure and naming conventions
- Keep unrelated refactors out of the same pull request
- Update docs when commands, behavior, or setup changes
- Add or update tests where practical
- Never commit secrets, credentials, or private repository data
- Be especially explicit about any change that alters network behavior, data
  retention, or what leaves the developer's machine

## Validation

Before opening a pull request:

- Run the relevant checks and tests for the area you changed
- Verify the change in a realistic local workflow when possible
- Include clear reproduction or verification steps if maintainers need to test
  manually

If your change touches the CLI experience, installation flow, dashboard, or
agent integrations, include terminal output or screenshots when they help
reviewers validate the behavior quickly.

## Submitting Changes

1. Commit your changes with a clear message.
2. Push your branch to your fork.
3. Open a pull request against the Bitloops repository.
4. Describe the problem, the change, and how you verified it.
5. Link any relevant issue, discussion, or design context.

Good pull requests are small enough to review, clear about tradeoffs, and
explicit about any breaking changes, migrations, or privacy/security impact.

## Bug Reports and Feature Requests

GitHub issues are the primary place to report bugs, request features, and start
technical discussions.

When opening an issue, include:

- The Bitloops version, release, or commit if known
- Your operating system and install method
- The agent or integration involved, if relevant
- Expected behavior
- Actual behavior
- Clear reproduction steps
- Logs or screenshots with sensitive data removed

## Security Reports

If you believe you found a vulnerability, do not open a public issue. Follow
the private reporting instructions in [SECURITY.md](SECURITY.md).

## Community Guidelines

- Be respectful and constructive
- Assume good intent and focus on the work
- Follow the [Code of Conduct](CODE_OF_CONDUCT.md)
- Prefer clarity over volume in issues and pull requests

## Licensing

Unless explicitly stated otherwise, contributions intentionally submitted for
inclusion in Bitloops are provided under the repository's Apache 2.0 license.

## Getting Help

- Check the [Bitloops docs](https://bitloops.com/docs)
- Search or open an issue in the repository
- For sensitive matters, email [opencode@bitloops.com](mailto:opencode@bitloops.com)
