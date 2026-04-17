use super::managed::archive::{
    ManagedEmbeddingsArchiveKind, extract_managed_embeddings_bundle_entries_from_file, sha256_hex,
};
use super::managed::config::{
    load_managed_embeddings_install_metadata, managed_embeddings_binary_name,
    raw_managed_runtime_command,
};
use super::managed::install::{
    install_managed_embeddings_binary_from_release_bytes, managed_embeddings_asset_spec_for,
};
use super::{
    EmbeddingsCommand, EmbeddingsInstallState, EmbeddingsRuntime,
    ManagedEmbeddingsBinaryInstallOutcome, clear_cache_for_profile, doctor_profile,
    inspect_embeddings_install_state, install_or_bootstrap_embeddings,
    platform_embeddings_gateway_url_override, pull_profile, with_managed_embeddings_install_hook,
};
use crate::cli::Cli;
use crate::config::{BITLOOPS_CONFIG_RELATIVE_PATH, resolve_embedding_capability_config_for_repo};
use crate::test_support::process_state::{enter_process_state, with_env_vars};
use clap::Parser;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use xz2::write::XzEncoder;
use zip::write::FileOptions;

const LOCAL_PULL_TIMEOUT_SECS: u64 = 300;
const TEST_MANAGED_EMBEDDINGS_VERSION: &str = "v1.2.3";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PullRuntimeConfig {
    startup_timeout_secs: u64,
    request_timeout_secs: u64,
}

fn pull_runtime_client_config(
    _repo_root: &Path,
    _config_path: &Path,
    capability: &crate::config::EmbeddingCapabilityConfig,
    profile_name: &str,
) -> PullRuntimeConfig {
    let profile = capability
        .inference
        .profiles
        .get(profile_name)
        .expect("embedding profile for timeout test");
    let runtime_name = profile
        .runtime
        .as_deref()
        .expect("runtime-backed embedding profile for timeout test");
    let runtime = capability
        .inference
        .runtimes
        .get(runtime_name)
        .expect("runtime config for timeout test");
    PullRuntimeConfig {
        startup_timeout_secs: runtime.startup_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS),
        request_timeout_secs: runtime.request_timeout_secs.max(LOCAL_PULL_TIMEOUT_SECS),
    }
}

fn write_embedding_config(repo_root: &Path) {
    fs::write(
        repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH),
        r#"
[runtime]
local_dev = false

[semantic_clones.inference]
code_embeddings = "local"
summary_embeddings = "local"

[inference.runtimes.bitloops_local_embeddings]
command = "bitloops-local-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.local]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "bge-m3"
cache_dir = ".bitloops/embeddings/models"

[inference.profiles.openai]
task = "embeddings"
driver = "openai"
model = "text-embedding-3-large"
api_key = "secret"
"#,
    )
    .expect("write config");
}

fn seed_repo() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path();
    crate::test_support::git_fixtures::init_test_repo(
        repo_root,
        "main",
        "Alice",
        "alice@example.com",
    );
    write_embedding_config(repo_root);
    dir
}

#[cfg(unix)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"#!/bin/sh
model_name="bge-m3"
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":[[0.1,0.2,0.3]],"model":"%s"}\n' "$req_id" "$model_name"
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"%s"}\n' "$req_id" "$model_name"
      exit 0
      ;;
    *)
      printf '{"id":"%s","ok":false,"error":{"message":"unexpected request"}}\n' "$req_id"
      ;;
  esac
done
"#;
    fs::write(&script_path, script).expect("write fake runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod fake runtime script");
    ("sh".to_string(), vec![script_path.display().to_string()])
}

#[cfg(windows)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"
$modelName = "bge-m3"
$ready = @{
  event = "ready"
  protocol = 1
  capabilities = @("embed", "shutdown")
}
$ready | ConvertTo-Json -Compress
$stdin = [Console]::In
while (($line = $stdin.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.cmd) {
    "embed" {
      $response = @{
        id = $request.id
        ok = $true
        vectors = @(@(0.1, 0.2, 0.3))
        model = $modelName
      }
    }
    "shutdown" {
      $response = @{
        id = $request.id
        ok = $true
        model = $modelName
      }
      $response | ConvertTo-Json -Compress
      break
    }
    default {
      $response = @{
        id = $request.id
        ok = $false
        error = @{
          message = "unexpected request"
        }
      }
    }
  }
  $response | ConvertTo-Json -Compress
}
"#;
    fs::write(&script_path, script).expect("write fake runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.display().to_string(),
        ],
    )
}

