use crate::qat_support::helpers;
use crate::qat_support::world::QatWorld;
use cucumber::codegen::LocalBoxFuture;

use super::common::run_step;

pub(super) fn given_clean_start(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let flow_name = ctx.matches[1].1.clone();
        run_step(
            "I run CleanStart for flow",
            helpers::run_clean_start(world, &flow_name),
        );
    })
}

pub(super) fn given_start_daemon(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I start the daemon",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::ensure_daemon_for_scenario(world)),
        );
    })
}

pub(super) fn given_init_commit(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run InitCommit",
            helpers::run_init_commit_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_init_commit_without_post_commit_refresh(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run InitCommit without post-commit refresh",
            helpers::run_init_commit_without_post_commit_refresh_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_init_commit_yesterday(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I ran InitCommit yesterday",
            helpers::run_init_commit_with_relative_day_for_repo(world, &repo_name, 1),
        );
    })
}

pub(super) fn given_create_vite_app(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a Vite app project",
            helpers::run_create_vite_app_project_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_init_bitloops(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I init bitloops",
            helpers::run_init_bitloops_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_init_bitloops_with_agent(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I run bitloops init --agent",
            helpers::run_init_bitloops_with_agent(world, &repo_name, &agent_name, false, None),
        );
    })
}

pub(super) fn given_init_bitloops_with_agents(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let first_agent_name = ctx.matches[1].1.clone();
        let second_agent_name = ctx.matches[2].1.clone();
        let repo_name = ctx.matches[3].1.clone();
        run_step(
            "I run bitloops init with agents",
            helpers::run_init_bitloops_with_agents(
                world,
                &repo_name,
                &[first_agent_name.as_str(), second_agent_name.as_str()],
                false,
                None,
            ),
        );
    })
}

pub(super) fn given_init_bitloops_with_agent_sync_false(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I run bitloops init --agent --sync=false",
            helpers::run_init_bitloops_with_agent(
                world,
                &repo_name,
                &agent_name,
                false,
                Some(false),
            ),
        );
    })
}

pub(super) fn given_init_bitloops_with_agent_sync_true(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I run bitloops init --agent --sync=true",
            helpers::run_init_bitloops_with_agent(
                world,
                &repo_name,
                &agent_name,
                false,
                Some(true),
            ),
        );
    })
}

pub(super) fn given_init_bitloops_with_agent_sync_false_ingest_true_backfill(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let backfill = ctx.matches[2]
            .1
            .parse::<usize>()
            .expect("backfill should parse as usize");
        let repo_name = ctx.matches[3].1.clone();
        run_step(
            "I run bitloops init --agent --sync=false --ingest=true --backfill",
            helpers::run_init_bitloops_with_agent_sync_ingest_backfill(
                world,
                &repo_name,
                &agent_name,
                false,
                true,
                backfill,
            ),
        );
    })
}

pub(super) fn given_enable_capture(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run bitloops enable --capture",
            helpers::run_bitloops_enable_with_flags(world, &repo_name, &["--capture"]),
        );
    })
}

pub(super) fn given_disable_capture_and_devql_guidance(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run bitloops disable --capture --devql-guidance",
            helpers::run_bitloops_disable_with_flags(
                world,
                &repo_name,
                &["--capture", "--devql-guidance"],
            ),
        );
    })
}

pub(super) fn given_uninstall_full(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run bitloops uninstall full",
            helpers::run_bitloops_uninstall_full(world, &repo_name),
        );
    })
}

pub(super) fn given_uninstall_hooks(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run bitloops uninstall hooks",
            helpers::run_bitloops_uninstall_hooks(world, &repo_name),
        );
    })
}

pub(super) fn given_claude_auth(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I ensure Claude Code auth",
            helpers::ensure_claude_auth_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_first_claude_change(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I make a first change using Claude Code",
            helpers::run_first_change_using_claude_code_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_first_agent_change(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I make a first change using an agent",
            helpers::run_first_change_using_agent_for_repo(world, &repo_name, &agent_name),
        );
    })
}

pub(super) fn given_claude_code_prompt(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let prompt = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I ask Claude Code to make a change",
            helpers::run_claude_code_prompt_for_repo(world, &repo_name, &prompt),
        );
    })
}

