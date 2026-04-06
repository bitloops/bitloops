@onboarding
Feature: Activation and Onboarding
    As a developer adopting Bitloops for the first time,
    I want to install, initialize, enable, and see first value quickly
    so that I can trust the tool before investing further.

    # ── Install Bitloops ───────────────────────────────────────
    Scenario: Bitloops binary is callable and reports a version
        Given I run CleanStart for flow "install-verify"
        Then  bitloops --version exits 0 and prints a semver version

    # ── Initialize global daemon config ───────────────────────
    Scenario: Initialize daemon config from scratch
        Given I run CleanStart for flow "daemon-config-init"
        And   I start the daemon in bitloops
        Then  the global daemon config file exists
        And   the config contains a relational store path
        And   the config contains an event store path
        And   the config contains a blob store path
        And   the store paths from the config exist on disk

    # ── Enable Bitloops in a repository ───────────────────────
    Scenario: Enable Bitloops in a fresh git repository
        Given I run CleanStart for flow "enable-repo"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent claude-code --sync=false in bitloops
        And   I run bitloops enable in bitloops
        Then  the repo-local .bitloops exists in bitloops
        And   the repo-local .bitloops.local.toml exists in bitloops



    # ── Install and verify agent hooks ──────────────────────────
    Scenario: Agent hooks are installed after init with claude-code agent
        Given I run CleanStart for flow "agent-hooks-claude"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent claude-code --sync=false in bitloops
        Then  git hooks exist for the claude-code agent in bitloops

    Scenario: Agent hooks are installed after init with codex agent
        Given I run CleanStart for flow "agent-hooks-codex"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent codex --sync=false in bitloops
        Then  git hooks exist for the codex agent in bitloops

    Scenario: Agent hooks are installed after init with cursor agent
        Given I run CleanStart for flow "agent-hooks-cursor"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent cursor --sync=false in bitloops
        Then  git hooks exist for the cursor agent in bitloops

    Scenario: Agent hooks are installed after init with gemini agent
        Given I run CleanStart for flow "agent-hooks-gemini"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent gemini --sync=false in bitloops
        Then  git hooks exist for the gemini agent in bitloops

    Scenario: Agent hooks are installed after init with copilot agent
        Given I run CleanStart for flow "agent-hooks-copilot"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent copilot --sync=false in bitloops
        Then  git hooks exist for the copilot agent in bitloops

    Scenario: Agent hooks are installed after init with open-code agent
        Given I run CleanStart for flow "agent-hooks-open-code"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent open-code --sync=false in bitloops
        Then  git hooks exist for the open-code agent in bitloops

    # ── Disable Bitloops in a repository ─────────────────────
    Scenario: Disable stops capture and status reflects disabled state
        Given I run CleanStart for flow "disable-repo"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent claude-code --sync=false in bitloops
        And   I run bitloops enable in bitloops
        And   I run bitloops disable in bitloops
        Then  bitloops status shows disabled in bitloops

    # ── Uninstall Bitloops from a repository ─────────────────
    Scenario: Uninstall removes agent and git hooks from the repository
        Given I run CleanStart for flow "uninstall-repo"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent claude-code --sync=false in bitloops
        And   I run bitloops enable in bitloops
        Then  git hooks exist for the claude-code agent in bitloops
        Given I run bitloops uninstall hooks in bitloops
        Then  agent hooks are removed for the claude-code agent in bitloops
        And   git hooks are removed in bitloops

    Scenario: Full uninstall removes all Bitloops artefacts from the repository
        Given I run CleanStart for flow "uninstall-full"
        And   I start the daemon in bitloops
        And   I run InitCommit for bitloops
        And   I run bitloops init --agent claude-code --sync=false in bitloops
        And   I run bitloops enable in bitloops
        Then  the repo-local .claude exists in bitloops
        And   git hooks exist for the claude-code agent in bitloops
        Given I run bitloops uninstall full in bitloops
        Then  agent hooks are removed for the claude-code agent in bitloops
        And   git hooks are removed in bitloops
        And   bitloops binary is not found
