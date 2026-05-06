Feature: Architecture role intelligence
  DevQL should classify, manage, and incrementally update architecture roles
  through the architecture_graph capability pack.

  Background:
    Given I run CleanStart for flow "ArchitectureRoles"
    And I start the daemon in bitloops
    And I create a bitloops-inference CLI fixture in bitloops
    And I create architecture role intelligence fixture modules in bitloops
    And I run InitCommit for bitloops
    And I init bitloops in bitloops
    And I configure deterministic architecture role inference in bitloops
    And I enqueue DevQL sync task with status in bitloops

  @devql @architecture_roles
  Scenario: Seeded taxonomy and deterministic rules classify canonical artefacts
    When I run architecture role seed in bitloops
    And I activate seeded architecture role rules in bitloops
    And I run architecture role classification with full refresh in bitloops
    Then architecture roles include canonical keys "process_entrypoint,runtime_bootstrapper,cli_command_grammar,command_dispatcher,storage_adapter,current_state_consumer,capability_registration,provider_adapter" in bitloops
    And architecture role facts include path "crates/bitloops-inference/src/main.rs" in bitloops
    And architecture role facts include path "crates/bitloops-inference/src/runtime.rs" in bitloops
    And architecture role facts include path "crates/bitloops-inference/src/cli.rs" in bitloops
    And architecture role facts include path "crates/bitloops-inference/src/lib.rs" in bitloops
    And architecture role facts include path "crates/bitloops-inference/src/storage.rs" in bitloops
    And architecture role facts include path "crates/bitloops-inference/src/current_state.rs" in bitloops
    And architecture role facts include path "crates/bitloops-inference/src/register.rs" in bitloops
    And architecture role rule signals include role "process_entrypoint" for path "crates/bitloops-inference/src/main.rs" in bitloops
    And architecture role rule signals include role "runtime_bootstrapper" for path "crates/bitloops-inference/src/runtime.rs" in bitloops
    And architecture role rule signals include role "cli_command_grammar" for path "crates/bitloops-inference/src/cli.rs" in bitloops
    And architecture role rule signals include role "command_dispatcher" for path "crates/bitloops-inference/src/lib.rs" in bitloops
    And architecture role rule signals include role "storage_adapter" for path "crates/bitloops-inference/src/storage.rs" in bitloops
    And architecture role rule signals include role "current_state_consumer" for path "crates/bitloops-inference/src/current_state.rs" in bitloops
    And architecture role rule signals include role "capability_registration" for path "crates/bitloops-inference/src/register.rs" in bitloops
    And architecture role assignment for role "process_entrypoint" and path "crates/bitloops-inference/src/main.rs" is active with source "rule" in bitloops
    And architecture role assignment for role "runtime_bootstrapper" and path "crates/bitloops-inference/src/runtime.rs" is active with source "rule" in bitloops
    And architecture role assignment for role "cli_command_grammar" and path "crates/bitloops-inference/src/cli.rs" is active with source "rule" in bitloops
    And architecture role assignment for role "command_dispatcher" and path "crates/bitloops-inference/src/lib.rs" is active with source "rule" in bitloops
    And architecture role assignment for role "storage_adapter" and path "crates/bitloops-inference/src/storage.rs" is active with source "rule" in bitloops
    And architecture role assignment for role "current_state_consumer" and path "crates/bitloops-inference/src/current_state.rs" is active with source "rule" in bitloops
    And architecture role assignment for role "capability_registration" and path "crates/bitloops-inference/src/register.rs" is active with source "rule" in bitloops
    And architecture role classification output wrote at least 7 role assignments in bitloops

  @devql @architecture_roles @architecture_roles_management
  Scenario: Role management keeps stable role identity and invalidates affected assignments
    Given seeded active architecture role rules classified bitloops
    And I snapshot architecture role id for canonical key "process_entrypoint" in bitloops
    And I snapshot architecture role assignment id for role "process_entrypoint" and path "crates/bitloops-inference/src/main.rs" in bitloops
    When I rename architecture role "process_entrypoint" to "Runtime Entrypoint" and apply the proposal in bitloops
    Then architecture role canonical key "process_entrypoint" has display name "Runtime Entrypoint" in bitloops
    And architecture role canonical key "process_entrypoint" still has the snapshotted role id in bitloops
    And architecture role assignment for role "process_entrypoint" and path "crates/bitloops-inference/src/main.rs" still has the snapshotted assignment id in bitloops
    When I deprecate architecture role "process_entrypoint" without replacement and apply the proposal in bitloops
    Then architecture role canonical key "process_entrypoint" has lifecycle "deprecated" in bitloops
    And architecture role assignment for role "process_entrypoint" and path "crates/bitloops-inference/src/main.rs" has status "needs_review" in bitloops

  @devql @architecture_roles @architecture_roles_management
  Scenario: Rule edit preview shows added and removed matches before activation
    Given seeded active architecture role rules classified bitloops
    And I snapshot architecture role assignments for role "command_dispatcher" in bitloops
    When I preview an architecture role rule edit for role "command_dispatcher" that removes path "crates/bitloops-inference/src/lib.rs" and adds path "crates/bitloops-inference/src/runtime.rs" in bitloops
    Then architecture role rule edit preview shows removed match path "crates/bitloops-inference/src/lib.rs" in bitloops
    And architecture role rule edit preview shows added match path "crates/bitloops-inference/src/runtime.rs" in bitloops
    And architecture role assignments for role "command_dispatcher" still match the snapshot in bitloops

  @devql @architecture_roles @architecture_roles_adjudication
  Scenario: Ambiguous architecture role classification is adjudicated through configured inference
    Given seeded active architecture role rules classified bitloops
    And I create ambiguous architecture role fixture path "crates/bitloops-inference/src/provider/dynamic.rs" in bitloops
    And I commit changes without hooks in bitloops
    When I enqueue DevQL sync task with paths "crates/bitloops-inference/src/provider/dynamic.rs" and status in bitloops
    Then architecture role adjudication job is queued for path "crates/bitloops-inference/src/provider/dynamic.rs" in bitloops
    When I process the ArchitectureGraph role adjudication job for path "crates/bitloops-inference/src/provider/dynamic.rs" in bitloops
    Then architecture role assignment for role "provider_adapter" and path "crates/bitloops-inference/src/provider/dynamic.rs" is active with source "llm" in bitloops
    And architecture role assignment for role "provider_adapter" and path "crates/bitloops-inference/src/provider/dynamic.rs" includes LLM adjudication evidence in bitloops

  @devql @architecture_roles @architecture_roles_sync
  Scenario: Current-state sync runs architecture role classification incrementally
    Given seeded active architecture role rules classified bitloops
    When I enqueue DevQL full sync task with status in bitloops
    Then daemon capability-event status shows ArchitectureGraph sync handler completed in bitloops
    And architecture role assignment for role "process_entrypoint" and path "crates/bitloops-inference/src/main.rs" is active with source "rule" in bitloops
    And architecture role classification metrics for latest ArchitectureGraph sync show full reconcile in bitloops
    Given I snapshot architecture role fact generation for path "crates/bitloops-inference/src/main.rs" in bitloops
    And I snapshot architecture role assignment ids except path "crates/bitloops-inference/src/main.rs" in bitloops
    When I modify a source file "crates/bitloops-inference/src/main.rs" in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync task with paths "crates/bitloops-inference/src/main.rs" and status in bitloops
    Then daemon capability-event status shows ArchitectureGraph sync handler completed in bitloops
    And architecture role facts for path "crates/bitloops-inference/src/main.rs" have a newer generation than the snapshot in bitloops
    And architecture role assignment for role "process_entrypoint" and path "crates/bitloops-inference/src/main.rs" is active with source "rule" in bitloops
    And architecture role classification metrics for latest ArchitectureGraph sync show at least 1 refreshed path in bitloops
    And architecture role assignment ids except path "crates/bitloops-inference/src/main.rs" still match the snapshot in bitloops

  @devql @architecture_roles @architecture_roles_sync
  Scenario: Removed artefacts clear role facts and mark assignments stale
    Given seeded active architecture role rules classified bitloops
    And architecture role facts include path "crates/bitloops-inference/src/storage.rs" in bitloops
    And architecture role assignment for role "storage_adapter" and path "crates/bitloops-inference/src/storage.rs" is active with source "rule" in bitloops
    When I remove source file "crates/bitloops-inference/src/storage.rs" in bitloops
    And I commit changes without hooks in bitloops
    And I enqueue DevQL sync task with paths "crates/bitloops-inference/src/storage.rs" and status in bitloops
    Then daemon capability-event status shows ArchitectureGraph sync handler completed in bitloops
    And architecture role facts do not include path "crates/bitloops-inference/src/storage.rs" in bitloops
    And architecture role assignment for role "storage_adapter" and path "crates/bitloops-inference/src/storage.rs" has status "stale" in bitloops
    And architecture role assignment history records status "stale" for role "storage_adapter" and path "crates/bitloops-inference/src/storage.rs" in bitloops
