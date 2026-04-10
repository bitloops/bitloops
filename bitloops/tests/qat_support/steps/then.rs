use crate::qat_support::helpers;
use crate::qat_support::world::QatWorld;
use cucumber::codegen::LocalBoxFuture;

use super::common::run_step;

pub(super) fn then_bitloops_stores_exist(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "bitloops stores exist",
            helpers::assert_bitloops_stores_exist_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn then_version_output(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "bitloops --version exits 0 and prints a semver version",
            helpers::assert_version_output(world),
        );
    })
}

pub(super) fn then_daemon_config_exists(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "the global daemon config file exists",
            helpers::assert_daemon_config_exists(world),
        );
    })
}

pub(super) fn then_config_has_relational_store(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "the config contains a relational store path",
            helpers::assert_config_has_relational_store(world),
        );
    })
}

pub(super) fn then_config_has_event_store(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "the config contains an event store path",
            helpers::assert_config_has_event_store(world),
        );
    })
}

pub(super) fn then_store_paths_exist(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "the store paths from the config exist on disk",
            helpers::assert_store_paths_exist(world),
        );
    })
}

pub(super) fn then_config_has_blob_store(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "the config contains a blob store path",
            helpers::assert_config_has_blob_store(world),
        );
    })
}

pub(super) fn then_repo_local_path_exists(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let relative_path = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "the repo-local path exists",
            helpers::assert_file_exists_in_repo(world, &repo_name, &relative_path),
        );
    })
}

pub(super) fn then_agent_hooks_exist(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "git hooks exist for the agent",
            helpers::assert_agent_hooks_installed(world, &repo_name, &agent_name),
        );
    })
}

pub(super) fn then_agent_hooks_removed(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "agent hooks are removed",
            helpers::assert_agent_hooks_removed(world, &repo_name, &agent_name),
        );
    })
}

pub(super) fn then_bitloops_binary_not_found(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "bitloops binary is not found",
            helpers::assert_bitloops_binary_removed(world),
        );
    })
}

pub(super) fn then_git_hooks_removed(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "git hooks are removed",
            helpers::assert_git_hooks_removed(world, &repo_name),
        );
    })
}

pub(super) fn then_status_shows_disabled(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "bitloops status shows disabled",
            helpers::assert_status_shows_disabled(world, &repo_name),
        );
    })
}

pub(super) fn then_commit_checkpoints_count(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let min_count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("commit_checkpoints count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "commit_checkpoints count is at least",
            helpers::assert_commit_checkpoints_count(world, &repo_name, min_count),
        );
    })
}

pub(super) fn then_commit_timeline_is_correct(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "commit timeline and contents are correct",
            helpers::assert_init_yesterday_and_final_today_commit_checkpoints_for_repo(
                world, &repo_name,
            ),
        );
    })
}

pub(super) fn then_claude_session_exists(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "claude-code session exists",
            helpers::assert_claude_session_exists_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn then_agent_session_exists(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "agent session exists",
            helpers::assert_agent_session_exists_for_repo(world, &repo_name, &agent_name),
        );
    })
}

pub(super) fn then_checkpoint_mapping_exists(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "checkpoint mapping exists",
            helpers::assert_checkpoint_mapping_exists_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn then_checkpoint_mapping_count_at_least(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let min_count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("checkpoint mapping count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "checkpoint mapping count is at least",
            helpers::assert_checkpoint_mapping_count_at_least_for_repo(
                world, &repo_name, min_count,
            ),
        );
    })
}

pub(super) fn then_devql_artefacts_returns_results(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "DevQL artefacts query returns results",
            helpers::assert_devql_artefacts_query_returns_results(world, &repo_name),
        );
    })
}

pub(super) fn then_devql_checkpoints_returns_results(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "DevQL checkpoints query returns results",
            helpers::assert_devql_checkpoints_query_returns_results(world, &repo_name, &agent),
        );
    })
}

pub(super) fn then_devql_chat_history_returns_results(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "DevQL chatHistory query returns results",
            helpers::assert_devql_chat_history_returns_results(world, &repo_name),
        );
    })
}

