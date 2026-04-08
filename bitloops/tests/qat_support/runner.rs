use super::helpers::{sanitize_name, stop_daemon_for_scenario};
use super::steps;
use super::world::{QatRunConfig, QatWorld};
use anyhow::{Context, Result, bail};
use cucumber::{World as _, writer::Stats as _};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

pub enum Suite {
    Smoke,
    Devql,
    DevqlIngest,
    DevqlSync,
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

    let suite_binary_snapshot = prepare_suite_binary(&binary_path, &suite_root)?;
    let execution_binary = resolve_execution_binary(&suite, &binary_path, &suite_binary_snapshot);

    println!(
        "Running Bitloops QAT features from {}",
        feature_path.display()
    );
    println!("Artifacts will be written to {}", suite_root.display());

    let config = Arc::new(QatRunConfig {
        binary_path: execution_binary,
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

fn prepare_suite_binary(binary_path: &Path, suite_root: &Path) -> Result<PathBuf> {
    let suite_binary_snapshot = suite_root.join(
        binary_path
            .file_name()
            .context("binary path has no file name")?,
    );
    fs::copy(binary_path, &suite_binary_snapshot).with_context(|| {
        format!(
            "copying binary {} -> {}",
            binary_path.display(),
            suite_binary_snapshot.display()
        )
    })?;
    stage_duckdb_runtime_for_snapshot(binary_path, &suite_binary_snapshot)?;
    Ok(suite_binary_snapshot)
}

fn resolve_execution_binary(
    _suite: &Suite,
    _original_binary: &Path,
    suite_snapshot: &Path,
) -> PathBuf {
    // Run all suites from the per-suite snapshot so suites remain isolated from each other.
    suite_snapshot.to_path_buf()
}

fn stage_duckdb_runtime_for_snapshot(source_binary: &Path, snapshot_binary: &Path) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }

    if !binary_links_duckdb_via_rpath(snapshot_binary) {
        return Ok(());
    }

    ensure_executable_path_rpath(snapshot_binary)?;

    let snapshot_dir = snapshot_binary
        .parent()
        .context("snapshot binary path has no parent directory")?;
    let staged_lib = snapshot_dir.join("libduckdb.dylib");
    if staged_lib.exists() {
        return Ok(());
    }

    let source_lib = resolve_duckdb_dylib_for_snapshot(source_binary)?;
    fs::copy(&source_lib, &staged_lib).with_context(|| {
        format!(
            "copying DuckDB runtime {} -> {}",
            source_lib.display(),
            staged_lib.display()
        )
    })?;
    Ok(())
}

fn binary_links_duckdb_via_rpath(binary_path: &Path) -> bool {
    let output = Command::new("otool").arg("-L").arg(binary_path).output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.contains("@rpath/libduckdb.dylib")
}

fn ensure_executable_path_rpath(binary_path: &Path) -> Result<()> {
    if binary_has_rpath(binary_path, "@executable_path") {
        return Ok(());
    }

    let status = Command::new("install_name_tool")
        .arg("-add_rpath")
        .arg("@executable_path")
        .arg(binary_path)
        .status()
        .with_context(|| format!("running install_name_tool for {}", binary_path.display()))?;
    if !status.success() {
        bail!(
            "install_name_tool -add_rpath @executable_path failed for {}",
            binary_path.display()
        );
    }
    Ok(())
}

fn binary_has_rpath(binary_path: &Path, rpath: &str) -> bool {
    let output = Command::new("otool").arg("-l").arg(binary_path).output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.contains(&format!("path {rpath} "))
}

fn resolve_duckdb_dylib_for_snapshot(source_binary: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(source_bin_dir) = source_binary.parent() {
        candidates.push(source_bin_dir.join("libduckdb.dylib"));
    }

    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest_root.clone());

    for root in [&manifest_root, &workspace_root] {
        candidates.push(root.join("target/release/deps/libduckdb.dylib"));
        candidates.push(root.join("target/debug/deps/libduckdb.dylib"));
        extend_duckdb_download_candidates(&mut candidates, root);
    }

    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not locate libduckdb.dylib for snapshot {}",
                snapshot_target_hint(source_binary)
            )
        })
}

fn extend_duckdb_download_candidates(candidates: &mut Vec<PathBuf>, root: &Path) {
    if let Ok(entries) = fs::read_dir(root.join("target/duckdb-download")) {
        for arch_entry in entries.flatten() {
            if let Ok(versions) = fs::read_dir(arch_entry.path()) {
                for version_entry in versions.flatten() {
                    candidates.push(version_entry.path().join("libduckdb.dylib"));
                }
            }
        }
    }
}

fn snapshot_target_hint(source_binary: &Path) -> String {
    source_binary
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| source_binary.display().to_string())
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
        Suite::DevqlIngest => root.join("devql").join("ingest_workspace.feature"),
        Suite::DevqlSync => root.join("devql-sync"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_suite_binary_copies_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary_path = temp.path().join("bitloops");
        fs::write(&binary_path, b"qat-test-binary").expect("write source binary");
        let suite_root = temp.path().join("suite");
        fs::create_dir_all(&suite_root).expect("create suite root");

        let snapshot_binary =
            prepare_suite_binary(&binary_path, &suite_root).expect("prepare suite binary");
        assert_eq!(snapshot_binary, suite_root.join("bitloops"));
        assert!(snapshot_binary.exists());
        assert_eq!(
            fs::read(&snapshot_binary).expect("read snapshot"),
            b"qat-test-binary"
        );
    }

    #[test]
    fn resolve_execution_binary_uses_snapshot_for_onboarding() {
        let original = PathBuf::from("/tmp/original-bitloops");
        let snapshot = PathBuf::from("/tmp/suite/bitloops");
        assert_eq!(
            resolve_execution_binary(&Suite::Onboarding, &original, &snapshot),
            snapshot
        );
    }

    #[test]
    fn resolve_execution_binary_uses_snapshot_for_devql_sync() {
        let original = PathBuf::from("/tmp/original-bitloops");
        let snapshot = PathBuf::from("/tmp/suite/bitloops");
        assert_eq!(
            resolve_execution_binary(&Suite::DevqlSync, &original, &snapshot),
            snapshot
        );
    }

    #[test]
    fn suite_feature_path_points_to_dedicated_devql_ingest_feature() {
        let path = suite_feature_path(&Suite::DevqlIngest);
        assert_eq!(
            path,
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("qat")
                .join("features")
                .join("devql")
                .join("ingest_workspace.feature")
        );
    }
}