pub(super) fn given_supported_agent_prompt(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let prompt = ctx.matches[2].1.clone();
        let repo_name = ctx.matches[3].1.clone();
        run_step(
            "I ask a supported agent to make a change",
            helpers::run_agent_prompt_for_repo(world, &repo_name, &agent_name, &prompt),
        );
    })
}

pub(super) fn given_second_claude_change(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I make a second change using Claude Code",
            helpers::run_second_change_using_claude_code_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_second_agent_change(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I make a second change using an agent",
            helpers::run_second_change_using_agent_for_repo(world, &repo_name, &agent_name),
        );
    })
}

pub(super) fn given_commit_yesterday(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I committed yesterday",
            helpers::commit_for_relative_day_for_repo(
                world,
                &repo_name,
                1,
                "test: committed yesterday",
            ),
        );
    })
}

pub(super) fn given_commit_today(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I committed today",
            helpers::commit_for_relative_day_for_repo(
                world,
                &repo_name,
                0,
                "test: committed today",
            ),
        );
    })
}

pub(super) fn given_devql_init(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run DevQL init",
            helpers::run_devql_init_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_enable_watcher_autostart(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I enable watcher autostart",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::enable_watcher_autostart_for_scenario(world)),
        );
    })
}

pub(super) fn given_enqueue_devql_ingest_task_with_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I enqueue DevQL ingest task with status",
            helpers::enqueue_devql_ingest_task_with_status_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_snapshot_ingest_db_state(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I snapshot ingest DB state",
            helpers::snapshot_ingest_db_state_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_create_ingest_commits(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("commit count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I create ingest commits",
            helpers::create_ingest_commits_for_repo(world, &repo_name, count),
        );
    })
}

pub(super) fn given_non_ff_merge_with_two_feature_commits(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a non-FF merge with 2 feature commits",
            helpers::create_non_ff_merge_with_two_feature_commits_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_ff_merge_with_two_feature_commits(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create an FF merge with 2 feature commits",
            helpers::create_ff_merge_with_two_feature_commits_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_cherry_pick_two_commits(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I cherry-pick 2 commits",
            helpers::cherry_pick_two_commits_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_capture_top_reachable_before_rewrite(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("rewrite capture count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I capture top reachable SHAs before rewrite",
            helpers::capture_top_reachable_shas_before_rewrite_for_repo(world, &repo_name, count),
        );
    })
}

pub(super) fn given_rebase_edit_rewrite_last_commits(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("rebase rewrite count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I rewrite commits with rebase edit",
            helpers::rewrite_last_commits_with_rebase_edit_for_repo(world, &repo_name, count),
        );
    })
}

pub(super) fn given_reset_and_rewrite_last_commits(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let count = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("reset rewrite count should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I reset and rewrite commits",
            helpers::reset_and_rewrite_last_commits_for_repo(world, &repo_name, count),
        );
    })
}

pub(super) fn given_create_ts_deps_project(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a TypeScript project with known dependencies",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::create_ts_project_with_known_deps(world.repo_dir())),
        );
    })
}

pub(super) fn given_add_new_caller(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let symbol = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I add a new caller",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::add_new_caller_of_symbol(world, &symbol)),
        );
    })
}

pub(super) fn given_create_ts_test_project(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a TypeScript project with tests and coverage",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::create_ts_project_with_tests_and_coverage(world.repo_dir())),
        );
    })
}

pub(super) fn given_create_rust_project_with_tests(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a Rust project with tests",
            helpers::create_rust_project_with_tests(world, &repo_name),
        );
    })
}

pub(super) fn given_testlens_ingest_tests(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run TestHarness ingest-tests",
            helpers::run_testlens_ingest_tests(world, &repo_name),
        );
    })
}

pub(super) fn given_testlens_ingest_coverage(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run TestHarness ingest-coverage",
            helpers::run_testlens_ingest_coverage(world, &repo_name),
        );
    })
}

pub(super) fn given_testlens_ingest_results_failing(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run TestHarness ingest-results with a failing test",
            helpers::run_testlens_ingest_results(
                world,
                &repo_name,
                "test-results/jest-results-fail.json",
            ),
        );
    })
}

pub(super) fn given_create_ts_similar_project(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a TypeScript project with similar implementations",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::create_ts_project_with_similar_impls(world.repo_dir())),
        );
    })
}

pub(super) fn given_create_ts_semantic_clone_quality_project(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a TypeScript project with semantic clone quality fixtures",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::create_ts_project_with_similar_impls(world.repo_dir())),
        );
    })
}

