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
    let path_text = path.to_string_lossy();
    path_text.contains("/tests/")
        || path_text.ends_with("_test.rs")
        || path_text.ends_with("tests.rs")
        || path_text.ends_with("_tests.rs")
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
