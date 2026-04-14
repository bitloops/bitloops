use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

const BITLOOPS_MANIFEST: &str = "bitloops/Cargo.toml";
const SLOW_TEST_TARGETS: &[&str] = &[
    "agent_cli_smoke",
    "claude_git_hooks_integration",
    "checkpoint_rewind_smoke",
    "copilot_integration",
    "cursor_e2e_scenarios",
    "dashboard_bundle_lifecycle_e2e",
    "e2e_scenario_groups",
    "graphql",
    "performance",
    "testlens_gherkin",
    "testlens_sqlite_acceptance",
];
const SLOW_LIB_TESTS: &[&str] = &["host::devql::cucumber_bdd::devql_bdd_features_pass"];
const MERGE_SMOKE_TARGETS: &[&str] = &[
    "agent_cli_smoke",
    "checkpoint_rewind_smoke",
    "dashboard_bundle_lifecycle_e2e",
    "graphql_smoke",
];
const DEFAULT_LCOV_PATH: &str = "bitloops/target/llvm-cov.info";

fn main() {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        print_usage();
        std::process::exit(2);
    };

    let result = match cmd.as_str() {
        "file-size" => {
            let root = args.next().unwrap_or_else(|| "bitloops".to_string());
            run_file_size_check(Path::new(&root))
        }
        "dev-loop" => run_dev_loop(),
        "install" => run_dev_install(),
        "test" => {
            let lane = args.next().unwrap_or_else(|| "fast".to_string());
            run_test_lane(&lane)
        }
        "coverage" => {
            let subcommand = args.next().unwrap_or_else(|| "run-lcov".to_string());
            run_coverage_command(&subcommand, args.collect())
        }
        other => Err(format!("unknown xtask command: {other}")),
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!("usage: cargo run -p xtask -- <command>");
    eprintln!("commands:");
    eprintln!("  file-size [root]");
    eprintln!("  dev-loop");
    eprintln!("  install");
    eprintln!("  test <lib|core|cli|fast|smoke|merge|slow|full>");
    eprintln!("  coverage run-lcov [--lcov <path>]");
    eprintln!("  coverage run-all [--lcov <path>] [--html-dir <path>]");
    eprintln!("  coverage metrics [--lcov <path>]");
    eprintln!("  coverage compare --lines <pct> --functions <pct> [--epsilon 0.5] [--lcov <path>]");
}

fn run_dev_loop() -> Result<(), String> {
    let workspace_root = workspace_root()?;

    run_command(
        &workspace_root,
        "cargo fmt --all --manifest-path bitloops/Cargo.toml",
        &["fmt", "--all", "--manifest-path", BITLOOPS_MANIFEST],
    )?;

    run_command(
        &workspace_root,
        "cargo clippy --manifest-path bitloops/Cargo.toml --all-targets --no-default-features -- -D warnings",
        &[
            "clippy",
            "--manifest-path",
            BITLOOPS_MANIFEST,
            "--all-targets",
            "--no-default-features",
            "--",
            "-D",
            "warnings",
        ],
    )?;

    run_test_lane("fast")?;
    run_file_size_check(&workspace_root.join("bitloops"))
}

fn run_dev_install() -> Result<(), String> {
    let workspace_root = workspace_root()?;

    run_command(
        &workspace_root,
        "cargo install --path bitloops --force",
        &["install", "--path", "bitloops", "--force"],
    )?;

    let binary_path = installed_binary_path("bitloops")?;
    stage_duckdb_runtime_for_installed_binary(&workspace_root, &binary_path)?;

    if should_sign() {
        let duckdb_dylib = binary_path
            .parent()
            .ok_or_else(|| format!("invalid binary path: {}", binary_path.display()))?
            .join("libduckdb.dylib");
        if duckdb_dylib.exists() {
            codesign_binary(&duckdb_dylib)?;
        }
        codesign_binary(&binary_path)?;
    }

    Ok(())
}

fn run_test_lane(lane: &str) -> Result<(), String> {
    ensure_cargo_subcommand_available("nextest", nextest_install_hint())?;
    let workspace_root = workspace_root()?;
    let lane_commands = test_lane_command_groups(lane)?;
    let test_threads = test_threads_for_lane(lane)?;
    let profile_args = nextest_profile_args();

    if should_sign() {
        let mut compiled_binaries = BTreeSet::new();
        for lane_args in &lane_commands {
            let list_args = nextest_list_args(lane_args, &profile_args)?;
            for binary in collect_test_binaries(&workspace_root, &list_args)? {
                compiled_binaries.insert(binary);
            }
        }
        for binary in compiled_binaries {
            codesign_binary(&binary)?;
        }
    }

    for lane_args in lane_commands {
        let mut run_args = lane_args;
        run_args.extend(profile_args.clone());
        if let Some(test_threads) = test_threads {
            run_args.push(format!("--test-threads={test_threads}"));
        }
        let command = prepend_cargo(&run_args);
        run_command_owned(
            &workspace_root,
            &format!("cargo {}", run_args.join(" ")),
            &command,
        )?;
    }

    Ok(())
}

fn ensure_cargo_subcommand_available(subcommand: &str, install_hint: &str) -> Result<(), String> {
    let status = Command::new("cargo")
        .arg(subcommand)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(format!(
            "required Cargo subcommand `cargo {subcommand}` is not available. Install it first, for example: {install_hint}"
        )),
        Err(err) => Err(format!(
            "failed to check whether `cargo {subcommand}` is available: {err}"
        )),
    }
}

fn nextest_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "`brew install cargo-nextest` or `cargo install cargo-nextest --locked`"
    } else {
        "`cargo install cargo-nextest --locked`"
    }
}

