#[cfg(unix)]
#[test]
fn ann_ab_script_mock_mode_writes_report_json() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");

    let report_path = temp.path().join("ann-ab-report.json");
    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--iterations",
            "3",
            "--neighbors",
            "999",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[("BITLOOPS_ANN_AB_MOCK", "1")],
    );

    assert!(
        output.status.success(),
        "script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");

    assert_eq!(report["config"]["iterations"].as_u64(), Some(3));
    assert_eq!(report["config"]["warmup"].as_u64(), Some(1));
    assert_eq!(report["config"]["neighbors"].as_u64(), Some(50));
    assert_eq!(report["config"]["bootstrap_sync"].as_bool(), Some(true));
    assert_eq!(report["valid_run"].as_bool(), Some(true));
    assert_eq!(report["ann_on_row_count"].as_u64(), Some(2));
    assert_eq!(report["ann_off_row_count"].as_u64(), Some(2));
    assert_eq!(
        report["metrics"]["ingest_ms"]["ann_on"]["samples"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(3)
    );
    assert_eq!(
        report["metrics"]["query_ms"]["ann_off"]["samples"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(3)
    );
}

#[cfg(unix)]
#[test]
fn ann_ab_script_requires_symbol_fqn_argument() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");

    let output = run_ann_ab_script(&repo_dir, &[], &[("BITLOOPS_ANN_AB_MOCK", "1")]);

    assert!(
        !output.status.success(),
        "script should fail when --symbol-fqn is missing"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--symbol-fqn is required"));
}

#[cfg(unix)]
#[test]
fn ann_ab_script_supports_skip_bootstrap_sync_flag() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");

    let report_path = temp.path().join("ann-ab-skip-bootstrap.json");
    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--skip-bootstrap-sync",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[("BITLOOPS_ANN_AB_MOCK", "1")],
    );

    assert!(
        output.status.success(),
        "script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    assert_eq!(report["config"]["bootstrap_sync"].as_bool(), Some(false));
}

#[cfg(unix)]
#[test]
fn ann_ab_script_honors_ort_dylib_override_path() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let ort_path = temp.path().join("libonnxruntime.dylib");
    fs::write(&ort_path, b"mock").expect("write ort dylib");
    let report_path = temp.path().join("ann-ab-report-ort-override.json");

    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--ort-dylib-path",
            ort_path.to_str().expect("ort path utf8"),
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[("BITLOOPS_ANN_AB_MOCK", "1")],
    );

    assert!(
        output.status.success(),
        "script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    assert_eq!(report["config"]["ort_source"].as_str(), Some("override"));
    assert_eq!(
        report["config"]["ort_dylib_path"].as_str(),
        ort_path.to_str()
    );
}

#[cfg(unix)]
#[test]
fn ann_ab_script_uses_repo_cached_ort_path_when_present() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let cache_path = repo_dir
        .join("target/qa/tools/onnxruntime/1.20.1")
        .join(current_platform_ort_tag())
        .join("lib")
        .join(current_platform_ort_lib_name());
    fs::create_dir_all(
        cache_path
            .parent()
            .expect("cached ORT path should have parent"),
    )
    .expect("create cached ort dir");
    fs::write(&cache_path, b"cached").expect("write cached ort file");
    let report_path = temp.path().join("ann-ab-report-ort-cache.json");

    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[("BITLOOPS_ANN_AB_MOCK", "1")],
    );

    assert!(
        output.status.success(),
        "script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    assert_eq!(report["config"]["ort_source"].as_str(), Some("repo_cache"));
    assert_eq!(
        report["config"]["ort_dylib_path"].as_str(),
        cache_path.to_str()
    );
}

#[cfg(unix)]
#[test]
fn ann_ab_script_generates_mode_config_with_expected_timeouts_and_defaults() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");

    let report_path = temp.path().join("ann-ab-config-check.json");
    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--iterations",
            "1",
            "--warmup",
            "0",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[("BITLOOPS_ANN_AB_MOCK", "1")],
    );

    assert!(
        output.status.success(),
        "script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    let run_id = report["run_id"].as_str().expect("run id in report");
    let mode_config_path = repo_dir
        .join("target/qa/runs")
        .join(run_id)
        .join("ann_on/home/xdg-config/bitloops/config.toml");
    let mode_config = fs::read_to_string(&mode_config_path).expect("read generated mode config");

    assert!(mode_config.contains("summary_mode = \"off\""));
    assert!(mode_config.contains("embedding_mode = \"deterministic\""));
    assert!(mode_config.contains("embedding_profile = \"local\""));
    assert!(mode_config.contains("startup_timeout_secs = 180"));
    assert!(mode_config.contains("request_timeout_secs = 180"));
}

