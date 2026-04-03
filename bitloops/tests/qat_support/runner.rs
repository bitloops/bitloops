use super::helpers::{sanitize_name, stop_daemon_for_scenario};
use super::steps;
use super::world::{QatRunConfig, QatWorld};
use anyhow::{Context, Result, bail};
use cucumber::{World as _, writer::Stats as _};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

pub enum Suite {
    Smoke,
    Devql,
    DevqlSync,
    ClaudeCode,
    Onboarding,
    Quickstart,
}

pub async fn run_suite(binary_path: PathBuf, suite: Suite) -> Result<()> {
    let max_concurrent = resolve_max_concurrent_scenarios();
    let runs_root = resolve_runs_root()?;
    let suite_root = create_suite_root(&runs_root)?;
    let feature_path = suite_feature_path(&suite);

    fs::write(
        runs_root.join(".last-run"),
        format!("{}\n", suite_root.display()),
    )
    .with_context(|| format!("writing latest qat pointer in {}", runs_root.display()))?;

    println!(
        "Running Bitloops QAT features from {}",
        feature_path.display()
    );
    println!("Artifacts will be written to {}", suite_root.display());

    let config = Arc::new(QatRunConfig {
        binary_path,
        suite_root: suite_root.clone(),
    });

    let before_config = Arc::clone(&config);
    let result = QatWorld::cucumber()
        .steps(steps::collection())
        .max_concurrent_scenarios(max_concurrent)
        .before(move |_, _, scenario, world| {
            let config = Arc::clone(&before_config);
            Box::pin(async move {
                let slug = sanitize_name(&scenario.name);
                world.prepare(config, &scenario.name, slug);
            })
        })
        .after(|_, _, scenario, _, world| {
            Box::pin(async move {
                if let Some(world) = world
                    && let Err(err) = stop_daemon_for_scenario(world)
                {
                    eprintln!(
                        "warning: daemon teardown failed for scenario `{}`: {err:#}",
                        scenario.name
                    );
                }
            })
        })
        .fail_on_skipped()
        .with_default_cli()
        .run(feature_path)
        .await;

    if result.execution_has_failed() || result.parsing_errors() != 0 {
        bail!(
            "bitloops qat reported failures (parsing_errors={}, skipped_steps={})\nartifacts: {}",
            result.parsing_errors(),
            result.skipped_steps(),
            suite_root.display()
        );
    }

    println!("Bitloops QAT completed successfully.");
    println!("Artifacts: {}", suite_root.display());
    Ok(())
}

fn resolve_runs_root() -> Result<PathBuf> {
    Ok(env::current_dir()
        .context("resolving current directory for qat runs dir")?
        .join("target")
        .join("qat-runs"))
}

fn create_suite_root(runs_root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(runs_root)
        .with_context(|| format!("creating qat runs root {}", runs_root.display()))?;
    let timestamp = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("formatting qat suite timestamp")?
        .replace(':', "-");
    let suite_dir = runs_root.join(format!(
        "{}-{}",
        timestamp,
        &Uuid::new_v4().simple().to_string()[..8]
    ));
    fs::create_dir_all(&suite_dir)
        .with_context(|| format!("creating qat suite dir {}", suite_dir.display()))?;
    Ok(suite_dir)
}

fn feature_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("qat")
        .join("features")
}

fn suite_feature_path(suite: &Suite) -> PathBuf {
    let root = feature_root();
    match suite {
        Suite::Smoke => root.join("smoke"),
        Suite::Devql => root.join("devql"),
        Suite::DevqlSync => root.join("devql-sync"),
        Suite::ClaudeCode => root.join("claude-code"),
        Suite::Onboarding => root.join("onboarding"),
        Suite::Quickstart => root.join("quickstart"),
    }
}

fn resolve_max_concurrent_scenarios() -> usize {
    env::var("BITLOOPS_QAT_MAX_CONCURRENT_SCENARIOS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1)
}
