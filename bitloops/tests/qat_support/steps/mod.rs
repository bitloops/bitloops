use crate::qat_support::world::QatWorld;
use cucumber::step::Collection;

mod common;
mod given;
mod then;

use self::common::{regex, step_fn};
use self::given::*;
use self::then::*;

pub fn collection() -> Collection<QatWorld> {
    Collection::new()
        .given(
            None,
            regex(r#"^I run CleanStart for flow \"([^\"]+)\"$"#),
            step_fn(given_clean_start),
        )
        .given(
            None,
            regex(r"^I start the daemon in (\S+)$"),
            step_fn(given_start_daemon),
        )
        .given(
            None,
            regex(r"^I run InitCommit for (\S+)$"),
            step_fn(given_init_commit),
        )
        .given(
            None,
            regex(r"^I run InitCommit without post-commit refresh for (\S+)$"),
            step_fn(given_init_commit_without_post_commit_refresh),
        )
        .given(
            None,
            regex(r"^I ran InitCommit yesterday for (\S+)$"),
            step_fn(given_init_commit_yesterday),
        )
        .given(
            None,
            regex(r"^I create a Vite app project in (\S+)$"),
            step_fn(given_create_vite_app),
        )
        .given(
            None,
            regex(r"^I init bitloops in (\S+)$"),
            step_fn(given_init_bitloops),
        )
        .given(
            None,
            regex(r"^I run bitloops init --agent (\S+) in (\S+)$"),
            step_fn(given_init_bitloops_with_agent),
        )
        .given(
            None,
            regex(r"^I run bitloops init with agents (\S+) and (\S+) in (\S+)$"),
            step_fn(given_init_bitloops_with_agents),
        )
        .given(
            None,
            regex(r"^I run bitloops init --agent (\S+) --sync=false in (\S+)$"),
            step_fn(given_init_bitloops_with_agent_sync_false),
        )
        .given(
            None,
            regex(r"^I run bitloops init --agent (\S+) --sync=true in (\S+)$"),
            step_fn(given_init_bitloops_with_agent_sync_true),
        )
        .given(
            None,
            regex(r"^I run bitloops init --agent (\S+) --sync=(true|false) --ingest=(true|false) in (\S+)$"),
            step_fn(given_init_bitloops_with_agent_sync_ingest),
        )
        .given(
            None,
            regex(r"^I set DevQL producer policy --sync=(true|false) --ingest=(true|false) in (\S+)$"),
            step_fn(given_set_devql_producer_policy),
        )
        .given(
            None,
            regex(r"^I run bitloops producer-contract init --agent (\S+) --sync=(true|false) in (\S+)$"),
            step_fn(given_init_bitloops_producer_contract),
        )
        .given(
            None,
            regex(
                r"^I run bitloops init --agent (\S+) --sync=false --ingest=true --backfill=(\d+) in (\S+)$",
            ),
            step_fn(given_init_bitloops_with_agent_sync_false_ingest_true_backfill),
        )
        .given(
            None,
            regex(r"^I run bitloops enable --capture in (\S+)$"),
            step_fn(given_enable_capture),
        )
        .given(
            None,
            regex(r"^I run bitloops disable --capture --devql-guidance in (\S+)$"),
            step_fn(given_disable_capture_and_devql_guidance),
        )
        .given(
            None,
            regex(r"^I run bitloops uninstall full in (\S+)$"),
            step_fn(given_uninstall_full),
        )
        .given(
            None,
            regex(r"^I run bitloops uninstall hooks in (\S+)$"),
            step_fn(given_uninstall_hooks),
        )
        .given(
            None,
            regex(r"^I ensure Claude Code auth in (\S+)$"),
            step_fn(given_claude_auth),
        )
        .given(
            None,
            regex(r"^I make a first change using Claude Code to (\S+)$"),
            step_fn(given_first_claude_change),
        )
        .given(
            None,
            regex(r"^I make a first change using (\S+) to (\S+)$"),
            step_fn(given_first_agent_change),
        )
        .given(
            None,
            regex(r#"^I ask Claude Code to "([^"]+)" in (\S+)$"#),
            step_fn(given_claude_code_prompt),
        )
        .when(
            None,
            regex(r#"^I ask Claude Code to "([^"]+)" in (\S+)$"#),
            step_fn(given_claude_code_prompt),
        )
        .given(
            None,
            regex(
                r#"^I ask (claude-code|cursor|gemini|copilot|codex|opencode|open-code) to "([^"]+)" in (\S+)$"#,
            ),
            step_fn(given_supported_agent_prompt),
        )
        .when(
            None,
            regex(
                r#"^I ask (claude-code|cursor|gemini|copilot|codex|opencode|open-code) to "([^"]+)" in (\S+)$"#,
            ),
            step_fn(given_supported_agent_prompt),
        )
        .given(
            None,
            regex(r"^I make a second change using Claude Code to (\S+)$"),
            step_fn(given_second_claude_change),
        )
        .given(
            None,
            regex(r"^I make a second change using (\S+) to (\S+)$"),
            step_fn(given_second_agent_change),
        )
        .given(
            None,
            regex(r"^I committed yesterday in (\S+)$"),
            step_fn(given_commit_yesterday),
        )
        .when(
            None,
            regex(r"^I committed yesterday in (\S+)$"),
            step_fn(given_commit_yesterday),
        )
        .given(
            None,
            regex(r"^I committed today in (\S+)$"),
            step_fn(given_commit_today),
        )
        .when(
            None,
            regex(r"^I committed today in (\S+)$"),
            step_fn(given_commit_today),
        )
        .given(
            None,
            regex(r"^I run DevQL init in (\S+)$"),
            step_fn(given_devql_init),
        )
        .given(
            None,
            regex(r"^I enable watcher autostart in (\S+)$"),
            step_fn(given_enable_watcher_autostart),
        )
        .given(
            None,
            regex(r"^I enqueue DevQL ingest task with status in (\S+)$"),
            step_fn(given_enqueue_devql_ingest_task_with_status),
        )
        .when(
            None,
            regex(r"^I enqueue DevQL ingest task with status in (\S+)$"),
            step_fn(given_enqueue_devql_ingest_task_with_status),
        )
        .then(
            None,
            regex(r"^I enqueue DevQL ingest task with status in (\S+)$"),
            step_fn(given_enqueue_devql_ingest_task_with_status),
        )
        .given(
            None,
            regex(r"^I enqueue DevQL ingest task with backfill (\d+) and status in (\S+)$"),
            step_fn(given_enqueue_devql_ingest_task_with_backfill_and_status),
        )
        .given(
            None,
            regex(r"^I snapshot ingest DB state in (\S+)$"),
            step_fn(given_snapshot_ingest_db_state),
        )
        .given(
            None,
            regex(r"^I create (\d+) ingest commits in (\S+)$"),
            step_fn(given_create_ingest_commits),
        )
        .given(
            None,
            regex(r"^I create a non-FF merge with 2 feature commits in (\S+)$"),
            step_fn(given_non_ff_merge_with_two_feature_commits),
        )
        .given(
            None,
            regex(r"^I create an FF merge with 2 feature commits in (\S+)$"),
            step_fn(given_ff_merge_with_two_feature_commits),
        )
        .given(
            None,
            regex(r"^I cherry-pick 2 commits in (\S+)$"),
            step_fn(given_cherry_pick_two_commits),
        )
        .given(
            None,
            regex(r"^I capture top (\d+) reachable SHAs before rewrite in (\S+)$"),
            step_fn(given_capture_top_reachable_before_rewrite),
        )
        .given(
            None,
            regex(r"^I rewrite last (\d+) commits with rebase edit in (\S+)$"),
            step_fn(given_rebase_edit_rewrite_last_commits),
        )
        .given(
            None,
            regex(r"^I reset last (\d+) commits and create replacement commits in (\S+)$"),
            step_fn(given_reset_and_rewrite_last_commits),
        )
        .given(
            None,
            regex(r"^I create a TypeScript project with known dependencies in (\S+)$"),
            step_fn(given_create_ts_deps_project),
        )
        .given(
            None,
            regex(r#"^I add a new caller of \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_add_new_caller),
        )
        .given(
            None,
            regex(r"^I create a TypeScript project with tests and coverage in (\S+)$"),
            step_fn(given_create_ts_test_project),
        )
        .given(
            None,
            regex(r"^I create a Rust project with tests in (\S+)$"),
            step_fn(given_create_rust_project_with_tests),
        )
        .given(
            None,
            regex(r"^I create a bitloops-inference CLI fixture in (\S+)$"),
            step_fn(given_create_bitloops_inference_cli_fixture),
        )
        .given(
            None,
            regex(r"^I create architecture role intelligence fixture modules in (\S+)$"),
            step_fn(given_create_architecture_role_intelligence_fixture_modules),
        )
        .given(
            None,
            regex(r"^I configure deterministic architecture role inference in (\S+)$"),
            step_fn(given_configure_deterministic_architecture_role_inference),
        )
        .given(
            None,
            regex(r"^seeded active architecture role rules classified (\S+)$"),
            step_fn(given_seeded_active_architecture_role_rules_classified),
        )
        .given(
            None,
            regex(r"^I run architecture role seed in (\S+)$"),
            step_fn(given_run_architecture_role_seed),
        )
        .when(
            None,
            regex(r"^I run architecture role seed in (\S+)$"),
            step_fn(given_run_architecture_role_seed),
        )
        .given(
            None,
            regex(r"^I activate seeded architecture role rules in (\S+)$"),
            step_fn(given_activate_seeded_architecture_role_rules),
        )
        .when(
            None,
            regex(r"^I activate seeded architecture role rules in (\S+)$"),
            step_fn(given_activate_seeded_architecture_role_rules),
        )
        .given(
            None,
            regex(r"^I run architecture role classification with full refresh in (\S+)$"),
            step_fn(given_run_architecture_role_classification_full_refresh),
        )
        .when(
            None,
            regex(r"^I run architecture role classification with full refresh in (\S+)$"),
            step_fn(given_run_architecture_role_classification_full_refresh),
        )
        .given(
            None,
            regex(r#"^I snapshot architecture role id for canonical key \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_snapshot_architecture_role_id),
        )
        .given(
            None,
            regex(r#"^I snapshot architecture role assignment id for role \"([^\"]+)\" and path \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_snapshot_architecture_role_assignment_id),
        )
        .when(
            None,
            regex(r#"^I rename architecture role \"([^\"]+)\" to \"([^\"]+)\" and apply the proposal in (\S+)$"#),
            step_fn(given_rename_architecture_role_and_apply_proposal),
        )
        .when(
            None,
            regex(r#"^I rename architecture role \"([^\"]+)\" to \"([^\"]+)\" and show the proposal in (\S+)$"#),
            step_fn(given_rename_architecture_role_and_show_proposal),
        )
        .when(
            None,
            regex(r#"^I deprecate architecture role \"([^\"]+)\" without replacement and apply the proposal in (\S+)$"#),
            step_fn(given_deprecate_architecture_role_without_replacement_and_apply_proposal),
        )
        .when(
            None,
            regex(r#"^I deprecate architecture role \"([^\"]+)\" without replacement and show the proposal in (\S+)$"#),
            step_fn(given_deprecate_architecture_role_without_replacement_and_show_proposal),
        )
        .when(
            None,
            regex(r"^I show the latest architecture role proposal in (\S+)$"),
            step_fn(given_show_latest_architecture_role_proposal),
        )
        .when(
            None,
            regex(r"^I apply the latest architecture role proposal in (\S+)$"),
            step_fn(given_apply_latest_architecture_role_proposal),
        )
        .given(
            None,
            regex(r#"^I snapshot architecture role assignments for role \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_snapshot_architecture_role_assignments_for_role),
        )
        .when(
            None,
            regex(r#"^I preview an architecture role rule edit for role \"([^\"]+)\" that removes path \"([^\"]+)\" and adds path \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_preview_architecture_role_rule_edit),
        )
        .given(
            None,
            regex(r#"^I create ambiguous architecture role fixture path \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_create_ambiguous_architecture_role_fixture_path),
        )
        .when(
            None,
            regex(r#"^I process the ArchitectureGraph role adjudication job for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_process_architecture_role_adjudication_job_for_path),
        )
        .given(
            None,
            regex(r#"^I snapshot architecture role fact generation for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_snapshot_architecture_role_fact_generation),
        )
        .given(
            None,
            regex(r#"^I snapshot architecture role rule assignment ids except path \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_snapshot_architecture_role_assignment_ids_except_path),
        )
        .when(
            None,
            regex(r"^I run architecture role status as JSON in (\S+)$"),
            step_fn(given_run_architecture_roles_status_json),
        )
        .when(
            None,
            regex(r#"^I run architecture role classification for paths \"([^\"]+)\" as JSON in (\S+)$"#),
            step_fn(given_run_architecture_role_classification_paths_json),
        )
        .when(
            None,
            regex(r#"^I run architecture role classification for paths \"([^\"]+)\" with adjudication disabled as JSON in (\S+)$"#),
            step_fn(given_run_architecture_role_classification_paths_json_with_adjudication_disabled),
        )
        .when(
            None,
            regex(r"^I run architecture role classification repair-stale as JSON in (\S+)$"),
            step_fn(given_run_architecture_role_classification_repair_stale_json),
        )
        .when(
            None,
            regex(r#"^I remove source file \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_remove_source_file_at_path),
        )
        .given(
            None,
            regex(r"^I run (?:TestHarness|TestLens) ingest-tests for latest commit in (\S+)$"),
            step_fn(given_testlens_ingest_tests),
        )
        .given(
            None,
            regex(r"^I run (?:TestHarness|TestLens) ingest-coverage for latest commit in (\S+)$"),
            step_fn(given_testlens_ingest_coverage),
        )
        .given(
            None,
            regex(
                r"^I run (?:TestHarness|TestLens) ingest-results with a failing test for latest commit in (\S+)$",
            ),
            step_fn(given_testlens_ingest_results_failing),
        )
        .given(
            None,
            regex(r"^I create a TypeScript project with similar implementations in (\S+)$"),
            step_fn(given_create_ts_similar_project),
        )
        .given(
            None,
            regex(r"^I create a TypeScript project with semantic clone quality fixtures in (\S+)$"),
            step_fn(given_create_ts_semantic_clone_quality_project),
        )
        .given(
            None,
            regex(r"^I add semantic clone fixtures in (\S+)$"),
            step_fn(given_add_semantic_clone_fixtures),
        )
        .given(
            None,
            regex(r"^I modify a semantic clone fixture source file in (\S+)$"),
            step_fn(given_modify_semantic_clone_fixture_source),
        )
        .given(
            None,
            regex(r"^I configure guide-aligned semantic clones with fake embeddings runtime in (\S+)$"),
            step_fn(given_configure_semantic_clones_guide_aligned_fake_runtime),
        )
        .given(
            None,
            regex(r"^I configure semantic clones with fake embeddings runtime in (\S+)$"),
            step_fn(given_configure_semantic_clones_fake_runtime),
        )
        .given(
            None,
            regex(r"^I configure context guidance with fake text-generation runtime in (\S+)$"),
            step_fn(given_configure_context_guidance_fake_runtime),
        )
        .given(
            None,
            regex(r"^DevQL pack health for semantic clones is ready in (\S+)$"),
            step_fn(given_devql_semantic_clones_pack_health_ready),
        )
        .given(
            None,
            regex(r"^I run DevQL semantic clones rebuild in (\S+)$"),
            step_fn(given_devql_semantic_clones_rebuild),
        )
        .given(
            None,
            regex(r"^I run daemon enrichments status in (\S+)$"),
            step_fn(given_daemon_enrichments_status),
        )
        .when(
            None,
            regex(r"^I run daemon enrichments status in (\S+)$"),
            step_fn(given_daemon_enrichments_status),
        )
        .given(
            None,
            regex(r"^I wait for semantic clone enrichments to drain in (\S+)$"),
            step_fn(given_wait_semantic_clone_enrichments_to_drain),
        )
        .when(
            None,
            regex(r"^I wait for semantic clone enrichments to drain in (\S+)$"),
            step_fn(given_wait_semantic_clone_enrichments_to_drain),
        )
        .given(
            None,
            regex(r"^I create a simple Rust project in (\S+)$"),
            step_fn(given_create_simple_rust_project),
        )
        .given(
            None,
            regex(r"^I enqueue DevQL sync task with status in (\S+)$"),
            step_fn(given_enqueue_devql_sync_task_with_status),
        )
        .when(
            None,
            regex(r"^I enqueue DevQL sync task with status in (\S+)$"),
            step_fn(given_enqueue_devql_sync_task_with_status),
        )
        .given(
            None,
            regex(r"^I enqueue DevQL sync task without status in (\S+)$"),
            step_fn(given_enqueue_devql_sync_task_without_status),
        )
        .when(
            None,
            regex(r"^I enqueue DevQL sync task without status in (\S+)$"),
            step_fn(given_enqueue_devql_sync_task_without_status),
        )
        .given(
            None,
            regex(r"^I enqueue DevQL ingest task without status in (\S+)$"),
            step_fn(given_enqueue_devql_ingest_task_without_status),
        )
        .when(
            None,
            regex(r"^I enqueue DevQL ingest task without status in (\S+)$"),
            step_fn(given_enqueue_devql_ingest_task_without_status),
        )
        .given(
            None,
            regex(r#"^I enqueue DevQL sync task with paths \"([^\"]+)\" and status in (\S+)$"#),
            step_fn(given_enqueue_devql_sync_task_with_paths_and_status),
        )
        .when(
            None,
            regex(r#"^I enqueue DevQL sync task with paths \"([^\"]+)\" and status in (\S+)$"#),
            step_fn(given_enqueue_devql_sync_task_with_paths_and_status),
        )
        .given(
            None,
            regex(r"^I enqueue DevQL full sync task with status in (\S+)$"),
            step_fn(given_enqueue_devql_full_sync_task_with_status),
        )
        .when(
            None,
            regex(r"^I enqueue DevQL full sync task with status in (\S+)$"),
            step_fn(given_enqueue_devql_full_sync_task_with_status),
        )
        .given(
            None,
            regex(r"^I enqueue DevQL sync validate task with status in (\S+)$"),
            step_fn(given_enqueue_devql_sync_validate_task_with_status),
        )
        .when(
            None,
            regex(r"^I enqueue DevQL sync validate task with status in (\S+)$"),
            step_fn(given_enqueue_devql_sync_validate_task_with_status),
        )
        .given(
            None,
            regex(r"^I enqueue DevQL sync repair task with status in (\S+)$"),
            step_fn(given_enqueue_devql_sync_repair_task_with_status),
        )
        .when(
            None,
            regex(r"^I enqueue DevQL sync repair task with status in (\S+)$"),
            step_fn(given_enqueue_devql_sync_repair_task_with_status),
        )
        .given(
            None,
            regex(r"^I attempt to enqueue DevQL sync task in (\S+)$"),
            step_fn(given_attempt_to_enqueue_devql_sync_task),
        )
        .given(
            None,
            regex(r"^I attempt to enqueue DevQL sync task with require-daemon in (\S+)$"),
            step_fn(given_attempt_to_enqueue_devql_sync_task_require_daemon),
        )
        .given(
            None,
            regex(r"^I run DevQL tasks status in (\S+)$"),
            step_fn(given_run_devql_tasks_status),
        )
        .given(
            None,
            regex(r"^I wait for the DevQL task queue to become idle in (\S+)$"),
            step_fn(given_wait_for_devql_task_queue_idle),
        )
        .given(
            None,
            regex(r#"^I snapshot completed DevQL sync task source \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_snapshot_completed_sync_task_source),
        )
        .given(
            None,
            regex(r"^I run DevQL tasks list in (\S+)$"),
            step_fn(given_run_devql_tasks_list),
        )
        .given(
            None,
            regex(r#"^I run DevQL tasks list for status \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_run_devql_tasks_list_for_status),
        )
        .given(
            None,
            regex(r"^I watch the last DevQL task in (\S+)$"),
            step_fn(given_watch_last_devql_task),
        )
        .given(
            None,
            regex(r#"^I pause the DevQL task queue with reason \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_pause_devql_tasks_with_reason),
        )
        .given(
            None,
            regex(r"^I resume the DevQL task queue in (\S+)$"),
            step_fn(given_resume_devql_tasks),
        )
        .given(
            None,
            regex(r"^I cancel the last DevQL task in (\S+)$"),
            step_fn(given_cancel_last_devql_task),
        )
        .given(
            None,
            regex(r"^I add a new source file in (\S+)$"),
            step_fn(given_add_new_source_file),
        )
        .when(
            None,
            regex(r"^I add a new source file in (\S+)$"),
            step_fn(given_add_new_source_file),
        )
        .given(
            None,
            regex(r#"^I add a source file \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_add_source_file_at_path),
        )
        .when(
            None,
            regex(r#"^I add a source file \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_add_source_file_at_path),
        )
        .given(
            None,
            regex(r"^I modify an existing source file in (\S+)$"),
            step_fn(given_modify_existing_source_file),
        )
        .when(
            None,
            regex(r"^I modify an existing source file in (\S+)$"),
            step_fn(given_modify_existing_source_file),
        )
        .given(
            None,
            regex(r#"^I modify a source file \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_modify_source_file_at_path),
        )
        .when(
            None,
            regex(r#"^I modify a source file \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_modify_source_file_at_path),
        )
        .given(
            None,
            regex(r#"^I snapshot current-state content ids for \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_snapshot_current_file_state_content_ids),
        )
        .given(
            None,
            regex(r"^I delete a source file in (\S+)$"),
            step_fn(given_delete_a_source_file),
        )
        .when(
            None,
            regex(r"^I delete a source file in (\S+)$"),
            step_fn(given_delete_a_source_file),
        )
        .given(
            None,
            regex(r"^I delete a test file in (\S+)$"),
            step_fn(given_delete_test_file),
        )
        .given(
            None,
            regex(r"^I commit changes without hooks in (\S+)$"),
            step_fn(given_commit_without_hooks),
        )
        .when(
            None,
            regex(r"^I commit changes without hooks in (\S+)$"),
            step_fn(given_commit_without_hooks),
        )
        .given(
            None,
            regex(r"^I commit changes with hooks in (\S+)$"),
            step_fn(given_commit_with_hooks),
        )
        .when(
            None,
            regex(r"^I commit changes with hooks in (\S+)$"),
            step_fn(given_commit_with_hooks),
        )
        .given(
            None,
            regex(r#"^I create a branch \"([^\"]+)\" with source file \"([^\"]+)\" and return in (\S+)$"#),
            step_fn(given_create_branch_with_source_file_and_return),
        )
        .given(
            None,
            regex(r"^I checkout the previous branch in (\S+)$"),
            step_fn(given_checkout_previous_branch),
        )
        .when(
            None,
            regex(r#"^I checkout branch \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_checkout_branch),
        )
        .when(
            None,
            regex(r"^I checkout the previous branch in (\S+)$"),
            step_fn(given_checkout_previous_branch),
        )
        .when(
            None,
            regex(r"^I run git reset --hard HEAD in (\S+)$"),
            step_fn(given_git_reset_hard_head),
        )
        .when(
            None,
            regex(r"^I run git clean -fd in (\S+)$"),
            step_fn(given_git_clean_fd),
        )
        .given(
            None,
            regex(r"^I stage the changes without committing in (\S+)$"),
            step_fn(given_stage_without_committing),
        )
        .given(
            None,
            regex(r"^I stop the daemon in (\S+)$"),
            step_fn(given_stop_daemon),
        )
        .given(
            None,
            regex(r"^I simulate a git pull with new changes in (\S+)$"),
            step_fn(given_simulate_git_pull),
        )
        .when(
            None,
            regex(r"^I simulate a git pull with new changes in (\S+)$"),
            step_fn(given_simulate_git_pull),
        )
        .given(
            None,
            regex(r"^I create a new branch with additional source files in (\S+)$"),
            step_fn(given_create_branch_with_files),
        )
        .given(
            None,
            regex(r#"^I add knowledge URL \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_knowledge_add),
        )
        .given(
            None,
            regex(r#"^I add knowledge URL \"([^\"]+)\" with commit association in (\S+)$"#),
            step_fn(given_knowledge_add_with_commit),
        )
        .given(
            None,
            regex(r#"^I associate knowledge \"([^\"]+)\" to knowledge \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_knowledge_associate),
        )
        .given(
            None,
            regex(r#"^I refresh knowledge \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_knowledge_refresh),
        )
        .given(
            None,
            regex(r"^I configure deterministic Confluence knowledge fixtures in (\S+)$"),
            step_fn(given_configure_deterministic_confluence_knowledge_fixtures),
        )
        .given(
            None,
            regex(r#"^I add fixture knowledge \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_fixture_knowledge_add),
        )
        .given(
            None,
            regex(r#"^I refresh fixture knowledge \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_fixture_knowledge_refresh),
        )
        .given(
            None,
            regex(r#"^I attempt to add knowledge URL \"([^\"]+)\" in (\S+)$"#),
            step_fn(given_knowledge_add_expect_failure),
        )
        .then(
            None,
            regex(r"^bitloops stores exist in (\S+)$"),
            step_fn(then_bitloops_stores_exist),
        )
        .then(
            None,
            regex(r"^bitloops --version exits 0 and prints a semver version$"),
            step_fn(then_version_output),
        )
        .then(
            None,
            regex(r"^the global daemon config file exists$"),
            step_fn(then_daemon_config_exists),
        )
        .then(
            None,
            regex(r"^the config contains a relational store path$"),
            step_fn(then_config_has_relational_store),
        )
        .then(
            None,
            regex(r"^the config contains an event store path$"),
            step_fn(then_config_has_event_store),
        )
        .then(
            None,
            regex(r"^the config contains a blob store path$"),
            step_fn(then_config_has_blob_store),
        )
        .then(
            None,
            regex(r"^the store paths from the config exist on disk$"),
            step_fn(then_store_paths_exist),
        )
        .then(
            None,
            regex(r"^the repo-local (.+) exists in (\S+)$"),
            step_fn(then_repo_local_path_exists),
        )
        .then(
            None,
            regex(r"^the repo-local (.+) does not exist in (\S+)$"),
            step_fn(then_repo_local_path_missing),
        )
        .then(
            None,
            regex(r"^global Bitloops runtime artefacts are removed$"),
            step_fn(then_global_runtime_artefacts_removed),
        )
        .then(
            None,
            regex(r"^git hooks exist for the (\S+) agent in (\S+)$"),
            step_fn(then_agent_hooks_exist),
        )
        .then(
            None,
            regex(r"^agent hooks are removed for the (\S+) agent in (\S+)$"),
            step_fn(then_agent_hooks_removed),
        )
        .then(
            None,
            regex(r"^git hooks are removed in (\S+)$"),
            step_fn(then_git_hooks_removed),
        )
        .then(
            None,
            regex(r"^git post-commit hook exists in (\S+)$"),
            step_fn(then_git_post_commit_hook_exists),
        )
        .then(
            None,
            regex(r"^bitloops status shows disabled in (\S+)$"),
            step_fn(then_status_shows_disabled),
        )
        .then(
            None,
            regex(r"^commit_checkpoints count is at least (\d+) in (\S+)$"),
            step_fn(then_commit_checkpoints_count),
        )
        .then(
            None,
            regex(r"^checkpoint timeline and contents are correct in (\S+)$"),
            step_fn(then_commit_timeline_is_correct),
        )
        .then(
            None,
            regex(r"^git timeline and contents are correct in (\S+)$"),
            step_fn(then_git_timeline_is_correct),
        )
        .then(
            None,
            regex(r#"^architecture roles include canonical keys \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_roles_include_canonical_keys),
        )
        .then(
            None,
            regex(r#"^architecture role facts include path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_facts_include_path),
        )
        .given(
            None,
            regex(r#"^architecture role facts include path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_facts_include_path),
        )
        .then(
            None,
            regex(r#"^architecture role facts do not include path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_facts_do_not_include_path),
        )
        .then(
            None,
            regex(r#"^architecture role facts for path \"([^\"]+)\" have a newer generation than the snapshot in (\S+)$"#),
            step_fn(then_architecture_role_facts_newer_than_snapshot),
        )
        .then(
            None,
            regex(r#"^architecture role rule signals include role \"([^\"]+)\" for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_rule_signal_for_path),
        )
        .then(
            None,
            regex(r#"^architecture role assignment for role \"([^\"]+)\" and path \"([^\"]+)\" is active with source \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_assignment_active_with_source),
        )
        .given(
            None,
            regex(r#"^architecture role assignment for role \"([^\"]+)\" and path \"([^\"]+)\" is active with source \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_assignment_active_with_source),
        )
        .then(
            None,
            regex(r#"^architecture role assignment for role \"([^\"]+)\" and path \"([^\"]+)\" has status \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_assignment_status),
        )
        .then(
            None,
            regex(r#"^architecture role classification output wrote at least ([0-9]+) role assignments in (\S+)$"#),
            step_fn(then_architecture_role_classification_output_wrote_at_least_role_assignments),
        )
        .then(
            None,
            regex(r#"^architecture role adjudication queue has no job for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_adjudication_queue_has_no_job_for_path),
        )
        .then(
            None,
            regex(r#"^architecture role adjudication job is queued for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_adjudication_job_exists_for_path),
        )
        .then(
            None,
            regex(r#"^architecture role assignment for role \"([^\"]+)\" and path \"([^\"]+)\" includes LLM adjudication evidence in (\S+)$"#),
            step_fn(then_architecture_role_assignment_includes_llm_evidence),
        )
        .then(
            None,
            regex(r#"^architecture role canonical key \"([^\"]+)\" has display name \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_display_name),
        )
        .then(
            None,
            regex(r#"^architecture role canonical key \"([^\"]+)\" still has the snapshotted role id in (\S+)$"#),
            step_fn(then_architecture_role_id_matches_snapshot),
        )
        .then(
            None,
            regex(r#"^architecture role assignment for role \"([^\"]+)\" and path \"([^\"]+)\" still has the snapshotted assignment id in (\S+)$"#),
            step_fn(then_architecture_role_assignment_id_matches_snapshot),
        )
        .then(
            None,
            regex(r#"^architecture role canonical key \"([^\"]+)\" has lifecycle \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_lifecycle),
        )
        .then(
            None,
            regex(r#"^architecture role rule edit preview shows removed match path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_rule_edit_preview_removed_path),
        )
        .then(
            None,
            regex(r#"^architecture role rule edit preview shows added match path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_rule_edit_preview_added_path),
        )
        .then(
            None,
            regex(r#"^architecture role assignments for role \"([^\"]+)\" still match the snapshot in (\S+)$"#),
            step_fn(then_architecture_role_assignments_for_role_match_snapshot),
        )
        .then(
            None,
            regex(r"^daemon capability-event status shows ArchitectureGraph sync handler completed in (\S+)$"),
            step_fn(then_architecture_graph_sync_handler_completed),
        )
        .then(
            None,
            regex(r"^architecture role classification metrics for latest ArchitectureGraph sync show full reconcile in (\S+)$"),
            step_fn(then_latest_architecture_role_metrics_full_reconcile),
        )
        .then(
            None,
            regex(r"^architecture role classification metrics for latest ArchitectureGraph sync show at least 1 refreshed path in (\S+)$"),
            step_fn(then_latest_architecture_role_metrics_refreshed_at_least_one_path),
        )
        .then(
            None,
            regex(r#"^architecture role rule assignment ids except path \"([^\"]+)\" still match the snapshot in (\S+)$"#),
            step_fn(then_architecture_role_assignment_ids_except_path_match_snapshot),
        )
        .then(
            None,
            regex(r#"^architecture role assignment history records status \"([^\"]+)\" for role \"([^\"]+)\" and path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_assignment_history_status),
        )
        .then(
            None,
            regex(r#"^architecture role proposal output includes text \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_proposal_output_includes_text),
        )
        .then(
            None,
            regex(r#"^architecture role status JSON includes a review item with status \"([^\"]+)\" for role \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_status_json_review_item_status_for_role),
        )
        .then(
            None,
            regex(r#"^architecture role status JSON includes a queue item for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_role_status_json_queue_item_for_path),
        )
        .then(
            None,
            regex(r"^architecture role classification JSON reports at least ([0-9]+) adjudication candidates? in (\S+)$"),
            step_fn(then_architecture_role_classification_json_adjudication_candidates_at_least),
        )
        .then(
            None,
            regex(r"^architecture role classification JSON reports ([0-9]+) enqueued adjudication jobs in (\S+)$"),
            step_fn(then_architecture_role_classification_json_enqueued_adjudication_jobs),
        )
        .then(
            None,
            regex(r"^architecture role classification JSON reports full_reconcile (true|false) in (\S+)$"),
            step_fn(then_architecture_role_classification_json_full_reconcile),
        )
        .then(
            None,
            regex(r"^architecture role classification JSON reports affected path count ([0-9]+) in (\S+)$"),
            step_fn(then_architecture_role_classification_json_affected_path_count),
        )
        .then(
            None,
            regex(r"^architecture role classification JSON includes stale assignment metric in (\S+)$"),
            step_fn(then_architecture_role_classification_json_includes_stale_assignment_metric),
        )
        .then(
            None,
            regex(r"^checkpointed captured commits are ordered in (\S+)$"),
            step_fn(then_captured_commit_history_is_ordered),
        )
        .then(
            None,
            regex(r"^claude-code session exists in (\S+)$"),
            step_fn(then_claude_session_exists),
        )
        .then(
            None,
            regex(
                r"^(claude-code|cursor|gemini|copilot|codex|opencode|open-code) interaction exists before commit in (\S+)$",
            ),
            step_fn(then_agent_interaction_exists_before_commit),
        )
        .then(
            None,
            regex(r"^(cursor|gemini|copilot|codex|opencode|open-code) session exists in (\S+)$"),
            step_fn(then_agent_session_exists),
        )
        .then(
            None,
            regex(r"^checkpoint mapping exists in (\S+)$"),
            step_fn(then_checkpoint_mapping_exists),
        )
        .then(
            None,
            regex(r"^checkpoint mapping count is at least (\d+) in (\S+)$"),
            step_fn(then_checkpoint_mapping_count_at_least),
        )
        .then(
            None,
            regex(r"^DevQL artefacts query returns results in (\S+)$"),
            step_fn(then_devql_artefacts_returns_results),
        )
        .then(
            None,
            regex(r#"^DevQL selectArtefacts search for \"([^\"]+)\" returns symbol \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_devql_select_artefacts_search_returns_symbol),
        )
        .then(
            None,
            regex(r#"^Architecture graph entry point kind \"([^\"]+)\" for path \"([^\"]+)\" is effective in (\S+)$"#),
            step_fn(then_architecture_entry_point_effective),
        )
        .then(
            None,
            regex(r#"^Architecture graph container kind \"([^\"]+)\" exposes entry point kind \"([^\"]+)\" for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_container_exposes_entry_point),
        )
        .then(
            None,
            regex(r#"^Architecture graph system membership \"([^\"]+)\" includes entry point kind \"([^\"]+)\" for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_system_membership_for_entry_point),
        )
        .then(
            None,
            regex(r#"^Architecture graph suppression hides entry point kind \"([^\"]+)\" for path \"([^\"]+)\" then revoke restores it in (\S+)$"#),
            step_fn(then_architecture_suppression_revoke_roundtrip),
        )
        .then(
            None,
            regex(r#"^Architecture graph assertion adds entry point kind \"([^\"]+)\" for path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_architecture_manual_entry_point),
        )
        .then(
            None,
            regex(r#"^DevQL selectArtefacts search for \"([^\"]+)\" returns at least (\d+) results? in (\S+)$"#),
            step_fn(then_devql_select_artefacts_search_returns_at_least),
        )
        .then(
            None,
            regex(r#"^DevQL checkpoints query returns results for \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_devql_checkpoints_returns_results),
        )
        .then(
            None,
            regex(r"^DevQL chatHistory query returns results in (\S+)$"),
            step_fn(then_devql_chat_history_returns_results),
        )
        .then(
            None,
            regex(r#"^DevQL deps query for \"([^\"]+)\" with direction \"([^\"]+)\" returns at least (\d+) results? in (\S+)$"#),
            step_fn(then_devql_deps_returns_at_least),
        )
        .then(
            None,
            regex(r#"^DevQL deps query for \"([^\"]+)\" with direction \"([^\"]+)\" and asOf latest commit returns at least (\d+) results? in (\S+)$"#),
            step_fn(then_devql_deps_as_of_latest_commit),
        )
        .then(
            None,
            regex(r#"^DevQL deps query for \"([^\"]+)\" with direction \"([^\"]+)\" and asOf previous commit returns exactly (\d+) results? in (\S+)$"#),
            step_fn(then_devql_deps_as_of_previous_commit_exact),
        )
        .then(
            None,
            regex(r"^DevQL artefacts query result count is stable across ingests in (\S+)$"),
            step_fn(then_devql_artefacts_stable),
        )
        .then(
            None,
            regex(r#"^DevQL context guidance query for \"([^\"]+)\" returns at least (\d+) items? in (\S+)$"#),
            step_fn(then_devql_context_guidance_returns_at_least),
        )
        .then(
            None,
            regex(r#"^DevQL context guidance query for \"([^\"]+)\" includes kind \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_devql_context_guidance_includes_kind),
        )
        .then(
            None,
            regex(r"^daemon enrichments eventually drain in (\S+)$"),
            step_fn(then_daemon_enrichments_eventually_drain),
        )
        .then(
            None,
            regex(
                r#"^(?:TestHarness|TestLens) query for \"([^\"]+)\" at (latest commit|current workspace state) with view \"([^\"]+)\" returns results in (\S+)$"#,
            ),
            step_fn(then_testlens_query_returns_results),
        )
        .then(
            None,
            regex(r"^(?:TestHarness|TestLens) summary shows non-zero test count in (\S+)$"),
            step_fn(then_testlens_summary_nonzero),
        )
        .then(
            None,
            regex(
                r"^(?:TestHarness|TestLens) tests include at least 1 test with a classification in (\S+)$",
            ),
            step_fn(then_testlens_tests_have_classification),
        )
        .then(
            None,
            regex(r"^(?:TestHarness|TestLens) coverage shows line coverage percentage in (\S+)$"),
            step_fn(then_testlens_coverage_has_line_pct),
        )
        .then(
            None,
            regex(
                r#"^(?:TestHarness|TestLens) query for \"([^\"]+)\" at (latest commit|current workspace state) with view \"([^\"]+)\" returns empty or zero-count in (\S+)$"#,
            ),
            step_fn(then_testlens_query_empty_or_zero),
        )
        .then(
            None,
            regex(
                r#"^(?:TestHarness|TestLens) query for \"([^\"]+)\" at (latest commit|current workspace state) with view \"([^\"]+)\" includes a failing test in (\S+)$"#,
            ),
            step_fn(then_testlens_includes_failing_test),
        )
        .then(
            None,
            regex(r"^daemon capability-event status shows TestHarness sync handler completed in (\S+)$"),
            step_fn(then_daemon_capability_event_status_test_harness_completed),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" returns at least (\d+) results? in (\S+)$"#),
            step_fn(then_devql_clones_returns_at_least),
        )
        .then(
            None,
            regex(r"^semantic clone historical tables are populated in (\S+)$"),
            step_fn(then_semantic_clone_historical_tables_populated),
        )
        .then(
            None,
            regex(r"^semantic clone current projection tables are populated in (\S+)$"),
            step_fn(then_semantic_clone_current_tables_populated),
        )
        .then(
            None,
            regex(r"^semantic clone ingest does not populate historical semantic tables in (\S+)$"),
            step_fn(then_semantic_clone_ingest_skips_historical_semantic_tables),
        )
        .then(
            None,
            regex(r"^semantic clone historical and current embeddings expose code and summary channels in (\S+)$"),
            step_fn(then_semantic_clone_representation_channels_populated),
        )
        .then(
            None,
            regex(r"^semantic clone current embeddings expose code and summary channels in (\S+)$"),
            step_fn(then_current_semantic_clone_representation_channels_populated),
        )
        .then(
            None,
            regex(r"^semantic clone enrichments show embeddings before clone-edge rebuild work fully drains in (\S+)$"),
            step_fn(then_semantic_clone_progress_observed),
        )
        .then(
            None,
            regex(r"^DevQL clones results include score and relation_kind fields in (\S+)$"),
            step_fn(then_devql_clones_have_score_and_kind),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" ranks \"([^\"]+)\" above \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_devql_clones_rank_target_above),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" with min_score (\S+) returns results in (\S+)$"#),
            step_fn(then_devql_clones_with_min_score),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" with min_score (\S+) excludes \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_devql_clones_with_min_score_excludes_target),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" with min_score (\S+) returns fewer or equal results in (\S+)$"#),
            step_fn(then_devql_clones_fewer_or_equal),
        )
        .then(
            None,
            regex(r#"^DevQL clone summary for \"([^\"]+)\" with min_score (\S+) returns grouped counts in (\S+)$"#),
            step_fn(then_devql_clone_summary_grouped_counts),
        )
        .then(
            None,
            regex(r#"^GraphQL clone summary for \"([^\"]+)\" with min_score (\S+) returns grouped counts in (\S+)$"#),
            step_fn(then_graphql_clone_summary_grouped_counts),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" has highest-scored result with score above (\S+) in (\S+)$"#),
            step_fn(then_devql_clones_top_score_above),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" returns results with explanation data in (\S+)$"#),
            step_fn(then_devql_clones_have_explanation),
        )
        .then(
            None,
            regex(r"^the knowledge add command fails with an error in (\S+)$"),
            step_fn(then_last_command_failed),
        )
        .then(
            None,
            regex(r"^DevQL knowledge query returns at least (\d+) items? in (\S+)$"),
            step_fn(then_devql_knowledge_count_at_least),
        )
        .then(
            None,
            regex(r"^DevQL knowledge query returns (\d+) items in (\S+)$"),
            step_fn(then_devql_knowledge_exact_count),
        )
        .then(
            None,
            regex(r#"^knowledge item has provider \"([^\"]+)\" and source_kind \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_knowledge_provider_and_kind),
        )
        .then(
            None,
            regex(r"^knowledge item is associated to a commit in (\S+)$"),
            step_fn(then_knowledge_has_commit_association),
        )
        .then(
            None,
            regex(r#"^knowledge \"([^\"]+)\" is associated to knowledge \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_knowledge_associated_to_knowledge),
        )
        .then(
            None,
            regex(r#"^knowledge versions for \"([^\"]+)\" shows exactly (\d+) versions? in (\S+)$"#),
            step_fn(then_knowledge_versions_count),
        )
        .then(
            None,
            regex(r"^DevQL task id is captured in (\S+)$"),
            step_fn(then_devql_task_id_captured),
        )
        .then(
            None,
            regex(r#"^the last DevQL task kind is \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_last_devql_task_kind_is),
        )
        .then(
            None,
            regex(r#"^DevQL task queue state is \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_devql_task_queue_state),
        )
        .then(
            None,
            regex(r#"^DevQL task queue pause reason is \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_devql_task_queue_pause_reason),
        )
        .then(
            None,
            regex(r"^DevQL tasks list includes the last task in (\S+)$"),
            step_fn(then_devql_tasks_list_includes_last_task),
        )
        .then(
            None,
            regex(r#"^the last DevQL task has status \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_last_devql_task_has_status),
        )
        .then(
            None,
            regex(r"^DevQL sync validation reports clean in (\S+)$"),
            step_fn(then_sync_validation_clean),
        )
        .then(
            None,
            regex(r"^DevQL sync validation reports drift in (\S+)$"),
            step_fn(then_sync_validation_drift),
        )
        .then(
            None,
            regex(r"^DevQL sync validation shows expected greater than (\d+) in (\S+)$"),
            step_fn(then_sync_validation_expected_greater_than),
        )
        .then(
            None,
            regex(r"^DevQL sync history shows added greater than 0 for current HEAD in (\S+)$"),
            step_fn(then_sync_history_added_for_current_head),
        )
        .then(
            None,
            regex(r"^DevQL sync history shows changed greater than 0 for current HEAD in (\S+)$"),
            step_fn(then_sync_history_changed_for_current_head),
        )
        .then(
            None,
            regex(r"^DevQL sync history shows removed greater than 0 for current HEAD in (\S+)$"),
            step_fn(then_sync_history_removed_for_current_head),
        )
        .then(
            None,
            regex(r"^DevQL sync history shows artefacts indexed for current HEAD in (\S+)$"),
            step_fn(then_sync_history_artefacts_for_current_head),
        )
        .then(
            None,
            regex(r"^all reachable SHAs are completed in commit_ingest_ledger in (\S+)$"),
            step_fn(then_all_reachable_shas_completed_in_ledger),
        )
        .then(
            None,
            regex(r"^artefacts_current has rows in (\S+)$"),
            step_fn(then_artefacts_current_has_rows),
        )
        .then(
            None,
            regex(r"^expected SHAs are completed in commit_ingest_ledger in (\S+)$"),
            step_fn(then_expected_shas_completed_in_ledger),
        )
        .then(
            None,
            regex(r"^expected SHAs have file_state rows in (\S+)$"),
            step_fn(then_expected_shas_have_file_state_rows),
        )
        .then(
            None,
            regex(r"^expected paths have file_state rows for expected SHAs in (\S+)$"),
            step_fn(then_expected_paths_have_file_state_rows_for_expected_shas),
        )
        .then(
            None,
            regex(r"^exact expected SHAs were newly completed since snapshot in (\S+)$"),
            step_fn(then_exact_expected_shas_newly_completed_since_snapshot),
        )
        .then(
            None,
            regex(r"^no new SHAs were completed since snapshot in (\S+)$"),
            step_fn(then_no_new_completed_shas_since_snapshot),
        )
        .then(
            None,
            regex(r"^completed ledger count is unchanged since snapshot in (\S+)$"),
            step_fn(then_ledger_completed_count_unchanged_since_snapshot),
        )
        .then(
            None,
            regex(r"^artefacts_current count is unchanged since snapshot in (\S+)$"),
            step_fn(then_artefacts_current_count_unchanged_since_snapshot),
        )
        .then(
            None,
            regex(r"^artefacts_current count increased since snapshot in (\S+)$"),
            step_fn(then_artefacts_current_count_increased_since_snapshot),
        )
        .then(
            None,
            regex(r#"^artefacts_current contains path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_artefacts_current_contains_path),
        )
        .given(
            None,
            regex(r#"^artefacts_current does not contain path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_artefacts_current_lacks_path),
        )
        .then(
            None,
            regex(r#"^artefacts_current does not contain path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_artefacts_current_lacks_path),
        )
        .then(
            None,
            regex(r#"^artefacts_current eventually contains path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_artefacts_current_contains_path_eventually),
        )
        .then(
            None,
            regex(r"^DevQL watcher is registered and running in (\S+)$"),
            step_fn(then_devql_watcher_registered_and_running),
        )
        .then(
            None,
            regex(r#"^artefacts_current eventually contains path \"([^\"]+)\" without nudge in (\S+)$"#),
            step_fn(then_artefacts_current_contains_path_eventually_without_nudge),
        )
        .then(
            None,
            regex(r#"^artefacts_current eventually does not contain path \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_artefacts_current_lacks_path_eventually),
        )
        .then(
            None,
            regex(r#"^current-state content id for \"([^\"]+)\" changed since snapshot in (\S+)$"#),
            step_fn(then_current_file_state_content_id_changed_since_snapshot),
        )
        .then(
            None,
            regex(r#"^current-state content id for \"([^\"]+)\" eventually changed since snapshot in (\S+)$"#),
            step_fn(then_current_file_state_content_id_changed_eventually),
        )
        .then(
            None,
            regex(r#"^a completed DevQL sync task with source \"([^\"]+)\" exists in (\S+)$"#),
            step_fn(then_completed_sync_task_source_exists),
        )
        .then(
            None,
            regex(r#"^a completed DevQL sync task with source \"([^\"]+)\" shows (work|added|changed|removed|unchanged|cache hits|cache misses|parse errors) greater than (\d+) since snapshot in (\S+)$"#),
            step_fn(then_completed_sync_task_source_summary_field_greater_than_since_snapshot),
        )
        .then(
            None,
            regex(r#"^no DevQL ingest task with source \"([^\"]+)\" exists since snapshot in (\S+)$"#),
            step_fn(then_no_devql_ingest_task_source_since_snapshot),
        )
        .then(
            None,
            regex(r#"^the latest completed DevQL sync task source is \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_latest_completed_sync_task_source_matches),
        )
        .then(
            None,
            regex(r#"^current-state content id for \"([^\"]+)\" is unchanged since snapshot in (\S+)$"#),
            step_fn(then_current_file_state_content_id_unchanged_since_snapshot),
        )
        .then(
            None,
            regex(
                r"^only latest (\d+) reachable SHAs are completed in commit_ingest_ledger in (\S+)$",
            ),
            step_fn(then_only_latest_reachable_shas_completed),
        )
        .then(
            None,
            regex(r"^DevQL ingest summary shows (\d+) ([a-z_]+) in (\S+)$"),
            step_fn(then_ingest_summary_field_exact),
        )
        .then(
            None,
            regex(r"^rewrite introduces exactly (\d+) new reachable SHAs in (\S+)$"),
            step_fn(then_rewrite_new_shas_count),
        )
        .then(
            None,
            regex(r"^old rewritten SHAs are absent from post-rewrite reachable segment in (\S+)$"),
            step_fn(then_pre_rewrite_shas_absent_from_post_segment),
        )
        .then(
            None,
            regex(r"^rewritten new SHAs are completed in commit_ingest_ledger in (\S+)$"),
            step_fn(then_rewrite_new_shas_completed_in_ledger),
        )
        .then(
            None,
            regex(r"^DevQL sync summary shows (added|changed|removed|unchanged|cache hits|cache misses|parse errors) greater than (\d+) in (\S+)$"),
            step_fn(then_sync_summary_field_greater_than),
        )
        .then(
            None,
            regex(r"^DevQL sync summary shows (\d+) (added|changed|removed|unchanged|cache hits|cache misses|parse errors) in (\S+)$"),
            step_fn(then_sync_summary_field_exact),
        )
        .then(
            None,
            regex(r"^the command fails with exit code non-zero in (\S+)$"),
            step_fn(then_command_fails_nonzero),
        )
        .then(
            None,
            regex(r#"^the command output contains \"([^\"]+)\" in (\S+)$"#),
            step_fn(then_command_output_contains),
        )
}
