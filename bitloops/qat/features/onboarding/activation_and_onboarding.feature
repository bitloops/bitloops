@onboarding
Feature: Activation and Onboarding
  As a developer adopting Bitloops for the first time,
  I want to install, initialize, enable, and see first value quickly
  so that I can trust the tool before investing further.

  # ── D1-J01: Install Bitloops ──────────────────────────────────────

  @D1-J01
  Scenario: Bitloops binary is callable and reports a version
    Given I run CleanStart for flow "install-verify"
    Then  bitloops --version exits 0 and prints a semver version

#   # ── D1-J02: Initialize global daemon config ───────────────────────
#
#   @D1-J02
#   Scenario: Initialize daemon config from scratch
#     Given I run CleanStart for flow "daemon-config-init"
#     And   I start the daemon in bitloops
#     Then  the global daemon config file exists
#     And   the config contains a relational store path
#
#   # ── D1-J03: Enable Bitloops in a repository ───────────────────────
#
#   @D1-J03
#   Scenario: Enable Bitloops in a fresh git repository
#     Given I run CleanStart for flow "enable-repo"
#     And   I start the daemon in bitloops
#     And   I run InitCommit for bitloops
#     And   I run bitloops init --agent claude-code in bitloops
#     And   I run bitloops enable in bitloops
#     Then  the repo-local .bitloops directory exists in bitloops
#     And   bitloops stores exist in bitloops
#
#   @D1-J03
#   Scenario: Enable with --project flag creates stores
#     Given I run CleanStart for flow "enable-project"
#     And   I start the daemon in bitloops
#     And   I run InitCommit for bitloops
#     And   I run bitloops init --agent claude-code in bitloops
#     And   I run bitloops enable --project in bitloops
#     Then  bitloops stores exist in bitloops
#
#   # ── D1-J04: Install and verify agent + git hooks ──────────────────
#
#   @D1-J04
#   Scenario: Agent hooks are installed after init with claude-code agent
#     Given I run CleanStart for flow "agent-hooks-claude"
#     And   I start the daemon in bitloops
#     And   I run InitCommit for bitloops
#     And   I run bitloops init --agent claude-code in bitloops
#     Then  git hooks exist for the claude-code agent in bitloops
#
#   @D1-J04
#   Scenario: Re-init with --force reinstalls hooks
#     Given I run CleanStart for flow "force-reinstall"
#     And   I start the daemon in bitloops
#     And   I run InitCommit for bitloops
#     And   I run bitloops init --agent claude-code in bitloops
#     And   I run bitloops init --agent claude-code --force in bitloops
#     Then  git hooks exist for the claude-code agent in bitloops
#
#   # ── D1-J06: First DevQL initialization and query ──────────────────
#
#   @D1-J06
#   Scenario: DevQL init creates schema and first ingest produces artefacts
#     Given I run CleanStart for flow "first-devql"
#     And   I start the daemon in bitloops
#     And   I create a TypeScript project with known dependencies in bitloops
#     And   I run InitCommit for bitloops
#     And   I run bitloops init --agent claude-code in bitloops
#     And   I run bitloops enable in bitloops
#     And   I simulate a claude checkpoint in bitloops
#     And   I run DevQL init in bitloops
#     And   I run DevQL ingest in bitloops
#     Then  DevQL artefacts query returns results in bitloops
#
#   @D1-J06
#   Scenario: DevQL query returns valid response on repo with no checkpoints
#     Given I run CleanStart for flow "devql-empty"
#     And   I start the daemon in bitloops
#     And   I run InitCommit for bitloops
#     And   I run bitloops init --agent claude-code in bitloops
#     And   I run bitloops enable in bitloops
#     And   I run DevQL init in bitloops
#     Then  DevQL ingest reports checkpoints_processed=0
#
#   # ── D1-J07: Disable Bitloops in a repository ─────────────────────
#
#   @D1-J07
#   Scenario: Disable stops capture and status reflects disabled state
#     Given I run CleanStart for flow "disable-repo"
#     And   I start the daemon in bitloops
#     And   I run InitCommit for bitloops
#     And   I run bitloops init --agent claude-code in bitloops
#     And   I run bitloops enable in bitloops
#     And   I run bitloops disable in bitloops
#     Then  bitloops status shows disabled in bitloops