pub(super) fn then_devql_deps_returns_at_least(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let direction = ctx.matches[2].1.clone();
        let min_count = ctx.matches[3]
            .1
            .parse::<usize>()
            .expect("min deps count should parse as usize");
        let repo_name = ctx.matches[4].1.clone();
        run_step(
            "DevQL deps query returns at least",
            helpers::assert_devql_deps_query(world, &repo_name, &symbol, &direction, min_count),
        );
    })
}

pub(super) fn then_devql_deps_as_of_latest_commit(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let direction = ctx.matches[2].1.clone();
        let min_count = ctx.matches[3]
            .1
            .parse::<usize>()
            .expect("min deps count should parse as usize");
        let repo_name = ctx.matches[4].1.clone();
        run_step(
            "DevQL deps query asOf latest commit",
            (|| {
                let latest_sha = world
                    .captured_commit_shas
                    .last()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no latest commit SHA captured"))?;
                helpers::assert_devql_deps_query_as_of_commit(
                    world,
                    &repo_name,
                    &symbol,
                    &direction,
                    &latest_sha,
                    min_count,
                )
            })(),
        );
    })
}

pub(super) fn then_devql_deps_as_of_previous_commit_exact(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let direction = ctx.matches[2].1.clone();
        let expected_count = ctx.matches[3]
            .1
            .parse::<usize>()
            .expect("deps expected_count should parse as usize");
        let repo_name = ctx.matches[4].1.clone();
        run_step(
            "DevQL deps query asOf previous commit exact count",
            (|| {
                let previous_sha = world
                    .captured_commit_shas
                    .iter()
                    .rev()
                    .nth(1)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no previous commit SHA captured"))?;
                helpers::assert_devql_deps_query_as_of_commit_exact_count(
                    world,
                    &repo_name,
                    &symbol,
                    &direction,
                    &previous_sha,
                    expected_count,
                )
            })(),
        );
    })
}

pub(super) fn then_devql_artefacts_stable(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "DevQL artefacts query result count is stable across ingests",
            helpers::assert_devql_artefacts_count_stable(world, &repo_name),
        );
    })
}

pub(super) fn then_testlens_query_returns_results(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let artefact = ctx.matches[1].1.clone();
        let state_scope = ctx.matches[2].1.clone();
        let view = ctx.matches[3].1.clone();
        let repo_name = ctx.matches[4].1.clone();
        run_step(
            "TestHarness query returns results",
            helpers::assert_testlens_query_returns_results(
                world,
                &repo_name,
                &artefact,
                &view,
                &state_scope,
            ),
        );
    })
}

pub(super) fn then_testlens_summary_nonzero(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "TestHarness summary shows non-zero test count",
            helpers::assert_testlens_summary_nonzero(world),
        );
    })
}

pub(super) fn then_testlens_tests_have_classification(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "TestHarness tests include at least 1 test with a classification",
            helpers::assert_testlens_tests_have_classification(world),
        );
    })
}

pub(super) fn then_testlens_coverage_has_line_pct(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "TestHarness coverage shows line coverage percentage",
            helpers::assert_testlens_coverage_has_line_pct(world),
        );
    })
}

pub(super) fn then_testlens_query_empty_or_zero(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let artefact = ctx.matches[1].1.clone();
        let state_scope = ctx.matches[2].1.clone();
        let view = ctx.matches[3].1.clone();
        let repo_name = ctx.matches[4].1.clone();
        run_step(
            "TestHarness query returns empty or zero-count",
            helpers::assert_testlens_query_empty_or_zero(
                world,
                &repo_name,
                &artefact,
                &view,
                &state_scope,
            ),
        );
    })
}

pub(super) fn then_testlens_includes_failing_test(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let artefact = ctx.matches[1].1.clone();
        let state_scope = ctx.matches[2].1.clone();
        let view = ctx.matches[3].1.clone();
        let repo_name = ctx.matches[4].1.clone();
        run_step(
            "TestHarness query includes a failing test",
            helpers::assert_testlens_includes_failing_test(
                world,
                &repo_name,
                &artefact,
                &view,
                &state_scope,
            ),
        );
    })
}

pub(super) fn then_daemon_capability_event_status_test_harness_completed(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "daemon capability-event status shows TestHarness sync handler completed",
            helpers::wait_for_test_harness_capability_event_completion_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn then_devql_clones_returns_at_least(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let min_count = ctx.matches[2]
            .1
            .parse::<usize>()
            .expect("min clone count should parse as usize");
        let repo_name = ctx.matches[3].1.clone();
        run_step(
            "DevQL clones query returns at least",
            helpers::assert_devql_clones_query(world, &repo_name, &symbol, min_count),
        );
    })
}