fn run_coverage_command(subcommand: &str, raw_args: Vec<String>) -> Result<(), String> {
    match subcommand {
        "run-lcov" => {
            let lcov_path = parse_lcov_path(&raw_args)?;
            run_coverage_lcov(&lcov_path)
        }
        "run-all" => {
            let (lcov_path, html_dir) = parse_coverage_all_paths(&raw_args)?;
            run_coverage_all(&lcov_path, &html_dir)
        }
        "metrics" => {
            let lcov_path = parse_lcov_path(&raw_args)?;
            let workspace_root = workspace_root()?;
            let metrics = read_lcov_metrics(&resolve_workspace_path(&workspace_root, &lcov_path))?;
            println!("lines_pct={:.2}", metrics.lines_pct);
            println!("functions_pct={:.2}", metrics.functions_pct);
            println!("lines_covered={}", metrics.lines_covered);
            println!("lines_total={}", metrics.lines_total);
            println!("functions_covered={}", metrics.functions_covered);
            println!("functions_total={}", metrics.functions_total);
            Ok(())
        }
        "compare" => {
            let options = parse_compare_options(&raw_args)?;
            let workspace_root = workspace_root()?;
            let metrics =
                read_lcov_metrics(&resolve_workspace_path(&workspace_root, &options.lcov_path))?;
            print_coverage_comparison(&metrics, options.lines_baseline, options.functions_baseline);

            if is_regression(metrics.lines_pct, options.lines_baseline, options.epsilon)
                || is_regression(
                    metrics.functions_pct,
                    options.functions_baseline,
                    options.epsilon,
                )
            {
                return Err(format!(
                    "coverage regression detected (epsilon {:.2}pp)",
                    options.epsilon
                ));
            }
            Ok(())
        }
        _ => Err(unknown_coverage_subcommand_error(subcommand)),
    }
}

fn resolve_workspace_path(workspace_root: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path);
    if path.is_absolute() {
        path
    } else {
        workspace_root.join(path)
    }
}

/// Cargo argv fragment for `llvm-cov` LCOV export (after `cargo`). Used for tests and `run_coverage_lcov`.
fn llvm_cov_lcov_cargo_args(output_path: &str) -> Vec<String> {
    vec![
        "llvm-cov".to_string(),
        "--manifest-path".to_string(),
        BITLOOPS_MANIFEST.to_string(),
        "--workspace".to_string(),
        "--all-targets".to_string(),
        "--features".to_string(),
        "slow-tests".to_string(),
        "--no-default-features".to_string(),
        "--lcov".to_string(),
        "--output-path".to_string(),
        output_path.to_string(),
    ]
}

fn llvm_cov_lcov_display_command(output_path: &str) -> String {
    format!(
        "cargo llvm-cov --manifest-path {} --workspace --all-targets --features slow-tests --no-default-features --lcov --output-path {output_path}",
        BITLOOPS_MANIFEST
    )
}

/// Cargo argv for `llvm-cov report --html` (after `cargo`).
fn llvm_cov_report_html_cargo_args(html_dir: &str) -> Vec<String> {
    vec![
        "llvm-cov".to_string(),
        "report".to_string(),
        "--manifest-path".to_string(),
        BITLOOPS_MANIFEST.to_string(),
        "--html".to_string(),
        "--output-dir".to_string(),
        html_dir.to_string(),
    ]
}

fn llvm_cov_report_html_display_command(html_dir: &str) -> String {
    format!(
        "cargo llvm-cov report --manifest-path {} --html --output-dir {html_dir}",
        BITLOOPS_MANIFEST
    )
}

fn collect_nextest_binary_paths(value: &Value, out: &mut BTreeSet<PathBuf>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if matches!(key.as_str(), "binary-path" | "binary_path")
                    && let Some(path) = child.as_str()
                {
                    out.insert(PathBuf::from(path));
                }
                collect_nextest_binary_paths(child, out);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_nextest_binary_paths(child, out);
            }
        }
        _ => {}
    }
}

fn parse_nextest_binary_paths(output: &str) -> Vec<PathBuf> {
    let mut binaries = BTreeSet::new();
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(json) = serde_json::from_str::<Value>(trimmed) {
        collect_nextest_binary_paths(&json, &mut binaries);
        return binaries.into_iter().collect();
    }

    for line in trimmed.lines() {
        if let Ok(json) = serde_json::from_str::<Value>(line) {
            collect_nextest_binary_paths(&json, &mut binaries);
        }
    }

    binaries.into_iter().collect()
}

fn otool_list_output_links_libduckdb(text: &str) -> bool {
    text.contains("@rpath/libduckdb.dylib")
}

fn otool_load_output_contains_rpath(text: &str, rpath: &str) -> bool {
    text.contains("cmd LC_RPATH") && text.contains(&format!("path {rpath} "))
}

fn unknown_coverage_subcommand_error(subcommand: &str) -> String {
    format!(
        "unknown coverage subcommand `{subcommand}` (expected: run-lcov|run-all|metrics|compare)"
    )
}

fn run_coverage_lcov(lcov_path: &str) -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let resolved_lcov_path = resolve_workspace_path(&workspace_root, lcov_path);
    if let Some(parent) = resolved_lcov_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let resolved_lcov_path = resolved_lcov_path.to_string_lossy().to_string();

    let display = llvm_cov_lcov_display_command(&resolved_lcov_path);
    let args = llvm_cov_lcov_cargo_args(&resolved_lcov_path);
    run_command_owned(&workspace_root, &display, &prepend_cargo(&args))
}

fn run_coverage_all(lcov_path: &str, html_dir: &str) -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let resolved_html_dir = resolve_workspace_path(&workspace_root, html_dir);
    fs::create_dir_all(&resolved_html_dir)
        .map_err(|err| format!("failed to create {}: {err}", resolved_html_dir.display()))?;

    run_coverage_lcov(lcov_path)?;

    let resolved_html_dir = resolved_html_dir.to_string_lossy().to_string();
    let display = llvm_cov_report_html_display_command(&resolved_html_dir);
    let args = llvm_cov_report_html_cargo_args(&resolved_html_dir);
    run_command_owned(&workspace_root, &display, &prepend_cargo(&args))
}

fn parse_lcov_path(args: &[String]) -> Result<String, String> {
    let mut lcov_path = DEFAULT_LCOV_PATH.to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lcov" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--lcov requires a value".to_string());
                };
                lcov_path = value.clone();
                i += 2;
            }
            other => return Err(format!("unknown coverage argument: {other}")),
        }
    }
    Ok(lcov_path)
}

fn parse_coverage_all_paths(args: &[String]) -> Result<(String, String), String> {
    let mut lcov_path = DEFAULT_LCOV_PATH.to_string();
    let mut html_dir = "bitloops/target/llvm-cov-html".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lcov" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--lcov requires a value".to_string());
                };
                lcov_path = value.clone();
                i += 2;
            }
            "--html-dir" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--html-dir requires a value".to_string());
                };
                html_dir = value.clone();
                i += 2;
            }
            other => return Err(format!("unknown coverage argument: {other}")),
        }
    }
    Ok((lcov_path, html_dir))
}