#[cfg(unix)]
fn fake_managed_runtime_path(repo_root: &Path) -> PathBuf {
    let script_path = repo_root.join(".bitloops/test-bin/fake-managed-bitloops-local-embeddings");
    let (_, args) = fake_runtime_command_and_args(repo_root);
    fs::copy(&args[0], &script_path).expect("copy managed runtime script");
    script_path
}

#[cfg(windows)]
fn fake_managed_runtime_path(repo_root: &Path) -> PathBuf {
    let script_dir = repo_root.join(".bitloops/test-bin");
    fs::create_dir_all(&script_dir).expect("create managed runtime dir");
    let powershell_script = script_dir.join("fake-managed-bitloops-local-embeddings.ps1");
    let launcher = script_dir.join("fake-managed-bitloops-local-embeddings.cmd");
    let (_, args) = fake_runtime_command_and_args(repo_root);
    fs::copy(&args[4], &powershell_script).expect("copy managed powershell script");
    fs::write(
        &launcher,
        format!(
            "@echo off\r\npowershell -NoProfile -ExecutionPolicy Bypass -File \"{}\" %*\r\n",
            powershell_script.display()
        ),
    )
    .expect("write managed runtime launcher");
    launcher
}

fn create_archive_bytes(
    archive_kind: ManagedEmbeddingsArchiveKind,
    binary_name: &str,
    payload: &[u8],
) -> Vec<u8> {
    let bundle_root = Path::new("managed-embed-runtime");
    let binary_path = bundle_root.join(binary_name);
    let support_path = bundle_root.join("_internal").join("Python");
    match archive_kind {
        ManagedEmbeddingsArchiveKind::Zip => {
            let cursor = Cursor::new(Vec::<u8>::new());
            let mut writer = zip::ZipWriter::new(cursor);
            writer
                .start_file(
                    binary_path.to_string_lossy(),
                    FileOptions::default().unix_permissions(0o755),
                )
                .expect("start zip entry");
            writer.write_all(payload).expect("write zip payload");
            writer
                .add_directory(
                    bundle_root.join("_internal").to_string_lossy(),
                    FileOptions::default(),
                )
                .expect("add support directory");
            writer
                .start_file(support_path.to_string_lossy(), FileOptions::default())
                .expect("start support zip entry");
            writer
                .write_all(b"python-runtime")
                .expect("write support payload");
            writer.finish().expect("finish zip").into_inner()
        }
        ManagedEmbeddingsArchiveKind::TarXz => {
            let encoder = XzEncoder::new(Vec::new(), 6);
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, &binary_path, payload)
                .expect("append tar entry");
            let mut support_header = tar::Header::new_gnu();
            support_header.set_size(b"python-runtime".len() as u64);
            support_header.set_mode(0o644);
            support_header.set_cksum();
            builder
                .append_data(&mut support_header, &support_path, &b"python-runtime"[..])
                .expect("append support tar entry");
            let encoder = builder.into_inner().expect("finish tar builder");
            encoder.finish().expect("finish xz encoder")
        }
    }
}

#[test]
fn extract_managed_embeddings_bundle_entries_from_file_reads_zip_archive() {
    assert_archive_file_extraction_matches_payload(ManagedEmbeddingsArchiveKind::Zip);
}

#[test]
fn extract_managed_embeddings_bundle_entries_from_file_reads_tar_xz_archive() {
    assert_archive_file_extraction_matches_payload(ManagedEmbeddingsArchiveKind::TarXz);
}