pub(super) fn then_devql_clones_have_score_and_kind(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "DevQL clones results include score and relation_kind fields",
            helpers::assert_devql_clones_have_score_and_kind(world),
        );
    })
}

pub(super) fn then_devql_clones_with_min_score(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let min_score = ctx.matches[2]
            .1
            .parse::<f64>()
            .expect("min_score should parse as f64");
        let repo_name = ctx.matches[3].1.clone();
        run_step(
            "DevQL clones query with min_score returns results",
            helpers::assert_devql_clones_with_min_score(world, &repo_name, &symbol, min_score),
        );
    })
}

pub(super) fn then_devql_clones_fewer_or_equal(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let min_score = ctx.matches[2]
            .1
            .parse::<f64>()
            .expect("min_score should parse as f64");
        let repo_name = ctx.matches[3].1.clone();
        run_step(
            "DevQL clones query with min_score returns fewer or equal results",
            (|| {
                let previous = world
                    .last_query_result_count
                    .ok_or_else(|| anyhow::anyhow!("no previous clone count captured"))?;
                helpers::record_devql_clones_with_min_score(world, &repo_name, &symbol, min_score)?;
                helpers::assert_last_query_fewer_or_equal(world, previous)
            })(),
        );
    })
}

pub(super) fn then_devql_clones_top_score_above(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let threshold = ctx.matches[2]
            .1
            .parse::<f64>()
            .expect("score threshold should parse as f64");
        let repo_name = ctx.matches[3].1.clone();
        run_step(
            "DevQL clones query top score above threshold",
            helpers::assert_devql_clones_top_score_above(world, &repo_name, &symbol, threshold),
        );
    })
}

pub(super) fn then_devql_clones_have_explanation(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "DevQL clones query returns explanation payload",
            helpers::assert_devql_clones_have_explanation(world, &repo_name, &symbol),
        );
    })
}

pub(super) fn then_last_command_failed(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "the knowledge add command fails with an error",
            helpers::assert_last_command_failed(world),
        );
    })
}

pub(super) fn then_devql_knowledge_count_at_least(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let min_count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("knowledge min_count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "DevQL knowledge query returns at least items",
            helpers::assert_devql_knowledge_query_count(world, &repo_name, min_count),
        );
    })
}

pub(super) fn then_devql_knowledge_exact_count(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected_count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("knowledge expected_count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "DevQL knowledge query returns exact items",
            helpers::assert_devql_knowledge_query_exact_count(world, &repo_name, expected_count),
        );
    })
}

pub(super) fn then_knowledge_provider_and_kind(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let provider = ctx.matches[1].1.clone();
        let source_kind = ctx.matches[2].1.clone();
        run_step(
            "knowledge item has provider and source_kind",
            helpers::assert_knowledge_item_provider_and_kind(world, &provider, &source_kind),
        );
    })
}

pub(super) fn then_knowledge_has_commit_association(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "knowledge item is associated to a commit",
            helpers::assert_knowledge_item_has_commit_association(world),
        );
    })
}

pub(super) fn then_knowledge_versions_count(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let input = ctx.matches[1].1.clone();
        let expected_count = ctx.matches[2]
            .1
            .parse::<usize>()
            .expect("knowledge versions expected_count should parse as usize");
        run_step(
            "knowledge versions count matches",
            helpers::assert_knowledge_versions_count(world, &input, expected_count),
        );
    })
}

// ── DevQL sync validation assertions ─────────────────────────

pub(super) fn then_sync_validation_clean(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "DevQL sync validation reports clean",
            (|| -> anyhow::Result<()> {
                let stdout = world.last_command_stdout.as_deref().unwrap_or("");
                anyhow::ensure!(
                    stdout.contains("sync validation: clean"),
                    "expected sync validation to report clean\nstdout: {stdout}"
                );
                Ok(())
            })(),
        );
    })
}

