use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use super::types::ListedArtefact;

pub fn run_testlens_or_panic(args: &[&str]) -> String {
    let output = run_testlens(args);
    if !output.status.success() {
        panic!(
            "testlens command failed: {:?}\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8(output.stdout).expect("stdout should be valid UTF-8")
}

pub fn run_testlens_allow_failure(args: &[&str]) -> Output {
    run_testlens(args)
}

pub fn list_artefacts_by_kind(db_path: &Path, commit: &str, kind: &str) -> Vec<ListedArtefact> {
    let db_path = db_path.to_string_lossy();
    let output =
        run_testlens_or_panic(&["list", "--db", &db_path, "--commit", commit, "--kind", kind]);

    serde_json::from_str(&output).expect("failed to parse list JSON output")
}

pub fn run_cargo_in_dir_or_panic(workdir: &Path, args: &[&str]) -> String {
    let output = run_command_in_dir(env!("CARGO"), workdir, args);
    assert_command_success("cargo", workdir, args, &output);
    String::from_utf8(output.stdout).expect("stdout should be valid UTF-8")
}

pub fn run_command_in_dir_or_panic(program: &str, workdir: &Path, args: &[&str]) -> String {
    let output = run_command_in_dir(program, workdir, args);
    assert_command_success(program, workdir, args, &output);
    String::from_utf8(output.stdout).expect("stdout should be valid UTF-8")
}

pub fn run_command_in_dir_allow_failure(program: &str, workdir: &Path, args: &[&str]) -> Output {
    run_command_in_dir(program, workdir, args)
}

fn run_testlens(args: &[&str]) -> Output {
    Command::new(env!("CARGO"))
        .current_dir(project_root())
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("testlens")
        .arg("--")
        .args(args)
        .output()
        .expect("failed to execute testlens command")
}

fn run_command_in_dir(program: &str, workdir: &Path, args: &[&str]) -> Output {
    Command::new(program)
        .current_dir(workdir)
        .args(args)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to execute command '{}' in {}: {}\nargs: {:?}",
                program,
                workdir.display(),
                error,
                args
            )
        })
}

fn assert_command_success(program: &str, workdir: &Path, args: &[&str], output: &Output) {
    if output.status.success() {
        return;
    }

    panic!(
        "{} command failed in {}: {:?}\nstdout:\n{}\nstderr:\n{}",
        program,
        workdir.display(),
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn project_root() -> PathBuf {
    let cwd = std::env::current_dir().expect("failed to resolve current working directory");

    for candidate in cwd.ancestors() {
        if candidate.join("Cargo.toml").exists() {
            return candidate.to_path_buf();
        }
    }

    panic!(
        "failed to locate project root from current working directory {}",
        cwd.display()
    );
}