fn assert_archive_file_extraction_matches_payload(archive_kind: ManagedEmbeddingsArchiveKind) {
    let temp_dir = TempDir::new().expect("tempdir");
    let binary_name = managed_embeddings_binary_name();
    let payload = b"managed-runtime-payload";
    let archive_bytes = create_archive_bytes(archive_kind, binary_name, payload);
    let archive_name = match archive_kind {
        ManagedEmbeddingsArchiveKind::Zip => "managed-runtime.zip",
        ManagedEmbeddingsArchiveKind::TarXz => "managed-runtime.tar.xz",
    };
    let archive_path = temp_dir.path().join(archive_name);
    fs::write(&archive_path, archive_bytes).expect("write archive");

    let bundle_entries = extract_managed_embeddings_bundle_entries_from_file(
        &archive_path,
        archive_kind,
        binary_name,
    )
    .expect("extract archive");

    assert!(bundle_entries.iter().any(|entry| {
        entry.relative_path.as_os_str() == binary_name && entry.bytes.as_slice() == payload
    }));
    assert!(
        bundle_entries
            .iter()
            .any(|entry| { entry.relative_path == Path::new("_internal").join("Python") })
    );
}

fn write_runtime_only_config(repo_root: &Path, command: &str, args: &[String]) {
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH),
        format!(
            r#"
[runtime]
local_dev = false

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5
"#
        ),
    )
    .expect("write runtime-only config");
}

#[test]
fn embeddings_cli_parses_subcommands() {
    let parsed = Cli::try_parse_from(["bitloops", "embeddings", "install"])
        .expect("embeddings install should parse");
    let Some(crate::cli::Commands::Embeddings(args)) = parsed.command else {
        panic!("expected embeddings command");
    };
    assert!(matches!(args.command, Some(EmbeddingsCommand::Install(_))));

    let parsed = Cli::try_parse_from(["bitloops", "embeddings", "doctor"])
        .expect("embeddings command should parse");
    let Some(crate::cli::Commands::Embeddings(args)) = parsed.command else {
        panic!("expected embeddings command");
    };
    assert!(matches!(args.command, Some(EmbeddingsCommand::Doctor(_))));
}

#[test]
fn embeddings_cli_parses_pull_and_clear_cache() {
    let parsed = Cli::try_parse_from(["bitloops", "embeddings", "pull", "local_code"])
        .expect("pull should parse");
    let Some(crate::cli::Commands::Embeddings(args)) = parsed.command else {
        panic!("expected embeddings command");
    };
    assert!(matches!(args.command, Some(EmbeddingsCommand::Pull(_))));

    let parsed = Cli::try_parse_from(["bitloops", "embeddings", "clear-cache", "local_code"])
        .expect("clear-cache should parse");
    let Some(crate::cli::Commands::Embeddings(args)) = parsed.command else {
        panic!("expected embeddings command");
    };
    assert!(matches!(
        args.command,
        Some(EmbeddingsCommand::ClearCache(_))
    ));
}

#[test]
fn embeddings_install_supports_platform_runtime_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "embeddings",
        "install",
        "--runtime",
        "platform",
        "--gateway-url",
        "https://gateway.example/v1/embeddings",
        "--api-key-env",
        "BITLOOPS_PLATFORM_GATEWAY_TOKEN",
    ])
    .expect("platform embeddings install should parse");
    let Some(crate::cli::Commands::Embeddings(args)) = parsed.command else {
        panic!("expected embeddings command");
    };
    let Some(EmbeddingsCommand::Install(args)) = args.command else {
        panic!("expected install command");
    };

    assert_eq!(args.runtime, EmbeddingsRuntime::Platform);
    assert_eq!(
        args.gateway_url.as_deref(),
        Some("https://gateway.example/v1/embeddings")
    );
    assert_eq!(args.api_key_env, "BITLOOPS_PLATFORM_GATEWAY_TOKEN");
}

#[test]
fn platform_embeddings_gateway_url_override_prefers_explicit_flag() {
    with_env_vars(
        &[(
            "BITLOOPS_PLATFORM_GATEWAY_URL",
            Some("https://platform.example"),
        )],
        || {
            assert_eq!(
                platform_embeddings_gateway_url_override(Some(
                    "https://override.example/v1/embeddings"
                ))
                .as_deref(),
                Some("https://override.example/v1/embeddings")
            );
        },
    );
}

