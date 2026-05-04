Feature: Architecture graph entry points
  DevQL should expose effective architectural entry points from language
  evidence, runtime config, and manual graph assertions.

  Background:
    Given I run CleanStart for flow "ArchitectureGraph"
    And I start the daemon in bitloops
    And I create a bitloops-inference CLI fixture in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I run DevQL init in bitloops
    And I enqueue DevQL sync task with status in bitloops

  @devql @architecture_graph
  Scenario: Architecture graph exposes CLI entry points and assertion overrides
    Then Architecture graph entry point kind "rust_main" for path "crates/bitloops-inference/src/main.rs" is effective in bitloops
    And Architecture graph entry point kind "rust_cli_dispatch" for path "crates/bitloops-inference/src/lib.rs" is effective in bitloops
    And Architecture graph entry point kind "cargo_bin" for path "crates/bitloops-inference/src/main.rs" is effective in bitloops
    And Architecture graph container kind "cli" exposes entry point kind "cargo_bin" for path "crates/bitloops-inference/src/main.rs" in bitloops
    And Architecture graph system membership "bitloops.platform" includes entry point kind "cargo_bin" for path "crates/bitloops-inference/src/main.rs" in bitloops
    And Architecture graph assertion adds entry point kind "manual_stdio_runtime" for path "crates/bitloops-inference/src/runtime.rs" in bitloops
    And Architecture graph suppression hides entry point kind "cargo_bin" for path "crates/bitloops-inference/src/main.rs" then revoke restores it in bitloops
