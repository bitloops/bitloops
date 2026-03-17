use std::fs;
use std::path::{Path, PathBuf};

use super::cli::{
    project_root, run_command_in_dir_allow_failure, run_command_in_dir_or_panic,
    run_testlens_or_panic,
};

pub fn copy_real_typescript_fixture(target_root: &Path) -> PathBuf {
    let source_repo = project_root().join("testlens-fixture");
    let target_repo = target_root.join("testlens-fixture");
    let source_arg = source_repo.to_string_lossy().to_string();
    let target_arg = target_repo.to_string_lossy().to_string();

    run_command_in_dir_or_panic("cp", target_root, &["-R", &source_arg, &target_arg]);

    let coverage_dir = target_repo.join("coverage");
    if coverage_dir.exists() {
        fs::remove_dir_all(&coverage_dir).expect("failed to clear copied coverage directory");
    }

    target_repo
}

pub fn index_real_typescript_fixture(
    db_path: &Path,
    repo_dir: &Path,
    commit_sha: &str,
    jest_json_path: &Path,
) {
    let db = db_path.to_string_lossy().to_string();
    let repo_dir_arg = repo_dir.to_string_lossy().to_string();
    let jest_json_arg = jest_json_path.to_string_lossy().to_string();

    run_testlens_or_panic(&["init", "--db", &db]);
    run_testlens_or_panic(&[
        "ingest-production-artefacts",
        "--db",
        &db,
        "--repo-dir",
        &repo_dir_arg,
        "--commit",
        commit_sha,
    ]);
    run_testlens_or_panic(&[
        "ingest-tests",
        "--db",
        &db,
        "--repo-dir",
        &repo_dir_arg,
        "--commit",
        commit_sha,
    ]);

    let jest_output = run_command_in_dir_allow_failure(
        "npx",
        repo_dir,
        &[
            "jest",
            "--coverage",
            "--json",
            "--outputFile",
            &jest_json_arg,
            "--runInBand",
        ],
    );
    assert!(
        jest_json_path.exists(),
        "expected Jest JSON output at {} even when Jest exits with {:?}\nstdout:\n{}\nstderr:\n{}",
        jest_json_path.display(),
        jest_output.status.code(),
        String::from_utf8_lossy(&jest_output.stdout),
        String::from_utf8_lossy(&jest_output.stderr)
    );

    let lcov_path = repo_dir.join("coverage/lcov.info");
    assert!(
        lcov_path.exists(),
        "expected LCOV report at {}",
        lcov_path.display()
    );
    let lcov_arg = lcov_path.to_string_lossy().to_string();

    run_testlens_or_panic(&[
        "ingest-coverage",
        "--db",
        &db,
        "--lcov",
        &lcov_arg,
        "--commit",
        commit_sha,
        "--scope",
        "workspace",
    ]);
    run_testlens_or_panic(&[
        "ingest-results",
        "--db",
        &db,
        "--jest-json",
        &jest_json_arg,
        "--commit",
        commit_sha,
    ]);
}