pub(super) fn given_add_semantic_clone_fixtures(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I add semantic clone fixtures",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::add_semantic_clone_fixtures(world.repo_dir())),
        );
    })
}

pub(super) fn given_modify_semantic_clone_fixture_source(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I modify a semantic clone fixture source file",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::modify_semantic_clone_fixture_source(world.repo_dir())),
        );
    })
}

pub(super) fn given_configure_semantic_clones_guide_aligned_fake_runtime(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I configure guide-aligned semantic clones with fake embeddings runtime",
            helpers::configure_semantic_clones_with_guide_aligned_fake_runtime(world, &repo_name),
        );
    })
}

pub(super) fn given_configure_semantic_clones_fake_runtime(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I configure semantic clones with fake embeddings runtime",
            helpers::configure_semantic_clones_with_fake_runtime(world, &repo_name),
        );
    })
}

pub(super) fn given_configure_context_guidance_fake_runtime(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I configure context guidance with fake text-generation runtime",
            helpers::configure_context_guidance_with_fake_runtime(world, &repo_name),
        );
    })
}

pub(super) fn given_daemon_enrichments_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run daemon enrichments status",
            helpers::run_daemon_enrichments_status(world, &repo_name).map(|_| ()),
        );
    })
}

pub(super) fn given_wait_semantic_clone_enrichments_to_drain(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I wait for semantic clone enrichments to drain",
            helpers::wait_for_semantic_clone_enrichments_to_drain(world, &repo_name),
        );
    })
}

pub(super) fn given_devql_semantic_clones_pack_health_ready(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "DevQL pack health for semantic clones is ready",
            helpers::assert_semantic_clones_pack_health_ready(world, &repo_name),
        );
    })
}

pub(super) fn given_devql_semantic_clones_rebuild(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run DevQL semantic clones rebuild",
            helpers::run_devql_semantic_clones_rebuild(world, &repo_name),
        );
    })
}

pub(super) fn given_knowledge_add(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I add knowledge URL",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::run_knowledge_add(world, &url)),
        );
    })
}

pub(super) fn given_configure_deterministic_confluence_knowledge_fixtures(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I configure deterministic Confluence knowledge fixtures",
            helpers::ensure_bitloops_repo_name(&repo_name).and_then(|_| {
                helpers::configure_deterministic_confluence_knowledge_fixtures_for_repo(
                    world, &repo_name,
                )
            }),
        );
    })
}

pub(super) fn given_fixture_knowledge_add(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let fixture_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I add fixture knowledge",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::run_fixture_knowledge_add(world, &fixture_name)),
        );
    })
}

pub(super) fn given_knowledge_add_with_commit(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I add knowledge URL with commit association",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::run_knowledge_add_with_commit(world, &url)),
        );
    })
}

pub(super) fn given_knowledge_associate(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let source = ctx.matches[1].1.clone();
        let target = ctx.matches[2].1.clone();
        let repo_name = ctx.matches[3].1.clone();
        run_step(
            "I associate knowledge to knowledge",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::run_knowledge_associate(world, &source, &target)),
        );
    })
}

pub(super) fn given_knowledge_refresh(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let input = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I refresh knowledge",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::run_knowledge_refresh(world, &input)),
        );
    })
}

pub(super) fn given_fixture_knowledge_refresh(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let fixture_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I refresh fixture knowledge",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::run_fixture_knowledge_refresh(world, &fixture_name)),
        );
    })
}

pub(super) fn given_enqueue_devql_sync_task_with_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I enqueue DevQL sync task with status",
            helpers::enqueue_devql_sync_task_with_status_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_enqueue_devql_sync_task_without_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I enqueue DevQL sync task without status",
            helpers::enqueue_devql_sync_task_without_status_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_enqueue_devql_ingest_task_without_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I enqueue DevQL ingest task without status",
            helpers::enqueue_devql_ingest_task_without_status_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_enqueue_devql_sync_task_with_paths_and_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let paths = ctx.matches[1]
            .1
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I enqueue DevQL sync task with paths and status",
            helpers::enqueue_devql_sync_task_with_paths_and_status_for_repo(
                world, &repo_name, &paths,
            ),
        );
    })
}

pub(super) fn given_enqueue_devql_full_sync_task_with_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I enqueue DevQL full sync task with status",
            helpers::enqueue_devql_full_sync_task_with_status_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_create_simple_rust_project(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a simple Rust project",
            helpers::create_simple_rust_project(world, &repo_name),
        );
    })
}