pub(super) fn then_sync_validation_drift(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "DevQL sync validation reports drift",
            (|| -> anyhow::Result<()> {
                let stdout = world.last_command_stdout.as_deref().unwrap_or("");
                anyhow::ensure!(
                    stdout.contains("sync validation: drift detected"),
                    "expected sync validation to report drift detected\nstdout: {stdout}"
                );
                Ok(())
            })(),
        );
    })
}

pub(super) fn then_sync_validation_expected_greater_than(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let min: usize = ctx.matches[1]
            .1
            .parse()
            .expect("parse min for validation expected");
        let _repo_name = ctx.matches[2].1.clone();
        run_step(
            &format!("DevQL sync validation expected > {min}"),
            (|| -> anyhow::Result<()> {
                let stdout = world.last_command_stdout.as_deref().unwrap_or("");
                let expected = helpers::parse_validation_field(stdout, "expected").unwrap_or(0);
                anyhow::ensure!(
                    expected > min,
                    "expected validation expected > {min}, got {expected}\nstdout: {stdout}"
                );
                Ok(())
            })(),
        );
    })
}

pub(super) fn then_sync_history_added_for_current_head(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "DevQL sync history shows added > 0 for current HEAD",
            helpers::assert_sync_history_has_added_for_current_head(world, &repo_name),
        );
    })
}

pub(super) fn then_sync_history_changed_for_current_head(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "DevQL sync history shows changed > 0 for current HEAD",
            helpers::assert_sync_history_has_changed_for_current_head(world, &repo_name),
        );
    })
}

pub(super) fn then_sync_history_removed_for_current_head(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "DevQL sync history shows removed > 0 for current HEAD",
            helpers::assert_sync_history_has_removed_for_current_head(world, &repo_name),
        );
    })
}

pub(super) fn then_sync_history_artefacts_for_current_head(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "DevQL sync history shows artefacts indexed for current HEAD",
            helpers::assert_sync_history_has_artefacts_for_current_head(world, &repo_name),
        );
    })
}

// ── DevQL sync/ingest summary assertions ─────────────────────

pub(super) fn then_all_reachable_shas_completed_in_ledger(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "all reachable SHAs are completed in commit_ingest_ledger",
            helpers::assert_all_reachable_shas_completed_in_ledger(world, &repo_name),
        );
    })
}

pub(super) fn then_artefacts_current_has_rows(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "artefacts_current has rows",
            helpers::assert_artefacts_current_has_rows(world, &repo_name),
        );
    })
}

pub(super) fn then_expected_shas_completed_in_ledger(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "expected SHAs are completed in commit_ingest_ledger",
            helpers::assert_expected_shas_completed_in_ledger(world, &repo_name),
        );
    })
}

pub(super) fn then_expected_shas_have_file_state_rows(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "expected SHAs have file_state rows",
            helpers::assert_expected_shas_have_file_state_rows(world, &repo_name),
        );
    })
}

pub(super) fn then_exact_expected_shas_newly_completed_since_snapshot(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "exact expected SHAs were newly completed since snapshot",
            helpers::assert_exact_expected_shas_newly_completed_since_snapshot(world, &repo_name),
        );
    })
}

pub(super) fn then_no_new_completed_shas_since_snapshot(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "no new SHAs were completed since snapshot",
            helpers::assert_no_new_completed_shas_since_snapshot(world, &repo_name),
        );
    })
}

pub(super) fn then_ledger_completed_count_unchanged_since_snapshot(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "completed ledger count is unchanged since snapshot",
            helpers::assert_ledger_completed_count_unchanged_since_snapshot(world, &repo_name),
        );
    })
}

pub(super) fn then_artefacts_current_count_unchanged_since_snapshot(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "artefacts_current count is unchanged since snapshot",
            helpers::assert_artefacts_current_count_unchanged_since_snapshot(world, &repo_name),
        );
    })
}

pub(super) fn then_artefacts_current_count_increased_since_snapshot(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "artefacts_current count increased since snapshot",
            helpers::assert_artefacts_current_count_increased_since_snapshot(world, &repo_name),
        );
    })
}

pub(super) fn then_artefacts_current_contains_path(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "artefacts_current contains path",
            helpers::assert_artefacts_current_contains_path(world, &repo_name, &path),
        );
    })
}