#[derive(Debug, Clone)]
struct CompareOptions {
    lines_baseline: f64,
    functions_baseline: f64,
    epsilon: f64,
    lcov_path: String,
}

fn parse_compare_options(args: &[String]) -> Result<CompareOptions, String> {
    let mut lines_baseline: Option<f64> = None;
    let mut functions_baseline: Option<f64> = None;
    let mut epsilon = 0.5;
    let mut lcov_path = DEFAULT_LCOV_PATH.to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lines" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--lines requires a value".to_string());
                };
                lines_baseline = Some(parse_f64_flag("--lines", value)?);
                i += 2;
            }
            "--functions" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--functions requires a value".to_string());
                };
                functions_baseline = Some(parse_f64_flag("--functions", value)?);
                i += 2;
            }
            "--epsilon" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--epsilon requires a value".to_string());
                };
                epsilon = parse_f64_flag("--epsilon", value)?;
                i += 2;
            }
            "--lcov" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--lcov requires a value".to_string());
                };
                lcov_path = value.clone();
                i += 2;
            }
            other => return Err(format!("unknown compare argument: {other}")),
        }
    }

    Ok(CompareOptions {
        lines_baseline: lines_baseline.ok_or_else(|| "--lines is required".to_string())?,
        functions_baseline: functions_baseline
            .ok_or_else(|| "--functions is required".to_string())?,
        epsilon,
        lcov_path,
    })
}

fn parse_f64_flag(flag: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("{flag} must be a decimal number"))
}

#[derive(Debug, Clone, Copy)]
struct CoverageMetrics {
    lines_covered: u64,
    lines_total: u64,
    functions_covered: u64,
    functions_total: u64,
    lines_pct: f64,
    functions_pct: f64,
}

fn read_lcov_metrics(path: &Path) -> Result<CoverageMetrics, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    parse_lcov_metrics(&content)
}

fn parse_lcov_metrics(content: &str) -> Result<CoverageMetrics, String> {
    let mut lines_total = 0_u64;
    let mut lines_covered = 0_u64;
    let mut functions_total = 0_u64;
    let mut functions_covered = 0_u64;

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("LF:") {
            lines_total += parse_u64_value("LF", value)?;
        } else if let Some(value) = line.strip_prefix("LH:") {
            lines_covered += parse_u64_value("LH", value)?;
        } else if let Some(value) = line.strip_prefix("FNF:") {
            functions_total += parse_u64_value("FNF", value)?;
        } else if let Some(value) = line.strip_prefix("FNH:") {
            functions_covered += parse_u64_value("FNH", value)?;
        }
    }

    Ok(CoverageMetrics {
        lines_covered,
        lines_total,
        functions_covered,
        functions_total,
        lines_pct: percentage(lines_covered, lines_total),
        functions_pct: percentage(functions_covered, functions_total),
    })
}

fn parse_u64_value(name: &str, raw: &str) -> Result<u64, String> {
    raw.trim()
        .parse::<u64>()
        .map_err(|_| format!("invalid LCOV number for {name}: `{raw}`"))
}

fn percentage(covered: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (covered as f64) * 100.0 / (total as f64)
    }
}

fn is_regression(current: f64, baseline: f64, epsilon: f64) -> bool {
    current < (baseline - epsilon)
}

fn print_coverage_comparison(
    metrics: &CoverageMetrics,
    lines_baseline: f64,
    functions_baseline: f64,
) {
    println!("Coverage comparison:");
    println!(
        "  Lines      baseline: {:7.2}%  current: {:7.2}%  delta: {:+.2}",
        lines_baseline,
        metrics.lines_pct,
        metrics.lines_pct - lines_baseline
    );
    println!(
        "  Functions  baseline: {:7.2}%  current: {:7.2}%  delta: {:+.2}",
        functions_baseline,
        metrics.functions_pct,
        metrics.functions_pct - functions_baseline
    );
}

fn base_test_lane_args() -> Vec<String> {
    vec![
        "nextest".to_string(),
        "run".to_string(),
        "--manifest-path".to_string(),
        BITLOOPS_MANIFEST.to_string(),
        "--no-default-features".to_string(),
    ]
}

fn slow_test_lane_args(targets: &[&str]) -> Vec<String> {
    let mut args = base_test_lane_args();
    args.push("--features".to_string());
    args.push("slow-tests".to_string());
    for target in targets {
        args.push("--test".to_string());
        args.push((*target).to_string());
    }
    args
}

fn slow_lib_test_lane_args(test_name: &str) -> Vec<String> {
    let mut args = base_test_lane_args();
    args.push("--features".to_string());
    args.push("slow-tests".to_string());
    args.push("--lib".to_string());
    args.push("--".to_string());
    args.push(test_name.to_string());
    args.push("--exact".to_string());
    args
}

fn test_lane_args(lane: &str) -> Result<Vec<String>, String> {
    let mut args = base_test_lane_args();

    match lane {
        "lib" | "core" => {
            args.push("--lib".to_string());
        }
        "cli" => {
            args.push("--bin".to_string());
            args.push("bitloops".to_string());
        }
        "fast" => {}
        "slow" => {
            return Ok(slow_test_lane_args(SLOW_TEST_TARGETS));
        }
        "smoke" => {
            return Ok(slow_test_lane_args(MERGE_SMOKE_TARGETS));
        }
        "full" => {
            args.push("--features".to_string());
            args.push("slow-tests".to_string());
        }
        _ => {
            return Err(format!(
                "unknown test lane `{lane}` (expected: lib|core|cli|fast|smoke|merge|slow|full)"
            ));
        }
    }

    Ok(args)
}

fn test_lane_command_groups(lane: &str) -> Result<Vec<Vec<String>>, String> {
    match lane {
        "merge" => Ok(vec![test_lane_args("fast")?, test_lane_args("smoke")?]),
        "slow" => {
            let mut groups = vec![test_lane_args("slow")?];
            groups.extend(
                SLOW_LIB_TESTS
                    .iter()
                    .map(|test_name| slow_lib_test_lane_args(test_name)),
            );
            Ok(groups)
        }
        _ => Ok(vec![test_lane_args(lane)?]),
    }
}