#[cfg(unix)]
#[test]
fn ann_ab_script_daemon_lifecycle_commands_include_config_flag() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");

    let fake_binary_path = temp.path().join("fake-bitloops");
    let fake_commands_log = temp.path().join("fake-commands.log");
    let fake_ort_path = temp.path().join("libonnxruntime.dylib");
    let report_path = temp.path().join("ann-ab-daemon-config-check.json");
    fs::write(&fake_ort_path, b"fake-ort").expect("write fake ort dylib");

    let fake_binary = r#"#!/usr/bin/env bash
set -euo pipefail

command_log="${BITLOOPS_FAKE_LOG:?}"
printf '%s\n' "$*" >>"$command_log"

if [[ "${1:-}" == "daemon" && "${2:-}" == "status" ]]; then
  printf 'Enrichment pending jobs: 0\n'
  printf 'Enrichment running jobs: 0\n'
  printf 'Enrichment failed jobs: 0\n'
  exit 0
fi

if [[ "${1:-}" == "devql" && "${2:-}" == "query" ]]; then
  printf '[{"symbolFqn":"mock::source","clones":{"edges":[{"node":{"target_symbol_fqn":"mock::target","score":0.9}}]}}]\n'
  exit 0
fi

exit 0
"#;
    fs::write(&fake_binary_path, fake_binary).expect("write fake binary");
    let mut permissions = fs::metadata(&fake_binary_path)
        .expect("fake binary metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_binary_path, permissions).expect("set fake binary executable");

    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--binary",
            fake_binary_path.to_str().expect("fake binary path utf8"),
            "--ort-dylib-path",
            fake_ort_path.to_str().expect("fake ort path utf8"),
            "--iterations",
            "1",
            "--warmup",
            "0",
            "--skip-bootstrap-sync",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[(
            "BITLOOPS_FAKE_LOG",
            fake_commands_log.to_str().expect("fake commands log utf8"),
        )],
    );

    assert!(
        output.status.success(),
        "script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let command_log = fs::read_to_string(&fake_commands_log).expect("read fake command log");
    let daemon_starts = command_log
        .lines()
        .filter(|line| line.starts_with("daemon start "))
        .collect::<Vec<_>>();
    let daemon_statuses = command_log
        .lines()
        .filter(|line| line.starts_with("daemon status "))
        .collect::<Vec<_>>();
    let daemon_stops = command_log
        .lines()
        .filter(|line| line.starts_with("daemon stop "))
        .collect::<Vec<_>>();

    assert!(
        !daemon_starts.is_empty(),
        "expected daemon start commands in fake command log"
    );
    assert!(
        !daemon_statuses.is_empty(),
        "expected daemon status commands in fake command log"
    );
    assert!(
        !daemon_stops.is_empty(),
        "expected daemon stop commands in fake command log"
    );
    assert!(
        daemon_starts.iter().all(|line| line.contains(" --config ")),
        "daemon start command missing --config: {daemon_starts:?}"
    );
    assert!(
        daemon_statuses
            .iter()
            .all(|line| line.contains(" --config ")),
        "daemon status command missing --config: {daemon_statuses:?}"
    );
    assert!(
        daemon_stops.iter().all(|line| line.contains(" --config ")),
        "daemon stop command missing --config: {daemon_stops:?}"
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    assert_eq!(report["valid_run"].as_bool(), Some(true));
}

#[cfg(unix)]
#[test]
fn ann_ab_script_rejects_empty_query_results_when_nonempty_required() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let report_path = temp.path().join("ann-ab-report-empty.json");

    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[
            ("BITLOOPS_ANN_AB_MOCK", "1"),
            ("BITLOOPS_ANN_AB_MOCK_QUERY_ROWS", "0"),
        ],
    );

    assert!(
        !output.status.success(),
        "script should fail for empty query rows in non-empty mode"
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    assert_eq!(report["valid_run"].as_bool(), Some(false));
    assert!(
        report["failure_reason"]
            .as_str()
            .map(|s| s.contains("zero clone rows"))
            .unwrap_or(false),
        "expected failure reason to mention zero clone rows, got: {:?}",
        report["failure_reason"]
    );
}

