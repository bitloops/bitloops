use super::helpers::sanitize_name;
use super::steps;
use super::world::{QatRunConfig, QatWorld};
use anyhow::{Context, Result, bail};
use clap::Args;
use cucumber::{World as _, writer::Stats as _};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

#[derive(Args, Debug, Clone, Default)]
pub struct QatArgs {
    /// Run the lightweight foundation smoke suite instead of the default Claude Code suite.
    #[arg(long, default_value_t = false)]
    pub smoke: bool,

    /// Run the DevQL capability journey suite.
    #[arg(long, default_value_t = false)]
    pub devql: bool,

    /// Run a specific feature file or feature directory.
    #[arg(long)]
    pub feature: Option<PathBuf>,

    /// Directory where run artifacts are written.
    #[arg(long)]
    pub runs_dir: Option<PathBuf>,

    /// Maximum number of scenarios to execute concurrently.
    ///
    /// Defaults to `1` to keep QAT runs deterministic and avoid output
    /// deadlocks under heavy feature suites.
    #[arg(long)]
    pub concurrency: Option<usize>,
}

pub async fn run(args: QatArgs) -> Result<()> {
    let max_concurrent = resolve_max_concurrent_scenarios(args.concurrency);
    let binary_path = env::current_exe().context("resolving current bitloops executable")?; // it gets bitloops binary path from target/debug/bitloops
    let runs_root = resolve_runs_root(args.runs_dir.clone())?;
    let suite_root = create_suite_root(&runs_root)?;
    let feature_path = resolve_feature_path(&args)?;

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

fn resolve_runs_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    match explicit {
        Some(path) if path.is_absolute() => Ok(path),
        Some(path) => Ok(env::current_dir()
            .context("resolving current directory for qat runs dir")?
            .join(path)),
        None => Ok(env::current_dir()
            .context("resolving current directory for qat runs dir")?
            .join("target")
            .join("qat-runs")),
    }
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

fn resolve_feature_path(args: &QatArgs) -> Result<PathBuf> {
    if let Some(path) = args.feature.as_ref() {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            env::current_dir()
                .context("resolving current directory for qat feature path")?
                .join(path)
        };
        return Ok(resolved);
    }

    Ok(default_feature_path(&feature_root(), args))
}

fn feature_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("qat")
        .join("features")
}

fn default_feature_path(feature_root: &Path, args: &QatArgs) -> PathBuf {
    if args.smoke {
        feature_root.join("smoke")
    } else if args.devql {
        feature_root.join("devql")
    } else {
        feature_root.join("claude-code")
    }
}

fn resolve_max_concurrent_scenarios(explicit: Option<usize>) -> usize {
    explicit.unwrap_or_else(|| {
        parse_positive_usize(
            env::var("BITLOOPS_QAT_MAX_CONCURRENT_SCENARIOS")
                .ok()
                .as_deref(),
            1,
        )
    })
}

fn parse_positive_usize(raw: Option<&str>, default_value: usize) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_feature_path_defaults_to_claude_suite() {
        let root = PathBuf::from("/tmp/qat/features");
        let mut args = QatArgs::default();
        assert_eq!(default_feature_path(&root, &args), root.join("claude-code"));

        args.smoke = true;
        assert_eq!(default_feature_path(&root, &args), root.join("smoke"));

        args.smoke = false;
        args.devql = true;
        assert_eq!(default_feature_path(&root, &args), root.join("devql"));
    }

    #[test]
    fn parse_positive_usize_defaults_for_invalid_values() {
        assert_eq!(parse_positive_usize(None, 3), 3);
        assert_eq!(parse_positive_usize(Some(""), 3), 3);
        assert_eq!(parse_positive_usize(Some("0"), 3), 3);
        assert_eq!(parse_positive_usize(Some("-1"), 3), 3);
        assert_eq!(parse_positive_usize(Some("abc"), 3), 3);
    }

    #[test]
    fn parse_positive_usize_accepts_positive_values() {
        assert_eq!(parse_positive_usize(Some("1"), 3), 1);
        assert_eq!(parse_positive_usize(Some(" 8 "), 3), 8);
    }
}