fn test_threads_for_lane(lane: &str) -> Result<Option<u64>, String> {
    if lane != "fast" {
        return Ok(None);
    }

    match env::var("BITLOOPS_TEST_THREADS") {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }

            let parsed = trimmed
                .parse::<u64>()
                .map_err(|_| "BITLOOPS_TEST_THREADS must be an integer".to_string())?;
            if parsed == 0 {
                return Err("BITLOOPS_TEST_THREADS must be greater than zero".to_string());
            }
            Ok(Some(parsed))
        }
        Err(_) => Ok(None),
    }
}

fn nextest_profile_args() -> Vec<String> {
    if env::var("GITHUB_ACTIONS")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        vec!["--profile".to_string(), "ci".to_string()]
    } else {
        Vec::new()
    }
}

fn nextest_list_args(run_args: &[String], profile_args: &[String]) -> Result<Vec<String>, String> {
    let mut args = run_args.to_vec();
    if args.get(0).map(String::as_str) != Some("nextest")
        || args.get(1).map(String::as_str) != Some("run")
    {
        return Err("expected nextest run arguments for test lane".to_string());
    }
    args[1] = "list".to_string();

    let separator_index = args.iter().position(|arg| arg == "--");
    let mut list_only_args = profile_args.to_vec();
    list_only_args.push("--list-type".to_string());
    list_only_args.push("binaries-only".to_string());
    list_only_args.push("--message-format".to_string());
    list_only_args.push("json".to_string());
    list_only_args.push("--cargo-message-format".to_string());
    list_only_args.push("json-render-diagnostics".to_string());

    if let Some(index) = separator_index {
        args.splice(index..index, list_only_args);
    } else {
        args.extend(list_only_args);
    }

    Ok(args)
}

fn collect_test_binaries(cwd: &Path, args: &[String]) -> Result<Vec<PathBuf>, String> {
    println!("==> cargo {}", args.join(" "));

    let output = Command::new("cargo")
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|err| format!("failed to start cargo nextest list step: {err}"))?;

    if !output.status.success() {
        return Err("command failed: cargo nextest list step".to_string());
    }

    Ok(parse_nextest_binary_paths(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn should_sign() -> bool {
    cfg!(target_os = "macos") && env::var("BITLOOPS_CODESIGN").map_or(true, |value| value != "0")
}

fn installed_binary_path(binary_name: &str) -> Result<PathBuf, String> {
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(default_cargo_home)
        .ok_or_else(|| "failed to resolve CARGO_HOME".to_string())?;
    Ok(cargo_home.join("bin").join(binary_name))
}

fn default_cargo_home() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cargo"))
}

fn codesign_binary(binary: &Path) -> Result<(), String> {
    let identity = env::var("BITLOOPS_CODESIGN_IDENTITY").unwrap_or_else(|_| "-".to_string());
    let verify = env::var("BITLOOPS_CODESIGN_VERIFY").map_or(true, |value| value != "0");

    run_command_owned(
        binary
            .parent()
            .ok_or_else(|| format!("invalid binary path: {}", binary.display()))?,
        &format!(
            "codesign --force --sign {identity} --timestamp=none {}",
            binary.display()
        ),
        &[
            "codesign".to_string(),
            "--force".to_string(),
            "--sign".to_string(),
            identity,
            "--timestamp=none".to_string(),
            binary.to_string_lossy().to_string(),
        ],
    )?;

    if verify {
        run_command_owned(
            binary
                .parent()
                .ok_or_else(|| format!("invalid binary path: {}", binary.display()))?,
            &format!("codesign --verify --verbose=2 {}", binary.display()),
            &[
                "codesign".to_string(),
                "--verify".to_string(),
                "--verbose=2".to_string(),
                binary.to_string_lossy().to_string(),
            ],
        )?;
    }

    Ok(())
}

fn stage_duckdb_runtime_for_installed_binary(
    workspace_root: &Path,
    binary_path: &Path,
) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    if !binary_links_duckdb_via_rpath(binary_path)? {
        return Ok(());
    }

    ensure_executable_path_rpath(binary_path)?;

    let bin_dir = binary_path
        .parent()
        .ok_or_else(|| format!("invalid binary path: {}", binary_path.display()))?;
    let staged_lib = bin_dir.join("libduckdb.dylib");
    if staged_lib.exists() {
        return Ok(());
    }

    let source_lib = resolve_workspace_duckdb_dylib(workspace_root)?;
    fs::copy(&source_lib, &staged_lib).map_err(|err| {
        format!(
            "failed to copy DuckDB runtime from {} to {}: {err}",
            source_lib.display(),
            staged_lib.display()
        )
    })?;
    Ok(())
}

fn binary_links_duckdb_via_rpath(binary_path: &Path) -> Result<bool, String> {
    let output = Command::new("otool")
        .arg("-L")
        .arg(binary_path)
        .output()
        .map_err(|err| format!("failed to run otool for {}: {err}", binary_path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "otool -L failed for {} with status {}",
            binary_path.display(),
            output.status
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|err| format!("otool output was not valid UTF-8: {err}"))?;
    Ok(otool_list_output_links_libduckdb(&text))
}

fn ensure_executable_path_rpath(binary_path: &Path) -> Result<(), String> {
    if binary_has_rpath(binary_path, "@executable_path")? {
        return Ok(());
    }

    let status = Command::new("install_name_tool")
        .arg("-add_rpath")
        .arg("@executable_path")
        .arg(binary_path)
        .status()
        .map_err(|err| {
            format!(
                "failed to run install_name_tool for {}: {err}",
                binary_path.display()
            )
        })?;
    if !status.success() {
        return Err(format!(
            "install_name_tool -add_rpath @executable_path failed for {}",
            binary_path.display()
        ));
    }
    Ok(())
}

fn binary_has_rpath(binary_path: &Path, rpath: &str) -> Result<bool, String> {
    let output = Command::new("otool")
        .arg("-l")
        .arg(binary_path)
        .output()
        .map_err(|err| format!("failed to run otool for {}: {err}", binary_path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "otool -l failed for {} with status {}",
            binary_path.display(),
            output.status
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|err| format!("otool output was not valid UTF-8: {err}"))?;
    Ok(otool_load_output_contains_rpath(&text, rpath))
}

fn resolve_workspace_duckdb_dylib(workspace_root: &Path) -> Result<PathBuf, String> {
    let mut candidates = vec![
        workspace_root.join("target/release/deps/libduckdb.dylib"),
        workspace_root.join("target/debug/deps/libduckdb.dylib"),
    ];
    if let Ok(entries) = fs::read_dir(workspace_root.join("target/duckdb-download")) {
        for arch_entry in entries.flatten() {
            if let Ok(versions) = fs::read_dir(arch_entry.path()) {
                for version_entry in versions.flatten() {
                    candidates.push(version_entry.path().join("libduckdb.dylib"));
                }
            }
        }
    }

    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| {
            "could not locate libduckdb.dylib under workspace target directories".to_string()
        })
}

fn workspace_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "failed to resolve workspace root".to_string())
}