#[cfg(unix)]
#[test]
fn ann_ab_script_rejects_empty_nested_clone_edges_when_nonempty_required() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let report_path = temp.path().join("ann-ab-report-empty-nested.json");

    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[
            ("BITLOOPS_ANN_AB_MOCK", "1"),
            ("BITLOOPS_ANN_AB_MOCK_QUERY_ROWS", "2"),
            ("BITLOOPS_ANN_AB_MOCK_QUERY_NESTED_EMPTY", "1"),
        ],
    );

    assert!(
        !output.status.success(),
        "script should fail for nested empty clone edges in non-empty mode"
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    assert_eq!(report["valid_run"].as_bool(), Some(false));
    assert!(
        report["failure_reason"]
            .as_str()
            .map(|s| s.contains("zero clone rows"))
            .unwrap_or(false),
        "expected failure reason to mention zero clone rows, got: {:?}",
        report["failure_reason"]
    );
}

#[cfg(unix)]
#[test]
fn ann_ab_script_rejects_invalid_query_json() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let report_path = temp.path().join("ann-ab-report-invalid-json.json");

    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[
            ("BITLOOPS_ANN_AB_MOCK", "1"),
            ("BITLOOPS_ANN_AB_MOCK_INVALID_QUERY_JSON", "1"),
        ],
    );

    assert!(
        !output.status.success(),
        "script should fail for invalid query JSON"
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    assert_eq!(report["valid_run"].as_bool(), Some(false));
    assert!(
        report["failure_reason"]
            .as_str()
            .map(|s| s.contains("invalid JSON"))
            .unwrap_or(false),
        "expected invalid JSON failure reason, got: {:?}",
        report["failure_reason"]
    );
}

#[cfg(unix)]
#[test]
fn ann_ab_script_rejects_ingest_runtime_error_markers() {
    use std::fs;

    let temp = tempfile::tempdir().expect("temp dir");
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).expect("create repo dir");
    let report_path = temp.path().join("ann-ab-report-ingest-marker.json");

    let output = run_ann_ab_script(
        &repo_dir,
        &[
            "--symbol-fqn",
            "src/services/orders.ts::OrderService.create",
            "--output",
            report_path.to_str().expect("report path utf8"),
        ],
        &[
            ("BITLOOPS_ANN_AB_MOCK", "1"),
            ("BITLOOPS_ANN_AB_MOCK_INGEST_ERROR", "1"),
        ],
    );

    assert!(
        !output.status.success(),
        "script should fail when ingest failure markers appear"
    );

    let report_raw = fs::read_to_string(&report_path).expect("read report json");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("parse report json");
    assert_eq!(report["valid_run"].as_bool(), Some(false));
    assert!(
        report["failure_reason"]
            .as_str()
            .map(|s| s.contains("embedding runtime failure marker"))
            .unwrap_or(false),
        "expected ingest marker failure reason, got: {:?}",
        report["failure_reason"]
    );
}

#[cfg(unix)]
fn run_ann_ab_script(
    repo_dir: &std::path::Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> std::process::Output {
    use std::process::Command;

    let script_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("qa")
        .join("semantic_clones_ann_ab.sh");
    assert!(
        script_path.is_file(),
        "missing script at {}",
        script_path.display()
    );

    let mut command = Command::new("bash");
    command
        .arg(script_path)
        .arg("--repo")
        .arg(repo_dir)
        .args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("run ann ab script")
}

#[cfg(unix)]
fn current_platform_ort_tag() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "osx-arm64",
        ("macos", "x86_64") => "osx-x86_64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-aarch64",
        (os, arch) => panic!("unsupported platform for ORT cache test: {os}/{arch}"),
    }
}

#[cfg(unix)]
fn current_platform_ort_lib_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "libonnxruntime.dylib",
        "linux" => "libonnxruntime.so",
        os => panic!("unsupported platform for ORT cache test library name: {os}"),
    }
}
