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

pub(super) fn given_default_clean_start(
    world: &mut QatWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "I run CleanStart",
            helpers::run_clean_start(world, "qat-manual"),
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
            helpers::run_init_bitloops_with_agent(world, &repo_name, &agent_name, false),
        );
    })
}

pub(super) fn given_init_bitloops_with_agent_force(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let agent_name = ctx.matches[1].1.clone();
        let repo_name = ctx.matches[2].1.clone();
        run_step(
            "I run bitloops init --agent --force",
            helpers::run_init_bitloops_with_agent(world, &repo_name, &agent_name, true),
        );
    })
}

pub(super) fn given_enable_cli(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run EnableCLI",
            helpers::run_enable_cli_for_repo(world, &repo_name),
        );
    })
}

pub(super) fn given_enable(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run bitloops enable",
            helpers::run_bitloops_enable_with_flags(world, &repo_name, &[]),
        );
    })
}

pub(super) fn given_enable_project(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run bitloops enable --project",
            helpers::run_bitloops_enable_with_flags(world, &repo_name, &["--project"]),
        );
    })
}

pub(super) fn given_disable(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run bitloops disable",
            helpers::run_bitloops_disable(world, &repo_name),
        );
    })
}

pub(super) fn given_simulate_codex_checkpoint(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I simulate a codex checkpoint",
            helpers::simulate_codex_checkpoint(world, &repo_name),
        );
    })
}

pub(super) fn given_simulate_claude_checkpoint(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I simulate a claude checkpoint",
            helpers::simulate_codex_checkpoint(world, &repo_name),
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

pub(super) fn given_devql_ingest(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run DevQL ingest",
            helpers::run_devql_ingest_for_repo(world, &repo_name),
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
            "I run TestLens ingest-tests",
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
            "I run TestLens ingest-coverage",
            helpers::run_testlens_ingest_coverage(world, &repo_name),
        );
    })
}

pub(super) fn given_testlens_ingest_tests_at_head(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run TestLens ingest-tests at HEAD",
            helpers::run_testlens_ingest_tests(world, &repo_name),
        );
    })
}

pub(super) fn given_testlens_ingest_coverage_at_head(
    world: &mut QatWorld,
    ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run TestLens ingest-coverage at HEAD",
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
            "I run TestLens ingest-results with a failing test",
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