fn prepend_cargo(args: &[String]) -> Vec<String> {
    let mut v = vec!["cargo".to_string()];
    v.extend_from_slice(args);
    v
}

fn run_command(cwd: &Path, display: &str, args: &[&str]) -> Result<(), String> {
    let mut owned = vec!["cargo".to_string()];
    owned.extend(args.iter().map(|s| (*s).to_string()));
    run_command_owned(cwd, display, &owned)
}

fn run_command_owned(cwd: &Path, display: &str, args: &[String]) -> Result<(), String> {
    println!("==> {display}");
    let status = Command::new(&args[0])
        .current_dir(cwd)
        .args(&args[1..])
        .status()
        .map_err(|err| format!("failed to start `{display}`: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed: {display}"))
    }
}

fn run_file_size_check(root: &Path) -> Result<(), String> {
    let warn_lines = env_u64("RUST_FILE_WARN_LINES", 500)?;
    let max_lines = env_u64("RUST_FILE_MAX_LINES", 1000)?;
    let include_tests = env::var("INCLUDE_TESTS").ok().as_deref() == Some("1");

    let git_toplevel = git_toplevel(root);
    let mut rust_files = Vec::new();
    collect_rs_files(root, &mut rust_files)?;
    rust_files.sort();

    if rust_files.is_empty() {
        println!("No Rust files found under {}", root.display());
        return Ok(());
    }

    let mut warnings: Vec<(u64, String)> = Vec::new();
    let mut failures: Vec<(u64, String)> = Vec::new();
    let mut size_index: Vec<(u64, String)> = Vec::new();

    for file in rust_files {
        if !include_tests && is_test_file(&file) {
            continue;
        }
        if is_gitignored(&file, git_toplevel.as_deref()) {
            continue;
        }

        let lines = count_lines(&file)?;
        size_index.push((lines, file.display().to_string()));

        if lines > max_lines {
            failures.push((lines, format!("{} (max {max_lines})", file.display())));
        } else if lines > warn_lines {
            warnings.push((lines, file.display().to_string()));
        }
    }

    println!("Rust file-size check");
    println!("- root: {}", root.display());
    println!("- warn: >{warn_lines} lines");
    println!("- max:  >{max_lines} lines (non-test)");
    println!();

    if !warnings.is_empty() {
        warnings.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        println!("Warnings:");
        for (lines, file) in &warnings {
            println!("  {lines} {file}");
        }
        println!();
    }

    if !size_index.is_empty() {
        size_index.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        println!("Top non-test Rust files by line count:");
        for (lines, file) in size_index.iter().take(15) {
            println!("  {lines} {file}");
        }
        println!();
    }

    if !failures.is_empty() {
        failures.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        println!("Failures:");
        for (lines, file) in &failures {
            println!("  {lines} {file}");
        }
        return Err("Rust file-size check failed.".to_string());
    }

    println!("OK: no non-test Rust file exceeded configured limits.");
    Ok(())
}

fn env_u64(key: &str, default: u64) -> Result<u64, String> {
    match env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| format!("{key} must be an integer")),
        Err(_) => Ok(default),
    }
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("failed to read directory entry: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn count_lines(path: &Path) -> Result<u64, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    Ok(content.lines().count() as u64)
}

fn is_test_file(path: &Path) -> bool {
    let in_tests_dir = path
        .components()
        .any(|component| component.as_os_str() == "tests");
    let file_name = path.file_name().and_then(|name| name.to_str());

    in_tests_dir
        || matches!(
            file_name,
            Some(name)
                if name.ends_with("_test.rs")
                    || name.ends_with("tests.rs")
                    || name.ends_with("_tests.rs")
        )
}