#[test]
fn platform_embeddings_gateway_url_override_derives_from_platform_gateway_env() {
    with_env_vars(
        &[(
            "BITLOOPS_PLATFORM_GATEWAY_URL",
            Some("https://platform.example"),
        )],
        || {
            assert_eq!(
                platform_embeddings_gateway_url_override(None).as_deref(),
                Some("https://platform.example/v1/embeddings")
            );
        },
    );
}

#[test]
fn platform_embeddings_gateway_url_override_is_optional() {
    with_env_vars(&[("BITLOOPS_PLATFORM_GATEWAY_URL", None)], || {
        assert_eq!(platform_embeddings_gateway_url_override(None), None);
    });
}

#[test]
fn managed_embeddings_asset_spec_matches_external_release_names() {
    assert_eq!(
        managed_embeddings_asset_spec_for("macos", "aarch64", TEST_MANAGED_EMBEDDINGS_VERSION)
            .expect("mac arm asset")
            .asset_name,
        format!(
            "bitloops-local-embeddings-{TEST_MANAGED_EMBEDDINGS_VERSION}-aarch64-apple-darwin.zip"
        )
    );
    assert_eq!(
        managed_embeddings_asset_spec_for("macos", "x86_64", TEST_MANAGED_EMBEDDINGS_VERSION)
            .expect("mac x64 asset")
            .asset_name,
        format!(
            "bitloops-local-embeddings-{TEST_MANAGED_EMBEDDINGS_VERSION}-x86_64-apple-darwin.zip"
        )
    );
    assert_eq!(
        managed_embeddings_asset_spec_for("linux", "aarch64", TEST_MANAGED_EMBEDDINGS_VERSION)
            .expect("linux arm asset")
            .asset_name,
        format!(
            "bitloops-local-embeddings-{TEST_MANAGED_EMBEDDINGS_VERSION}-aarch64-unknown-linux-gnu.tar.xz"
        )
    );
    assert_eq!(
        managed_embeddings_asset_spec_for("linux", "x86_64", TEST_MANAGED_EMBEDDINGS_VERSION)
            .expect("linux x64 asset")
            .asset_name,
        format!(
            "bitloops-local-embeddings-{TEST_MANAGED_EMBEDDINGS_VERSION}-x86_64-unknown-linux-gnu.tar.xz"
        )
    );
    assert_eq!(
        managed_embeddings_asset_spec_for("windows", "x86_64", TEST_MANAGED_EMBEDDINGS_VERSION)
            .expect("windows x64 asset")
            .asset_name,
        format!(
            "bitloops-local-embeddings-{TEST_MANAGED_EMBEDDINGS_VERSION}-x86_64-pc-windows-msvc.zip"
        )
    );
}

#[test]
fn managed_install_rejects_mismatched_digest() {
    let repo = TempDir::new().expect("tempdir");
    let data = TempDir::new().expect("data tempdir");
    let archive_kind = ManagedEmbeddingsArchiveKind::Zip;
    let archive = create_archive_bytes(archive_kind, managed_embeddings_binary_name(), b"fake");
    let data_root = data.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[("BITLOOPS_TEST_DATA_DIR_OVERRIDE", Some(data_root.as_str()))],
    );

    let err = install_managed_embeddings_binary_from_release_bytes(
        TEST_MANAGED_EMBEDDINGS_VERSION,
        "asset.zip",
        archive_kind,
        &"0".repeat(64),
        &archive,
    )
    .expect_err("digest mismatch should fail");

    assert!(err.to_string().contains("digest mismatch"));
}

