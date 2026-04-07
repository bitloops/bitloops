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
            regex(r"^I run EnableCLI for (\S+)$"),
            step_fn(given_enable_cli),
        )
        .given(
            None,
            regex(r"^I run bitloops enable in (\S+)$"),
            step_fn(given_enable),
        )
        .given(
            None,
            regex(r"^I run bitloops disable in (\S+)$"),
            step_fn(given_disable),
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
            regex(r#"^I ask Claude Code to "([^"]+)" in (\S+)$"#),
            step_fn(given_claude_code_prompt),
        )
        .given(
            None,
            regex(r"^I make a second change using Claude Code to (\S+)$"),
            step_fn(given_second_claude_change),
        )
        .given(
            None,
            regex(r"^I committed yesterday in (\S+)$"),
            step_fn(given_commit_yesterday),
        )
        .given(
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
            regex(r"^I run DevQL ingest in (\S+)$"),
            step_fn(given_devql_ingest),
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
            regex(r"^I run DevQL semantic clones rebuild in (\S+)$"),
            step_fn(given_devql_semantic_clones_rebuild),
        )
        .given(
            None,
            regex(r"^I create a simple Rust project in (\S+)$"),
            step_fn(given_create_simple_rust_project),
        )
        .given(
            None,
            regex(r"^I run DevQL sync(?: --status)? in (\S+)$"),
            step_fn(given_devql_sync),
        )
        .given(
            None,
            regex(r"^I run DevQL sync validate(?: --status)? in (\S+)$"),
            step_fn(given_devql_sync_validate),
        )
        .given(
            None,
            regex(r"^I run DevQL sync repair(?: --status)? in (\S+)$"),
            step_fn(given_devql_sync_repair),
        )
        .given(
            None,
            regex(r"^I attempt to run DevQL sync in (\S+)$"),
            step_fn(given_attempt_devql_sync),
        )
        .given(
            None,
            regex(r"^I add a new source file in (\S+)$"),
            step_fn(given_add_new_source_file),
        )
        .given(
            None,
            regex(r"^I modify an existing source file in (\S+)$"),
            step_fn(given_modify_existing_source_file),
        )
        .given(
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
            regex(r"^git hooks exist for the (\S+) agent in (\S+)$"),
            step_fn(then_agent_hooks_exist),
        )
        .then(
            None,
            regex(r"^bitloops binary is not found$"),
            step_fn(then_bitloops_binary_not_found),
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
            regex(r"^commit timeline and contents are correct in (\S+)$"),
            step_fn(then_commit_timeline_is_correct),
        )
        .then(
            None,
            regex(r"^claude-code session exists in (\S+)$"),
            step_fn(then_claude_session_exists),
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
            regex(
                r#"^(?:TestHarness|TestLens) query for \"([^\"]+)\" at (?:latest commit|current workspace state) with view \"([^\"]+)\" returns results in (\S+)$"#,
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
                r#"^(?:TestHarness|TestLens) query for \"([^\"]+)\" at (?:latest commit|current workspace state) with view \"([^\"]+)\" returns empty or zero-count in (\S+)$"#,
            ),
            step_fn(then_testlens_query_empty_or_zero),
        )
        .then(
            None,
            regex(
                r#"^(?:TestHarness|TestLens) query for \"([^\"]+)\" at (?:latest commit|current workspace state) with view \"([^\"]+)\" includes a failing test in (\S+)$"#,
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
            regex(r"^DevQL clones results include score and relation_kind fields in (\S+)$"),
            step_fn(then_devql_clones_have_score_and_kind),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" with min_score (\S+) returns results in (\S+)$"#),
            step_fn(then_devql_clones_with_min_score),
        )
        .then(
            None,
            regex(r#"^DevQL clones query for \"([^\"]+)\" with min_score (\S+) returns fewer or equal results in (\S+)$"#),
            step_fn(then_devql_clones_fewer_or_equal),
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
            regex(r#"^knowledge versions for \"([^\"]+)\" shows exactly (\d+) versions? in (\S+)$"#),
            step_fn(then_knowledge_versions_count),
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