pub(super) fn then_only_latest_reachable_shas_completed(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("latest reachable count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "only latest reachable SHAs are completed in commit_ingest_ledger",
            helpers::assert_only_latest_reachable_shas_completed_in_ledger(
                world, &repo_name, count,
            ),
        );
    })
}

pub(super) fn then_ingest_summary_field_exact(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected: usize = ctx.matches[1]
            .1
            .parse()
            .expect("expected ingest summary value should parse");
        let field = ctx.matches[2].1.clone();
        let _repo_name = ctx.matches[3].1.clone();
        run_step(
            &format!("DevQL ingest summary `{field}` == {expected}"),
            (|| -> anyhow::Result<()> {
                let stdout = world.last_command_stdout.as_deref().unwrap_or("");
                let value = helpers::parse_ingest_summary_field(stdout, &field).ok_or_else(|| {
                    anyhow::anyhow!(
                        "missing ingest summary field `{field}` while expecting value {expected}\nstdout: {stdout}"
                    )
                })?;
                anyhow::ensure!(
                    value == expected,
                    "expected ingest summary `{field}` == {expected}, got {value}\nstdout: {stdout}"
                );
                Ok(())
            })(),
        );
    })
}

pub(super) fn then_rewrite_new_shas_count(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("expected rewrite SHA count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "rewrite introduces expected number of new SHAs",
            helpers::assert_rewrite_new_shas_count(world, &repo_name, expected),
        );
    })
}

pub(super) fn then_pre_rewrite_shas_absent_from_post_segment(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "old rewritten SHAs are absent from post-rewrite segment",
            helpers::assert_pre_rewrite_shas_absent_from_post_segment(world, &repo_name),
        );
    })
}

pub(super) fn then_rewrite_new_shas_completed_in_ledger(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "rewritten new SHAs are completed in commit_ingest_ledger",
            helpers::assert_rewrite_new_shas_completed_in_ledger(world, &repo_name),
        );
    })
}

pub(super) fn then_sync_summary_field_greater_than(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let field = ctx.matches[1].1.clone();
        let min: usize = ctx.matches[2]
            .1
            .parse()
            .expect("parse min count for sync summary");
        let _repo_name = ctx.matches[3].1.clone();
        run_step(
            &format!("DevQL sync summary `{field}` > {min}"),
            (|| -> anyhow::Result<()> {
                let stdout = world.last_command_stdout.as_deref().unwrap_or("");
                let value = helpers::parse_sync_summary_field(stdout, &field).unwrap_or(0);
                anyhow::ensure!(
                    value > min,
                    "expected sync summary `{field}` > {min}, got {value}\nstdout: {stdout}"
                );
                Ok(())
            })(),
        );
    })
}

pub(super) fn then_sync_summary_field_exact(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let expected: usize = ctx.matches[1]
            .1
            .parse()
            .expect("parse expected count for sync summary");
        let field = ctx.matches[2].1.clone();
        let _repo_name = ctx.matches[3].1.clone();
        run_step(
            &format!("DevQL sync summary `{field}` == {expected}"),
            (|| -> anyhow::Result<()> {
                let stdout = world.last_command_stdout.as_deref().unwrap_or("");
                let value = helpers::parse_sync_summary_field(stdout, &field).unwrap_or(0);
                anyhow::ensure!(
                    value == expected,
                    "expected sync summary `{field}` == {expected}, got {value}\nstdout: {stdout}"
                );
                Ok(())
            })(),
        );
    })
}

pub(super) fn then_command_fails_nonzero(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "command fails with non-zero exit",
            (|| -> anyhow::Result<()> {
                let exit_code = world.last_command_exit_code.unwrap_or(0);
                anyhow::ensure!(
                    exit_code != 0,
                    "expected non-zero exit code, got {exit_code}"
                );
                Ok(())
            })(),
        );
    })
}

pub(super) fn then_command_output_contains(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let text = ctx.matches[1].1.clone();
        let _repo_name = ctx.matches[2].1.clone();
        run_step(
            &format!("command output contains \"{text}\""),
            (|| -> anyhow::Result<()> {
                let stdout = world.last_command_stdout.as_deref().unwrap_or("");
                anyhow::ensure!(
                    stdout.contains(&text),
                    "expected command output to contain \"{text}\"\nstdout: {stdout}"
                );
                Ok(())
            })(),
        );
    })
}