#[test]
fn managed_install_writes_binary_and_metadata() {
    let repo = TempDir::new().expect("tempdir");
    let data = TempDir::new().expect("data tempdir");
    let archive_kind = ManagedEmbeddingsArchiveKind::Zip;
    let archive = create_archive_bytes(archive_kind, managed_embeddings_binary_name(), b"fake");
    let expected_digest = sha256_hex(&archive);
    let data_root = data.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[("BITLOOPS_TEST_DATA_DIR_OVERRIDE", Some(data_root.as_str()))],
    );

    let outcome = install_managed_embeddings_binary_from_release_bytes(
        TEST_MANAGED_EMBEDDINGS_VERSION,
        "asset.zip",
        archive_kind,
        &expected_digest,
        &archive,
    )
    .expect("install managed runtime");

    assert_eq!(outcome.version, TEST_MANAGED_EMBEDDINGS_VERSION);
    assert!(outcome.binary_path.is_file());
    assert!(
        outcome
            .binary_path
            .parent()
            .expect("binary parent")
            .join("_internal")
            .join("Python")
            .is_file()
    );
    let metadata =
        fs::read_to_string(super::managed_embeddings_metadata_path().expect("metadata path"))
            .expect("read metadata");
    assert!(metadata.contains(&format!(
        "\"version\": \"{TEST_MANAGED_EMBEDDINGS_VERSION}\""
    )));
}

#[test]
fn doctor_uses_active_profile_when_not_explicit() {
    let repo = seed_repo();
    let capability = resolve_embedding_capability_config_for_repo(repo.path());
    let lines = doctor_profile(repo.path(), &capability, None).expect("doctor report");

    assert!(lines.iter().any(|line| line.contains("Profile: local")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Kind: bitloops_embeddings_ipc"))
    );
}

#[test]
fn doctor_reports_hosted_profile_sensibly() {
    let repo = seed_repo();
    let capability = resolve_embedding_capability_config_for_repo(repo.path());
    let lines =
        doctor_profile(repo.path(), &capability, Some("openai")).expect("hosted doctor report");

    assert!(lines.iter().any(|line| line.contains("Profile: openai")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Runtime: hosted profile"))
    );
}

#[test]
fn doctor_reports_disabled_when_no_profiles_exist() {
    let repo = TempDir::new().expect("tempdir");
    let capability = resolve_embedding_capability_config_for_repo(repo.path());
    let lines = doctor_profile(repo.path(), &capability, None).expect("disabled report");

    assert!(
        lines
            .iter()
            .any(|line| line.contains("Embeddings: disabled"))
    );
}

#[test]
fn load_managed_embeddings_install_metadata_reports_invalid_json() {
    let repo = TempDir::new().expect("tempdir");
    let data = TempDir::new().expect("data tempdir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Alice",
        "alice@example.com",
    );
    let data_root = data.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[("BITLOOPS_TEST_DATA_DIR_OVERRIDE", Some(data_root.as_str()))],
    );

    let metadata_path = super::managed_embeddings_metadata_path().expect("metadata path");
    fs::create_dir_all(metadata_path.parent().expect("metadata parent"))
        .expect("create metadata parent");
    fs::write(&metadata_path, "{ not valid json").expect("write corrupt metadata");

    let err = load_managed_embeddings_install_metadata().expect_err("invalid metadata error");
    assert!(
        err.to_string()
            .contains("parsing managed embeddings metadata"),
        "unexpected error: {err}"
    );
}

#[test]
fn doctor_reports_invalid_managed_runtime_metadata() {
    let repo = TempDir::new().expect("tempdir");
    let data = TempDir::new().expect("data tempdir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Alice",
        "alice@example.com",
    );
    write_embedding_config(repo.path());
    let data_root = data.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[("BITLOOPS_TEST_DATA_DIR_OVERRIDE", Some(data_root.as_str()))],
    );

    with_managed_embeddings_install_hook(
        move |_repo_root| {
            Ok(ManagedEmbeddingsBinaryInstallOutcome {
                version: TEST_MANAGED_EMBEDDINGS_VERSION.to_string(),
                binary_path: super::managed_embeddings_binary_path().expect("managed runtime path"),
                freshly_installed: true,
            })
        },
        || {
            let config_path = repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
            super::managed::ensure_managed_embeddings_runtime(repo.path(), Some(&config_path))
                .expect("install managed runtime");
            let metadata_path = super::managed_embeddings_metadata_path().expect("metadata path");
            fs::create_dir_all(metadata_path.parent().expect("metadata parent"))
                .expect("create metadata parent");
            fs::write(&metadata_path, "{ not valid json").expect("write corrupt metadata");

            let capability = resolve_embedding_capability_config_for_repo(repo.path());
            let lines = doctor_profile(repo.path(), &capability, None).expect("doctor report");

            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Managed runtime metadata warning:")),
                "expected metadata warning, got: {lines:?}"
            );
        },
    );
}

