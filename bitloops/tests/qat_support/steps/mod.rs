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
            regex(r"^I run CleanStart$"),
            step_fn(given_default_clean_start),
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
            regex(r"^I run bitloops init --agent (\S+) --force in (\S+)$"),
            step_fn(given_init_bitloops_with_agent_force),
        )
        .given(
            None,
            regex(r"^I run EnableCLI for (\S+)$"),
            step_fn(given_enable_cli),
        )
        .given(
            None,
            regex(r"^I run EnableCLIs for (\S+)$"),
            step_fn(given_enable_cli),
        )
        .given(
            None,
            regex(r"^I run bitloops enable in (\S+)$"),
            step_fn(given_enable),
        )
        .given(
            None,
            regex(r"^I run bitloops enable --project in (\S+)$"),
            step_fn(given_enable_project),
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
            regex(r"^I simulate a codex checkpoint in (\S+)$"),
            step_fn(given_simulate_codex_checkpoint),
        )
        .given(
            None,
            regex(r"^I simulate a claude checkpoint in (\S+)$"),
            step_fn(given_simulate_claude_checkpoint),
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
            regex(r"^I run TestLens ingest-tests for latest commit in (\S+)$"),
            step_fn(given_testlens_ingest_tests),
        )
        .given(
            None,
            regex(r"^I run TestLens ingest-coverage for latest commit in (\S+)$"),
            step_fn(given_testlens_ingest_coverage),
        )
        .given(
            None,
            regex(r"^I run TestLens ingest-tests at HEAD in (\S+)$"),
            step_fn(given_testlens_ingest_tests_at_head),
        )
        .given(
            None,
            regex(r"^I run TestLens ingest-coverage at HEAD in (\S+)$"),
            step_fn(given_testlens_ingest_coverage_at_head),
        )
        .given(
            None,
            regex(r"^I run TestLens ingest-results with a failing test for latest commit in (\S+)$"),
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
            regex(r"^the repo-local \.bitloops directory exists in (\S+)$"),
            step_fn(then_repo_local_bitloops_dir_exists),
        )
        .then(
            None,
            regex(r#"^the repo-local path \"([^\"]+)\" exists in (\S+)$"#),
            step_fn(then_repo_local_path_exists),
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
            regex(r"^DevQL ingest reports checkpoints_processed=0$"),
            step_fn(then_ingest_reports_zero_checkpoints),
        )
        .then(
            None,
            regex(r"^bitloops daemon stop exits 0$"),
            step_fn(then_daemon_stop_exits_zero),
        )
        .then(
            None,
            regex(r"^commit_checkpoints count is at least (\d+) in (\S+)$"),
            step_fn(then_commit_checkpoints_count),
        )
        .then(
            None,
            regex(r"^coverage_captures count is at least (\d+) in (\S+)$"),
            step_fn(then_coverage_captures_count),
        )
        .then(
            None,
            regex(r"^coverage_hits count is at least (\d+) in (\S+)$"),
            step_fn(then_coverage_hits_count),
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
            regex(r#"^DevQL deps query for \"([^\"]+)\" with direction \"([^\"]+)\" and asOf previous commit returns at least (\d+) results? in (\S+)$"#),
            step_fn(then_devql_deps_as_of_previous_commit),
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
            regex(r#"^TestLens query for \"([^\"]+)\" at latest commit with view \"([^\"]+)\" returns results in (\S+)$"#),
            step_fn(then_testlens_query_returns_results),
        )
        .then(
            None,
            regex(r"^TestLens summary shows non-zero test count in (\S+)$"),
            step_fn(then_testlens_summary_nonzero),
        )
        .then(
            None,
            regex(r"^TestLens tests include at least 1 test with a classification in (\S+)$"),
            step_fn(then_testlens_tests_have_classification),
        )
        .then(
            None,
            regex(r"^TestLens coverage shows line coverage percentage in (\S+)$"),
            step_fn(then_testlens_coverage_has_line_pct),
        )
        .then(
            None,
            regex(r#"^TestLens query for \"([^\"]+)\" at latest commit with view \"([^\"]+)\" returns empty or zero-count in (\S+)$"#),
            step_fn(then_testlens_query_empty_or_zero),
        )
        .then(
            None,
            regex(r#"^TestLens query for \"([^\"]+)\" at latest commit with view \"([^\"]+)\" includes a failing test in (\S+)$"#),
            step_fn(then_testlens_includes_failing_test),
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
}