fn git_toplevel(root: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn is_gitignored(file: &Path, git_toplevel: Option<&Path>) -> bool {
    let Some(root) = git_toplevel else {
        return false;
    };
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("check-ignore")
        .arg("-q")
        .arg("--")
        .arg(file)
        .status();
    matches!(status, Ok(s) if s.success())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::env;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use tempfile::TempDir;

    use super::{
        BITLOOPS_MANIFEST, DEFAULT_LCOV_PATH, MERGE_SMOKE_TARGETS, SLOW_TEST_TARGETS,
        collect_nextest_binary_paths, collect_rs_files, count_lines, env_u64, is_regression,
        is_test_file, llvm_cov_lcov_cargo_args, llvm_cov_lcov_display_command,
        llvm_cov_report_html_cargo_args, llvm_cov_report_html_display_command,
        otool_list_output_links_libduckdb, otool_load_output_contains_rpath, parse_compare_options,
        parse_coverage_all_paths, parse_f64_flag, parse_lcov_metrics, parse_lcov_path,
        parse_nextest_binary_paths, parse_u64_value, percentage, prepend_cargo, read_lcov_metrics,
        resolve_workspace_path, run_file_size_check, test_lane_args, test_lane_command_groups,
        test_threads_for_lane, unknown_coverage_subcommand_error, workspace_root,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parses_lcov_metrics() {
        let lcov = "TN:\nSF:a.rs\nFNF:4\nFNH:3\nLF:10\nLH:8\nend_of_record\nSF:b.rs\nFNF:1\nFNH:1\nLF:2\nLH:2\nend_of_record\n";
        let metrics = parse_lcov_metrics(lcov).expect("parse lcov");

        assert_eq!(metrics.lines_covered, 10);
        assert_eq!(metrics.lines_total, 12);
        assert_eq!(metrics.functions_covered, 4);
        assert_eq!(metrics.functions_total, 5);
        assert!((metrics.lines_pct - 83.33).abs() < 0.01);
        assert!((metrics.functions_pct - 80.0).abs() < 0.001);
    }

    #[test]
    fn compare_honours_epsilon_boundary() {
        assert!(!is_regression(79.5, 80.0, 0.5));
        assert!(is_regression(79.49, 80.0, 0.5));
        assert!(!is_regression(74.5, 75.0, 0.5));
        assert!(is_regression(74.49, 75.0, 0.5));
    }

    #[test]
    fn percentage_zero_total() {
        assert_eq!(percentage(0, 0), 0.0);
        assert_eq!(percentage(3, 10), 30.0);
    }

    #[test]
    fn parse_u64_value_trims_and_rejects_invalid() {
        assert_eq!(parse_u64_value("LF", " 42 ").unwrap(), 42);
        assert!(
            parse_u64_value("LF", "x")
                .unwrap_err()
                .contains("invalid LCOV")
        );
    }

    #[test]
    fn parse_f64_flag_accepts_decimals() {
        assert_eq!(parse_f64_flag("--lines", "80.5").unwrap(), 80.5);
        assert!(
            parse_f64_flag("--lines", "nope")
                .unwrap_err()
                .contains("must be a decimal")
        );
    }

    #[test]
    fn parse_lcov_path_default_and_override() {
        assert_eq!(parse_lcov_path(&[]).unwrap(), DEFAULT_LCOV_PATH.to_string());
        assert_eq!(
            parse_lcov_path(&["--lcov".into(), "out.lcov".into()]).unwrap(),
            "out.lcov"
        );
        assert_eq!(
            parse_lcov_path(&["--lcov".into(), "a".into(), "--lcov".into(), "b".into()]).unwrap(),
            "b"
        );
        assert!(
            parse_lcov_path(&["--lcov".into()])
                .unwrap_err()
                .contains("requires a value")
        );
        assert!(
            parse_lcov_path(&["--what".into()])
                .unwrap_err()
                .contains("unknown coverage argument")
        );
    }

    #[test]
    fn parse_coverage_all_paths_defaults_and_flags() {
        let (lcov, html) = parse_coverage_all_paths(&[]).unwrap();
        assert_eq!(lcov, DEFAULT_LCOV_PATH);
        assert_eq!(html, "bitloops/target/llvm-cov-html");
        let (lcov, html) = parse_coverage_all_paths(&[
            "--lcov".into(),
            "c.lcov".into(),
            "--html-dir".into(),
            "html-out".into(),
        ])
        .unwrap();
        assert_eq!(lcov, "c.lcov");
        assert_eq!(html, "html-out");
        assert!(
            parse_coverage_all_paths(&["--html-dir".into()])
                .unwrap_err()
                .contains("requires a value")
        );
    }

    #[test]
    fn parse_compare_options_required_and_optional_flags() {
        let opts = parse_compare_options(&[
            "--lines".into(),
            "80".into(),
            "--functions".into(),
            "75".into(),
        ])
        .unwrap();
        assert_eq!(opts.lines_baseline, 80.0);
        assert_eq!(opts.functions_baseline, 75.0);
        assert_eq!(opts.epsilon, 0.5);
        assert_eq!(opts.lcov_path, DEFAULT_LCOV_PATH);

        let opts = parse_compare_options(&[
            "--lines".into(),
            "1".into(),
            "--functions".into(),
            "2".into(),
            "--epsilon".into(),
            "0.1".into(),
            "--lcov".into(),
            "m.lcov".into(),
        ])
        .unwrap();
        assert_eq!(opts.epsilon, 0.1);
        assert_eq!(opts.lcov_path, "m.lcov");

        assert!(
            parse_compare_options(&["--functions".into(), "75".into()])
                .unwrap_err()
                .contains("--lines is required")
        );
        assert!(
            parse_compare_options(&["--lines".into(), "80".into()])
                .unwrap_err()
                .contains("--functions is required")
        );
        assert!(
            parse_compare_options(&[
                "--lines".into(),
                "80".into(),
                "--functions".into(),
                "75".into(),
                "--bad".into(),
            ])
            .unwrap_err()
            .contains("unknown compare argument")
        );
    }

    #[test]
    fn resolve_workspace_path_joins_relative_paths() {
        let tmp = TempDir::new().expect("tempdir");
        let ws = std::fs::canonicalize(tmp.path()).expect("canonicalize");
        let abs_other = std::fs::canonicalize(tmp.path().join(".")).expect("canonicalize");
        assert_eq!(
            resolve_workspace_path(&ws, "rel/out.info"),
            ws.join("rel/out.info")
        );
        let abs_str = abs_other.to_str().expect("utf8 path");
        assert_eq!(resolve_workspace_path(&ws, abs_str), abs_other);
    }

    #[test]
    fn env_u64_reads_integer_or_default() {
        let _g = ENV_LOCK.lock().expect("env lock");
        let key = format!("XTASK_ENV_U64_TEST_{}", std::process::id());
        unsafe {
            env::remove_var(&key);
        }
        assert_eq!(env_u64(&key, 99).unwrap(), 99);
        unsafe {
            env::set_var(&key, "42");
        }
        assert_eq!(env_u64(&key, 99).unwrap(), 42);
        unsafe {
            env::set_var(&key, "not_int");
        }
        assert!(env_u64(&key, 0).unwrap_err().contains("must be an integer"));
        unsafe {
            env::remove_var(&key);
        }
    }

    #[test]
    fn fast_lane_uses_profile_defaults_and_allows_override() {
        let _g = ENV_LOCK.lock().expect("env lock");
        unsafe {
            env::remove_var("BITLOOPS_TEST_THREADS");
        }

        assert_eq!(test_threads_for_lane("fast").unwrap(), None);
        assert_eq!(test_threads_for_lane("smoke").unwrap(), None);

        unsafe {
            env::set_var("BITLOOPS_TEST_THREADS", "12");
        }
        assert_eq!(test_threads_for_lane("fast").unwrap(), Some(12));

        unsafe {
            env::set_var("BITLOOPS_TEST_THREADS", "0");
        }
        assert!(
            test_threads_for_lane("fast")
                .unwrap_err()
                .contains("greater than zero")
        );

        unsafe {
            env::remove_var("BITLOOPS_TEST_THREADS");
        }
    }

    #[test]
    fn nextest_profile_args_use_ci_on_github_actions() {
        let _g = ENV_LOCK.lock().expect("env lock");
        unsafe {
            env::remove_var("GITHUB_ACTIONS");
        }
        assert!(super::nextest_profile_args().is_empty());

        unsafe {
            env::set_var("GITHUB_ACTIONS", "true");
        }
        assert_eq!(
            super::nextest_profile_args(),
            vec!["--profile".to_string(), "ci".to_string()]
        );

        unsafe {
            env::remove_var("GITHUB_ACTIONS");
        }
    }

    #[test]
    fn nextest_list_args_swap_run_for_list_and_append_machine_output() {
        let args = vec![
            "nextest".to_string(),
            "run".to_string(),
            "--manifest-path".to_string(),
            BITLOOPS_MANIFEST.to_string(),
            "--no-default-features".to_string(),
            "--lib".to_string(),
        ];
        let out = super::nextest_list_args(&args, &["--profile".to_string(), "ci".to_string()])
            .expect("list args");
        assert_eq!(out[0], "nextest");
        assert_eq!(out[1], "list");
        assert!(out.contains(&"--profile".to_string()));
        assert!(out.contains(&"ci".to_string()));
        assert!(out.contains(&"--list-type".to_string()));
        assert!(out.contains(&"binaries-only".to_string()));
        assert!(out.contains(&"json-render-diagnostics".to_string()));
    }

    #[test]
    fn nextest_list_args_inserts_machine_output_before_test_binary_args() {
        let args = vec![
            "nextest".to_string(),
            "run".to_string(),
            "--manifest-path".to_string(),
            BITLOOPS_MANIFEST.to_string(),
            "--no-default-features".to_string(),
            "--features".to_string(),
            "slow-tests".to_string(),
            "--lib".to_string(),
            "--".to_string(),
            "host::devql::cucumber_bdd::devql_bdd_features_pass".to_string(),
            "--exact".to_string(),
        ];
        let out = super::nextest_list_args(&args, &[]).expect("list args");

        let separator_index = out
            .iter()
            .position(|arg| arg == "--")
            .expect("separator should be preserved");
        assert!(
            separator_index > 1,
            "separator should remain after nextest list arguments"
        );
        assert_eq!(
            out[separator_index + 1],
            "host::devql::cucumber_bdd::devql_bdd_features_pass"
        );
        assert_eq!(out[separator_index + 2], "--exact");
        assert!(
            out[..separator_index].contains(&"--list-type".to_string()),
            "list-only flags must stay before the test-binary separator"
        );
        assert!(
            !out[separator_index + 1..].contains(&"--list-type".to_string()),
            "list-only flags must not be forwarded to the test binary"
        );
    }

    #[test]
    fn test_lane_args_builds_expected_fragments() {
        let base = || {
            vec![
                "nextest".to_string(),
                "run".to_string(),
                "--manifest-path".to_string(),
                BITLOOPS_MANIFEST.to_string(),
                "--no-default-features".to_string(),
            ]
        };

        for lane in ["lib", "core"] {
            let mut expected = base();
            expected.push("--lib".to_string());
            assert_eq!(test_lane_args(lane).unwrap(), expected);
        }

        let mut expected = base();
        expected.push("--bin".to_string());
        expected.push("bitloops".to_string());
        assert_eq!(test_lane_args("cli").unwrap(), expected);

        assert_eq!(test_lane_args("fast").unwrap(), base());

        let smoke = test_lane_args("smoke").unwrap();
        assert!(smoke.contains(&"slow-tests".to_string()));
        assert_eq!(
            smoke.iter().filter(|s| *s == "--test").count(),
            MERGE_SMOKE_TARGETS.len()
        );
        for target in MERGE_SMOKE_TARGETS {
            assert!(
                smoke.contains(&(*target).to_string()),
                "smoke lane should include smoke target {target}"
            );
        }

        let args = test_lane_args("slow").unwrap();
        assert!(args.contains(&"slow-tests".to_string()));
        assert_eq!(
            args.iter().filter(|s| *s == "--test").count(),
            SLOW_TEST_TARGETS.len()
        );
        for t in SLOW_TEST_TARGETS {
            assert!(args.contains(&(*t).to_string()));
        }

        let args = test_lane_args("full").unwrap();
        assert!(args.contains(&"slow-tests".to_string()));
        assert!(!args.contains(&"--test".to_string()));

        let merge = test_lane_command_groups("merge").unwrap();
        assert_eq!(
            merge.len(),
            2,
            "merge lane should run fast + slow smoke groups"
        );
        assert_eq!(
            merge[0],
            base(),
            "merge lane should start with fast lane args"
        );
        assert_eq!(merge[1], smoke, "merge lane should append smoke-only args");

        assert!(
            test_lane_args("nope")
                .unwrap_err()
                .contains("unknown test lane")
        );
    }

    #[test]
    fn slow_lane_excludes_qat_acceptance() {
        assert!(
            !SLOW_TEST_TARGETS.contains(&"qat_acceptance"),
            "qat acceptance should remain outside the generic slow lane"
        );
    }

    #[test]
    fn cargo_manifest_and_qat_aliases_use_dedicated_qat_tests_feature() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");

        let manifest_path = workspace_root.join("bitloops").join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|err| panic!("reading {} failed: {err}", manifest_path.display()));
        assert!(
            manifest.contains("qat-tests = []"),
            "bitloops manifest should declare a dedicated qat-tests feature"
        );
        assert!(
            manifest.contains(
                "name = \"qat_acceptance\"\npath = \"tests/qat_acceptance.rs\"\nrequired-features = [\"qat-tests\"]"
            ),
            "qat_acceptance target should require only the qat-tests feature"
        );

        let config_path = workspace_root.join(".cargo").join("config.toml");
        let config = fs::read_to_string(&config_path)
            .unwrap_or_else(|err| panic!("reading {} failed: {err}", config_path.display()));

        for alias in [
            "qat = ",
            "qat-smoke = ",
            "qat-devql-capabilities = ",
            "qat-devql-sync = ",
            "qat-onboarding = ",
            "qat-devql-ingest = ",
        ] {
            let line = config
                .lines()
                .find(|line| line.starts_with(alias))
                .unwrap_or_else(|| {
                    panic!(
                        "missing cargo alias `{}` in {}",
                        alias,
                        config_path.display()
                    )
                });
            assert!(
                line.contains("--features qat-tests"),
                "cargo alias should enable the dedicated qat-tests feature: {line}"
            );
        }
    }

    #[test]
    fn is_test_file_detects_tests_dir_and_suffixes() {
        assert!(is_test_file(Path::new("tests/foo.rs")));
        assert!(is_test_file(Path::new("crate/tests/integration/main.rs")));
        assert!(is_test_file(Path::new("src/foo_test.rs")));
        assert!(is_test_file(Path::new("src/tests.rs")));
        assert!(is_test_file(Path::new("src/api_tests.rs")));
        assert!(!is_test_file(Path::new("src/api/client.rs")));
    }

    #[test]
    fn collect_rs_files_finds_nested_sources() {
        let tmp = TempDir::new().expect("tempdir");
        fs::create_dir_all(tmp.path().join("nested")).expect("mkdir");
        fs::write(tmp.path().join("a.rs"), "fn a() {}\n").expect("write");
        fs::write(tmp.path().join("nested/b.rs"), "fn b() {}\n").expect("write");
        let mut out = Vec::new();
        collect_rs_files(tmp.path(), &mut out).expect("collect");
        out.sort();
        assert_eq!(out.len(), 2);
        assert!(out[0].ends_with("a.rs"));
        assert!(out[1].ends_with("b.rs"));
    }

    #[test]
    fn count_lines_counts_newlines() {
        let tmp = TempDir::new().expect("tempdir");
        let p = tmp.path().join("x.rs");
        fs::write(&p, "a\nb\nc\n").expect("write");
        assert_eq!(count_lines(&p).unwrap(), 3);
    }

    #[test]
    fn run_file_size_check_passes_for_small_tree() {
        let tmp = TempDir::new().expect("tempdir");
        fs::create_dir_all(tmp.path().join("pkg")).expect("mkdir");
        let body: String = (0..50).map(|_| "//x\n").collect();
        fs::write(tmp.path().join("pkg/lib.rs"), body).expect("write");
        run_file_size_check(tmp.path()).expect("under default max");
    }

    #[test]
    fn run_file_size_check_fails_when_file_exceeds_default_max() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("big.rs");
        let mut f = fs::File::create(&path).expect("create");
        for i in 0..1001 {
            writeln!(f, "// line {i}").expect("write");
        }
        drop(f);
        let err = run_file_size_check(tmp.path()).unwrap_err();
        assert!(err.contains("file-size check failed"));
    }

    #[test]
    fn run_file_size_check_empty_tree_prints_message() {
        let tmp = TempDir::new().expect("tempdir");
        run_file_size_check(tmp.path()).expect("no rust files ok");
    }

    #[test]
    fn read_lcov_metrics_missing_file_errors() {
        let tmp = TempDir::new().expect("tempdir");
        let p = tmp.path().join("nope.lcov");
        let err = read_lcov_metrics(&p).unwrap_err();
        assert!(err.contains("failed to read"));
    }

    #[test]
    fn workspace_root_points_at_repo_workspace() {
        let root = workspace_root().expect("workspace root");
        assert!(root.join("bitloops").join("Cargo.toml").is_file());
    }

    #[test]
    fn llvm_cov_lcov_args_and_display_match() {
        let args = llvm_cov_lcov_cargo_args("/tmp/out.info");
        assert_eq!(args.last().map(String::as_str), Some("/tmp/out.info"));
        let full = prepend_cargo(&args);
        assert_eq!(full[0], "cargo");
        let display = llvm_cov_lcov_display_command("/tmp/out.info");
        assert!(display.contains("llvm-cov"));
        assert!(display.contains("/tmp/out.info"));
    }

    #[test]
    fn llvm_cov_report_html_args_round_trip() {
        let args = llvm_cov_report_html_cargo_args("target/h");
        assert!(args.contains(&"--html".to_string()));
        let display = llvm_cov_report_html_display_command("target/h");
        assert!(display.contains("report"));
        assert!(display.contains("target/h"));
    }

    #[test]
    fn parse_nextest_binary_paths_extracts_binary_paths() {
        let line =
            r#"{"rust-suites":[{"binary-id":"bitloops::tests/graphql","binary-path":"/tmp/t"}]}"#;
        assert_eq!(
            parse_nextest_binary_paths(line),
            vec![PathBuf::from("/tmp/t")]
        );
        assert!(
            parse_nextest_binary_paths(
                r#"{"rust-suites":[{"binary_path":"/tmp/a"},{"binary-path":"/tmp/b"}]}"#
            )
            .contains(&PathBuf::from("/tmp/a"))
        );
        assert_eq!(
            parse_nextest_binary_paths("not json"),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn collect_nextest_binary_paths_walks_nested_json() {
        let json = serde_json::json!({
            "outer": {
                "binary-path": "/tmp/one",
                "items": [
                    {"binary_path": "/tmp/two"},
                    {"ignored": true}
                ]
            }
        });
        let mut out = BTreeSet::new();
        collect_nextest_binary_paths(&json, &mut out);
        assert_eq!(
            out.into_iter().collect::<Vec<_>>(),
            vec![PathBuf::from("/tmp/one"), PathBuf::from("/tmp/two")]
        );
    }

    #[test]
    fn otool_heuristics_detect_dylib_and_rpath() {
        assert!(otool_list_output_links_libduckdb(
            "\t@rpath/libduckdb.dylib (compatibility version 0.0.0)\n"
        ));
        assert!(!otool_list_output_links_libduckdb(
            "\t/usr/lib/libSystem.B.dylib\n"
        ));
        let load = "cmd LC_RPATH\ncmdsize 32\npath @executable_path \n";
        assert!(otool_load_output_contains_rpath(load, "@executable_path"));
        assert!(!otool_load_output_contains_rpath(
            "path /usr/lib/ ",
            "@executable_path"
        ));
    }

    #[test]
    fn unknown_coverage_subcommand_message_lists_expected() {
        let e = unknown_coverage_subcommand_error("foo");
        assert!(e.contains("foo"));
        assert!(e.contains("run-lcov"));
    }

    #[test]
    fn ensure_cargo_subcommand_available_reports_install_hint_for_missing_subcommand() {
        let err = super::ensure_cargo_subcommand_available(
            "definitely-not-a-real-subcommand",
            "brew install cargo-nextest",
        )
        .unwrap_err();
        assert!(err.contains("cargo definitely-not-a-real-subcommand"));
        assert!(err.contains("brew install cargo-nextest"));
    }
}
