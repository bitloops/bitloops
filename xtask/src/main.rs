use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

const BITLOOPS_MANIFEST: &str = "bitloops/Cargo.toml";
const SLOW_TEST_TARGETS: &[&str] = &[
    "claude_git_hooks_integration",
    "copilot_integration",
    "cursor_e2e_scenarios",
    "dashboard_bundle_lifecycle_e2e",
    "e2e_scenario_groups",
    "graphql",
    "performance",
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
    eprintln!("  test <lib|core|cli|fast|slow|full>");
    eprintln!("  coverage run-lcov [--lcov <path>]");
    eprintln!("  coverage metrics [--lcov <path>]");
    eprintln!(
        "  coverage compare --lines <pct> --functions <pct> [--epsilon 0.05] [--lcov <path>]"
    );
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
    let workspace_root = workspace_root()?;
    let lane_args = test_lane_args(lane)?;

    let mut compile_args = lane_args.clone();
    compile_args.push("--no-run".to_string());
    compile_args.push("--message-format=json-render-diagnostics".to_string());

    let compiled_binaries = collect_test_binaries(&workspace_root, &compile_args)?;
    if should_sign() {
        for binary in compiled_binaries {
            codesign_binary(&binary)?;
        }
    }

    let mut run_args = lane_args;
    run_args.push("--".to_string());
    run_args.push("--format=terse".to_string());
    let mut command = vec!["cargo".to_string()];
    command.extend(run_args.iter().cloned());
    run_command_owned(
        &workspace_root,
        &format!("cargo {}", run_args.join(" ")),
        &command,
    )
}

fn run_coverage_command(subcommand: &str, raw_args: Vec<String>) -> Result<(), String> {
    match subcommand {
        "run-lcov" => {
            let lcov_path = parse_lcov_path(&raw_args)?;
            run_coverage_lcov(&lcov_path)
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
        _ => Err(format!(
            "unknown coverage subcommand `{subcommand}` (expected: run-lcov|metrics|compare)"
        )),
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

fn run_coverage_lcov(lcov_path: &str) -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let resolved_lcov_path = resolve_workspace_path(&workspace_root, lcov_path);
    if let Some(parent) = resolved_lcov_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let resolved_lcov_path = resolved_lcov_path.to_string_lossy().to_string();

    run_command(
        &workspace_root,
        &format!(
            "cargo llvm-cov --manifest-path bitloops/Cargo.toml --workspace --all-targets --features slow-tests --no-default-features --lcov --output-path {resolved_lcov_path}"
        ),
        &[
            "llvm-cov",
            "--manifest-path",
            BITLOOPS_MANIFEST,
            "--workspace",
            "--all-targets",
            "--features",
            "slow-tests",
            "--no-default-features",
            "--lcov",
            "--output-path",
            &resolved_lcov_path,
        ],
    )
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
    let mut epsilon = 0.05;
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

fn test_lane_args(lane: &str) -> Result<Vec<String>, String> {
    let mut args = vec![
        "test".to_string(),
        "--manifest-path".to_string(),
        BITLOOPS_MANIFEST.to_string(),
        "--no-fail-fast".to_string(),
        "--no-default-features".to_string(),
    ];

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
            args.push("--features".to_string());
            args.push("slow-tests".to_string());
            for target in SLOW_TEST_TARGETS {
                args.push("--test".to_string());
                args.push((*target).to_string());
            }
        }
        "full" => {
            args.push("--features".to_string());
            args.push("slow-tests".to_string());
        }
        _ => {
            return Err(format!(
                "unknown test lane `{lane}` (expected: lib|core|cli|fast|slow|full)"
            ));
        }
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
        .map_err(|err| format!("failed to start cargo test compile step: {err}"))?;

    if !output.status.success() {
        return Err("command failed: cargo test compile step".to_string());
    }

    let mut binaries = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(json) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(executable) = json.get("executable").and_then(|v| v.as_str()) else {
            continue;
        };
        let is_test_profile = json
            .get("profile")
            .and_then(|p| p.get("test"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_test_profile {
            binaries.insert(PathBuf::from(executable));
        }
    }

    Ok(binaries.into_iter().collect())
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
    Ok(text.contains("@rpath/libduckdb.dylib"))
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
    Ok(text.contains("cmd LC_RPATH") && text.contains(&format!("path {rpath} ")))
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
    use super::{is_regression, parse_lcov_metrics};

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
        assert!(!is_regression(79.95, 80.0, 0.05));
        assert!(is_regression(79.94, 80.0, 0.05));
        assert!(!is_regression(74.95, 75.0, 0.05));
        assert!(is_regression(74.94, 75.0, 0.05));
    }
}
