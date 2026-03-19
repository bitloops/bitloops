use super::helpers;
use super::world::FtfWorld;
use cucumber::{codegen::LocalBoxFuture, step::Collection};
use regex::Regex;

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|err| panic!("invalid step regex `{pattern}`: {err}"))
}

fn step_fn(
    f: for<'a> fn(&'a mut FtfWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()>,
) -> for<'a> fn(&'a mut FtfWorld, cucumber::step::Context) -> LocalBoxFuture<'a, ()> {
    f
}

fn run_step(step_name: &str, result: anyhow::Result<()>) {
    if let Err(err) = result {
        panic!("{step_name} failed: {err:#}");
    }
}

fn given_clean_start(world: &mut FtfWorld, ctx: cucumber::step::Context) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let flow_name = ctx.matches[1].1.clone();
        run_step(
            "I run CleanStart for flow",
            helpers::run_clean_start(world, &flow_name),
        );
    })
}

fn given_default_clean_start(
    world: &mut FtfWorld,
    _ctx: cucumber::step::Context,
) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        run_step(
            "I run CleanStart",
            helpers::run_clean_start(world, "ftf-manual"),
        );
    })
}

fn given_init_commit(world: &mut FtfWorld, ctx: cucumber::step::Context) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run InitCommit",
            helpers::run_init_commit_for_repo(world, &repo_name),
        );
    })
}

fn given_init_commit_yesterday(
    world: &mut FtfWorld,
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

fn given_create_vite_app(
    world: &mut FtfWorld,
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

fn given_init_bitloops(
    world: &mut FtfWorld,
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

fn given_enable_cli(world: &mut FtfWorld, ctx: cucumber::step::Context) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let repo_name = ctx.matches[1].1.clone();
        run_step(
            "I run EnableCLI",
            helpers::run_enable_cli_for_repo(world, &repo_name),
        );
    })
}

fn given_first_claude_change(
    world: &mut FtfWorld,
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

fn given_second_claude_change(
    world: &mut FtfWorld,
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

fn given_commit_yesterday(
    world: &mut FtfWorld,
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

fn given_commit_today(
    world: &mut FtfWorld,
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

fn then_bitloops_stores_exist(
    world: &mut FtfWorld,
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

fn then_commit_timeline_is_correct(
    world: &mut FtfWorld,
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

fn then_claude_session_exists(
    world: &mut FtfWorld,
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

fn then_checkpoint_mapping_exists(
    world: &mut FtfWorld,
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

fn then_checkpoint_mapping_count_at_least(
    world: &mut FtfWorld,
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

pub fn collection() -> Collection<FtfWorld> {
    Collection::new()
        .given(
            None,
            regex(r#"^I run CleanStart for flow "([^"]+)"$"#),
            step_fn(given_clean_start),
        )
        .given(
            None,
            regex(r"^I run CleanStart$"),
            step_fn(given_default_clean_start),
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
            regex(r"^I make a first change using Claude Code to (\S+)$"),
            step_fn(given_first_claude_change),
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
        .then(
            None,
            regex(r"^bitloops stores exist in (\S+)$"),
            step_fn(then_bitloops_stores_exist),
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
}