#[test]
fn clear_cache_removes_local_cache_directory() {
    let repo = seed_repo();
    let capability = resolve_embedding_capability_config_for_repo(repo.path());
    let cache_dir = repo.path().join(".bitloops/embeddings/models");
    fs::create_dir_all(&cache_dir).expect("create cache dir");

    let lines =
        clear_cache_for_profile(repo.path(), &capability, "local").expect("clear cache report");

    assert!(!cache_dir.exists());
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Cleared cache for profile `local`"))
    );
}

#[test]
fn pull_uses_extended_timeouts_for_local_warmup() {
    let repo = seed_repo();
    let capability = resolve_embedding_capability_config_for_repo(repo.path());
    let runtime = pull_runtime_client_config(
        repo.path(),
        &repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH),
        &capability,
        "local",
    );

    assert_eq!(runtime.startup_timeout_secs, LOCAL_PULL_TIMEOUT_SECS);
    assert_eq!(runtime.request_timeout_secs, LOCAL_PULL_TIMEOUT_SECS);
}

#[test]
fn pull_rejects_hosted_profiles_without_network() {
    let repo = seed_repo();
    let capability = resolve_embedding_capability_config_for_repo(repo.path());
    let err = pull_profile(repo.path(), &capability, "openai").expect_err("hosted pull");
    assert!(
        err.to_string()
            .contains("not a `bitloops_embeddings_ipc` profile")
    );
}

#[test]
fn inspect_embeddings_install_state_reports_not_configured() {
    let repo = TempDir::new().expect("tempdir");
    let home = TempDir::new().expect("home dir");
    let home_path = home.path().to_string_lossy().to_string();
    let config_root = TempDir::new().expect("config tempdir");
    let config_root_value = config_root.path().to_string_lossy().into_owned();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[
            ("HOME", Some(home_path.as_str())),
            ("USERPROFILE", Some(home_path.as_str())),
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_root_value.as_str()),
            ),
        ],
    );
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Alice",
        "alice@example.com",
    );

    assert!(matches!(
        inspect_embeddings_install_state(repo.path()),
        EmbeddingsInstallState::NotConfigured
    ));
}

#[test]
fn install_or_bootstrap_embeddings_writes_local_profile_and_warms_runtime() {
    let repo = TempDir::new().expect("tempdir");
    let data = TempDir::new().expect("data tempdir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Alice",
        "alice@example.com",
    );
    write_runtime_only_config(
        repo.path(),
        "bitloops-local-embeddings",
        &[
            "-B".to_string(),
            "-m".to_string(),
            "bitloops_local_embeddings".to_string(),
        ],
    );
    let data_root = data.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[("BITLOOPS_TEST_DATA_DIR_OVERRIDE", Some(data_root.as_str()))],
    );

    with_managed_embeddings_install_hook(
        move |repo_root| {
            Ok(ManagedEmbeddingsBinaryInstallOutcome {
                version: TEST_MANAGED_EMBEDDINGS_VERSION.to_string(),
                binary_path: fake_managed_runtime_path(repo_root),
                freshly_installed: true,
            })
        },
        || {
            let lines = install_or_bootstrap_embeddings(repo.path())
                .expect("install embeddings via managed runtime");
            let config = fs::read_to_string(repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH))
                .expect("read updated config");
            let managed_command =
                raw_managed_runtime_command(&repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH))
                    .expect("read managed runtime command");

            assert!(config.contains("code_embeddings = \"local_code\""));
            assert!(config.contains("[inference.profiles.local_code]"));
            assert!(config.contains("driver = \"bitloops_embeddings_ipc\""));
            assert!(
                config.contains("args = []"),
                "expected managed runtime args to be reset:\n{config}"
            );
            assert!(
                !config.contains("\"-B\""),
                "expected stale python-style runtime args to be removed:\n{config}"
            );
            assert!(
                managed_command.as_deref()
                    == Some(
                        fake_managed_runtime_path(repo.path())
                            .to_string_lossy()
                            .as_ref()
                    ),
                "expected managed runtime path in config:\n{config}"
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Configured embeddings in")),
                "expected configuration line, got: {lines:?}"
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Installed managed standalone")),
                "expected managed install line, got: {lines:?}"
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Pulled embedding profile `local_code`")),
                "expected warmup line, got: {lines:?}"
            );
        },
    );
}