pub(super) fn given_enqueue_devql_sync_validate_task_with_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I enqueue DevQL sync validate task with status",
            helpers::enqueue_devql_sync_validate_task_with_status_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_enqueue_devql_sync_repair_task_with_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I enqueue DevQL sync repair task with status",
            helpers::enqueue_devql_sync_repair_task_with_status_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_attempt_to_enqueue_devql_sync_task(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I attempt to enqueue DevQL sync task",
            helpers::attempt_to_enqueue_devql_sync_task_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_attempt_to_enqueue_devql_sync_task_require_daemon(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I attempt to enqueue DevQL sync task with require-daemon",
            helpers::attempt_to_enqueue_devql_sync_task_require_daemon_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_run_devql_tasks_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run DevQL tasks status",
            helpers::run_devql_tasks_status_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_wait_for_devql_task_queue_idle(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I wait for the DevQL task queue to become idle",
            helpers::wait_for_devql_task_queue_idle_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_run_devql_tasks_list(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run DevQL tasks list",
            helpers::run_devql_tasks_list_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_run_devql_tasks_list_for_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let status = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I run DevQL tasks list for status",
            helpers::run_devql_tasks_list_for_status_for_repo(world, &repo_name, &status),
        );
    })
}

pub(super) fn given_watch_last_devql_task(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I watch the last DevQL task",
            helpers::watch_last_devql_task_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_pause_devql_tasks_with_reason(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let reason = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I pause the DevQL task queue",
            helpers::pause_devql_tasks_for_repo(world, &repo_name, Some(&reason)),
        );
    })
}

pub(super) fn given_resume_devql_tasks(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I resume the DevQL task queue",
            helpers::resume_devql_tasks_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_cancel_last_devql_task(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I cancel the last DevQL task",
            helpers::cancel_last_devql_task_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_enqueue_devql_ingest_task_with_backfill_and_status(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let backfill = ctx.matches[1]
            .1
            .parse::<usize>()
            .expect("backfill should parse as usize");
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I enqueue DevQL ingest task with backfill and status",
            helpers::enqueue_devql_ingest_task_with_backfill_and_status_for_repo(
                world, &repo_name, backfill,
            ),
        );
    })
}

pub(super) fn given_add_new_source_file(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I add a new source file",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::add_new_rust_source_file(world)),
        );
    })
}

pub(super) fn given_add_source_file_at_path(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I add a source file at path",
            helpers::add_source_file_at_path_for_repo(world, &repo_name, &path),
        );
    })
}

pub(super) fn given_modify_existing_source_file(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I modify an existing source file",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::modify_rust_main(world)),
        );
    })
}

pub(super) fn given_modify_source_file_at_path(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let path = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I modify a source file at path",
            helpers::modify_source_file_at_path_for_repo(world, &repo_name, &path),
        );
    })
}

pub(super) fn given_snapshot_current_file_state_content_ids(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let paths = ctx.matches[1]
            .1
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I snapshot current-state content ids",
            helpers::snapshot_current_file_state_content_ids_for_paths(world, &repo_name, &paths),
        );
    })
}

pub(super) fn given_delete_a_source_file(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I delete a source file",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::delete_rust_source_file(world)),
        );
    })
}

pub(super) fn given_delete_test_file(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I delete a test file",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::delete_test_file(world)),
        );
    })
}

pub(super) fn given_commit_without_hooks(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I commit changes without hooks",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::commit_without_hooks(world)),
        );
    })
}

pub(super) fn given_stage_without_committing(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I stage the changes without committing",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::stage_changes_without_committing(world)),
        );
    })
}

pub(super) fn given_stop_daemon(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I stop the daemon",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::stop_daemon_for_scenario(world)),
        );
    })
}

pub(super) fn given_simulate_git_pull(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I simulate a git pull with new changes",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::simulate_git_pull_with_changes(world)),
        );
    })
}

pub(super) fn given_create_branch_with_files(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I create a new branch with additional source files",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::create_branch_with_additional_files(world)),
        );
    })
}

pub(super) fn given_knowledge_add_expect_failure(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let url = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I attempt to add knowledge URL",
            helpers::ensure_bitloops_repo_name(&repo_name)
                .and_then(|_| helpers::run_knowledge_add_expect_failure(world, &url)),
        );
    })
}
