use std::path::{Path, PathBuf};

use super::cli::{project_root, run_command_in_dir_or_panic};

pub fn copy_real_rust_fixture(target_root: &Path) -> PathBuf {
    let source_repo = project_root().join("testlens-fixture-rust");
    let target_repo = target_root.join("testlens-fixture-rust");
    let source_arg = source_repo.to_string_lossy().to_string();
    let target_arg = target_repo.to_string_lossy().to_string();

    run_command_in_dir_or_panic("cp", target_root, &["-R", &source_arg, &target_arg]);
    target_repo
}

pub fn generate_rust_lcov(repo_dir: &Path, lcov_path: &Path) {
    let lcov_arg = lcov_path.to_string_lossy().to_string();
    run_command_in_dir_or_panic(
        "cargo",
        repo_dir,
        &["llvm-cov", "--lcov", "--output-path", &lcov_arg],
    );
}