#[test]
fn install_or_bootstrap_embeddings_rolls_back_when_runtime_bootstrap_fails() {
    let repo = TempDir::new().expect("tempdir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Alice",
        "alice@example.com",
    );
    write_runtime_only_config(repo.path(), "definitely-missing-embeddings-runtime", &[]);
    let config_path = repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let original = fs::read_to_string(&config_path).expect("read original config");

    with_managed_embeddings_install_hook(
        |_repo_root| anyhow::bail!("simulated managed runtime install failure"),
        || {
            let err = install_or_bootstrap_embeddings(repo.path())
                .expect_err("runtime bootstrap should fail");
            let after = fs::read_to_string(&config_path).expect("read rolled-back config");

            assert_eq!(after, original);
            assert!(
                format!("{err:#}").contains("simulated managed runtime install failure"),
                "unexpected error: {err:#}"
            );
        },
    );
}

#[test]
fn install_or_bootstrap_embeddings_preserves_existing_hosted_profile() {
    let repo = TempDir::new().expect("tempdir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Alice",
        "alice@example.com",
    );
    let config_path = repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
    fs::write(
        &config_path,
        r#"
[runtime]
local_dev = false

[semantic_clones.inference]
code_embeddings = "openai"

[inference.profiles.openai]
task = "embeddings"
driver = "openai"
model = "text-embedding-3-large"
"#,
    )
    .expect("write hosted config");
    let original = fs::read_to_string(&config_path).expect("read original config");

    let lines =
        install_or_bootstrap_embeddings(repo.path()).expect("existing hosted profile result");
    let after = fs::read_to_string(&config_path).expect("read final config");

    assert_eq!(after, original);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("skipped local runtime bootstrap")),
        "expected hosted skip line, got: {lines:?}"
    );
}

#[test]
fn pull_installs_managed_runtime_for_default_local_runtime() {
    let repo = TempDir::new().expect("tempdir");
    let data = TempDir::new().expect("data tempdir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Alice",
        "alice@example.com",
    );
    write_embedding_config(repo.path());
    let data_root = data.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo.path()),
        &[("BITLOOPS_TEST_DATA_DIR_OVERRIDE", Some(data_root.as_str()))],
    );

    with_managed_embeddings_install_hook(
        move |repo_root| {
            Ok(ManagedEmbeddingsBinaryInstallOutcome {
                version: TEST_MANAGED_EMBEDDINGS_VERSION.to_string(),
                binary_path: fake_managed_runtime_path(repo_root),
                freshly_installed: true,
            })
        },
        || {
            let capability = resolve_embedding_capability_config_for_repo(repo.path());
            let lines = pull_profile(repo.path(), &capability, "local").expect("pull profile");
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("Installed managed standalone")),
                "expected managed install line, got: {lines:?}"
            );
        },
    );
}

#[test]
fn pull_does_not_install_managed_runtime_for_custom_runtime_command() {
    let repo = TempDir::new().expect("tempdir");
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Alice",
        "alice@example.com",
    );
    let (command, args) = fake_runtime_command_and_args(repo.path());
    write_runtime_only_config(repo.path(), &command, &args);
    let config_path = repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let mut config = fs::read_to_string(&config_path).expect("read config");
    config.push_str(
        r#"
[semantic_clones.inference]
code_embeddings = "local_code"
summary_embeddings = "local_code"

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "bge-m3"
"#,
    );
    fs::write(&config_path, config).expect("write config");
    let capability = resolve_embedding_capability_config_for_repo(repo.path());

    let lines = pull_profile(repo.path(), &capability, "local_code").expect("pull profile");

    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Installed managed standalone")),
        "custom runtime command should not trigger managed install: {lines:?}"
    );
}
