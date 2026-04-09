use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::Parser;
use tempfile::TempDir;

use super::targets::{
    ALL_TARGETS, UninstallTarget, collect_requested_targets, validate_scope_flags,
};
use super::{
    BinaryCandidatesFn, DaemonStopper, NO_FLAGS_ERROR, RunContext, ServiceUninstaller,
    UninstallArgs, UninstallSelector, run_with_context,
};
use crate::adapters::agents::claude_code::git_hooks;
use crate::adapters::agents::codex::hooks as codex_hooks;
use crate::cli::enable::SHELL_COMPLETION_COMMENT;
use crate::config::settings::SETTINGS_DIR;
use crate::devql_transport::{RepoPathRegistry, RepoPathRegistryEntry, persist_repo_path_registry};
use crate::test_support::process_state::{git_command, with_cwd, with_process_state};
use crate::utils::platform_dirs::{
    bitloops_cache_dir, bitloops_config_dir, bitloops_data_dir, bitloops_state_dir,
};

fn setup_git_repo(dir: &TempDir) {
    let status = git_command()
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "git init should succeed");
}

fn with_platform_dirs<T>(
    config: &TempDir,
    data: &TempDir,
    cache: &TempDir,
    state: &TempDir,
    home: &TempDir,
    cwd: Option<&Path>,
    f: impl FnOnce() -> T,
) -> T {
    let config_path = config.path().to_string_lossy().to_string();
    let data_path = data.path().to_string_lossy().to_string();
    let cache_path = cache.path().to_string_lossy().to_string();
    let state_path = state.path().to_string_lossy().to_string();
    let home_path = home.path().to_string_lossy().to_string();

    with_process_state(
        cwd,
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_path.as_str()),
            ),
            ("BITLOOPS_TEST_DATA_DIR_OVERRIDE", Some(data_path.as_str())),
            (
                "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
                Some(cache_path.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_path.as_str()),
            ),
            ("HOME", Some(home_path.as_str())),
        ],
        f,
    )
}

fn write_repo_registry(path: &Path, repo_roots: &[&Path]) {
    let entries = repo_roots
        .iter()
        .enumerate()
        .map(|(index, repo_root)| RepoPathRegistryEntry {
            repo_id: format!("repo-{index}"),
            provider: "github".to_string(),
            organisation: "bitloops".to_string(),
            name: format!("repo-{index}"),
            identity: format!("bitloops/repo-{index}"),
            repo_root: (*repo_root).to_path_buf(),
            last_branch: Some("main".to_string()),
            git_dir_relative_path: Some(".git".to_string()),
            updated_at_unix: 1,
        })
        .collect();

    persist_repo_path_registry(
        path,
        &RepoPathRegistry {
            version: 1,
            entries,
        },
    )
    .unwrap();
}

fn run_uninstall_for_test(
    args: UninstallArgs,
    cwd: Option<&Path>,
    select_fn: Option<&UninstallSelector>,
    daemon_stopper: &DaemonStopper,
    service_uninstaller: &ServiceUninstaller,
    binary_candidates: &BinaryCandidatesFn,
) -> Result<String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let context = RunContext {
        select_fn,
        daemon_stopper,
        service_uninstaller,
        binary_candidates,
    };

    let result = match cwd {
        Some(path) => with_cwd(path, || {
            runtime.block_on(run_with_context(args, &mut out, &mut err, context))
        }),
        None => runtime.block_on(run_with_context(args, &mut out, &mut err, context)),
    };
    result?;

    String::from_utf8(out).map_err(Into::into)
}

#[test]
fn uninstall_subcommand_parses_full_flag() {
    let parsed = crate::cli::Cli::try_parse_from(["bitloops", "uninstall", "--full"]).unwrap();
    let Some(crate::cli::Commands::Uninstall(args)) = parsed.command else {
        panic!("expected uninstall command");
    };
    assert!(args.full);
}

#[test]
fn disable_uninstall_flag_is_rejected() {
    assert!(crate::cli::Cli::try_parse_from(["bitloops", "disable", "--uninstall"]).is_err());
}

#[test]
fn no_flags_selector_maps_targets() {
    let mut out = Vec::new();
    let targets = collect_requested_targets(
        &UninstallArgs::default(),
        &mut out,
        Some(&|_| Ok(vec![UninstallTarget::Data, UninstallTarget::Shell])),
    )
    .unwrap()
    .unwrap();

    assert!(targets.contains(&UninstallTarget::Data));
    assert!(targets.contains(&UninstallTarget::Shell));
    assert_eq!(targets.len(), 2);
}

#[test]
fn no_flags_without_tty_errors() {
    let mut out = Vec::new();
    let err = collect_requested_targets(&UninstallArgs::default(), &mut out, None).unwrap_err();
    assert_eq!(err.to_string(), NO_FLAGS_ERROR);
}

#[test]
fn full_flag_collects_every_known_target() {
    let mut out = Vec::new();
    let targets = collect_requested_targets(
        &UninstallArgs {
            full: true,
            ..UninstallArgs::default()
        },
        &mut out,
        None,
    )
    .unwrap()
    .unwrap();

    assert_eq!(targets, BTreeSet::from(ALL_TARGETS));
}

#[test]
fn explicit_flags_collect_requested_targets_without_prompting() {
    let mut out = Vec::new();
    let targets = collect_requested_targets(
        &UninstallArgs {
            data: true,
            config: true,
            git_hooks: true,
            ..UninstallArgs::default()
        },
        &mut out,
        None,
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        targets,
        BTreeSet::from([
            UninstallTarget::Data,
            UninstallTarget::Config,
            UninstallTarget::GitHooks,
        ])
    );
}

#[test]
fn only_current_project_requires_hook_targets() {
    let targets = BTreeSet::from([UninstallTarget::Data]);
    let err = validate_scope_flags(
        &UninstallArgs {
            only_current_project: true,
            ..UninstallArgs::default()
        },
        &targets,
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("--only-current-project"));
}

#[test]
fn only_current_project_accepts_hook_only_targets() {
    let targets = BTreeSet::from([UninstallTarget::AgentHooks, UninstallTarget::GitHooks]);
    validate_scope_flags(
        &UninstallArgs {
            only_current_project: true,
            ..UninstallArgs::default()
        },
        &targets,
    )
    .expect("hook-only targets should be valid");
}

#[test]
fn uninstall_git_hooks_uses_all_known_repos_by_default() {
    let repo_one = tempfile::tempdir().unwrap();
    let repo_two = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    setup_git_repo(&repo_one);
    setup_git_repo(&repo_two);
    git_hooks::install_git_hooks(repo_one.path(), false).unwrap();
    git_hooks::install_git_hooks(repo_two.path(), false).unwrap();

    with_platform_dirs(&config, &data, &cache, &state, &home, None, || {
        let registry_path = bitloops_state_dir()
            .unwrap()
            .join("daemon")
            .join("repo-path-registry.json");
        write_repo_registry(&registry_path, &[repo_one.path(), repo_two.path()]);

        run_uninstall_for_test(
            UninstallArgs {
                git_hooks: true,
                force: true,
                ..UninstallArgs::default()
            },
            None,
            None,
            &|| Box::pin(async { Ok(()) }),
            &|| Ok(()),
            &|| Ok(Vec::new()),
        )
        .unwrap();
    });

    assert!(!git_hooks::is_git_hook_installed(repo_one.path()));
    assert!(!git_hooks::is_git_hook_installed(repo_two.path()));
}

#[test]
fn only_current_project_limits_hook_removal() {
    let repo_one = tempfile::tempdir().unwrap();
    let repo_two = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    setup_git_repo(&repo_one);
    setup_git_repo(&repo_two);
    git_hooks::install_git_hooks(repo_one.path(), false).unwrap();
    git_hooks::install_git_hooks(repo_two.path(), false).unwrap();

    with_platform_dirs(
        &config,
        &data,
        &cache,
        &state,
        &home,
        Some(repo_one.path()),
        || {
            let registry_path = bitloops_state_dir()
                .unwrap()
                .join("daemon")
                .join("repo-path-registry.json");
            write_repo_registry(&registry_path, &[repo_one.path(), repo_two.path()]);

            run_uninstall_for_test(
                UninstallArgs {
                    git_hooks: true,
                    only_current_project: true,
                    force: true,
                    ..UninstallArgs::default()
                },
                None,
                None,
                &|| Box::pin(async { Ok(()) }),
                &|| Ok(()),
                &|| Ok(Vec::new()),
            )
            .unwrap();
        },
    );

    assert!(!git_hooks::is_git_hook_installed(repo_one.path()));
    assert!(git_hooks::is_git_hook_installed(repo_two.path()));
}

#[test]
fn data_target_removes_only_data() {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    with_platform_dirs(&config, &data, &cache, &state, &home, None, || {
        fs::create_dir_all(bitloops_data_dir().unwrap()).unwrap();
        fs::create_dir_all(bitloops_cache_dir().unwrap()).unwrap();
        fs::create_dir_all(bitloops_config_dir().unwrap()).unwrap();
        fs::create_dir_all(bitloops_state_dir().unwrap()).unwrap();

        run_uninstall_for_test(
            UninstallArgs {
                data: true,
                force: true,
                ..UninstallArgs::default()
            },
            None,
            None,
            &|| Box::pin(async { Ok(()) }),
            &|| Ok(()),
            &|| Ok(Vec::new()),
        )
        .unwrap();

        assert!(!bitloops_data_dir().unwrap().exists());
        assert!(bitloops_cache_dir().unwrap().exists());
        assert!(bitloops_config_dir().unwrap().exists());
        assert!(bitloops_state_dir().unwrap().exists());
    });
}

#[test]
fn full_uninstall_removes_supported_temp_artefacts() {
    let repo = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let binary_dir = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_platform_dirs(
        &config,
        &data,
        &cache,
        &state,
        &home,
        Some(repo.path()),
        || {
            fs::create_dir_all(repo.path().join(SETTINGS_DIR)).unwrap();
            fs::create_dir_all(bitloops_config_dir().unwrap()).unwrap();
            fs::create_dir_all(bitloops_data_dir().unwrap()).unwrap();
            fs::create_dir_all(bitloops_cache_dir().unwrap()).unwrap();
            fs::create_dir_all(bitloops_state_dir().unwrap()).unwrap();
            fs::create_dir_all(home.path().join(".bitloops").join("certs")).unwrap();
            fs::write(
                home.path().join(".zshrc"),
                format!("{SHELL_COMPLETION_COMMENT}\nsource <(bitloops completion zsh)\n"),
            )
            .unwrap();
            codex_hooks::install_hooks_at(repo.path(), false, false).unwrap();
            assert!(repo.path().join(".codex/config.toml").exists());
            git_hooks::install_git_hooks(repo.path(), false).unwrap();

            let registry_path = bitloops_state_dir()
                .unwrap()
                .join("daemon")
                .join("repo-path-registry.json");
            write_repo_registry(&registry_path, &[repo.path()]);

            let binary_path = binary_dir.path().join("bitloops");
            let binary_path_for_closure = binary_path.clone();
            fs::write(&binary_path, "binary").unwrap();

            let service_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let service_called_ref = service_called.clone();

            run_uninstall_for_test(
                UninstallArgs {
                    full: true,
                    force: true,
                    ..UninstallArgs::default()
                },
                None,
                None,
                &|| Box::pin(async { Ok(()) }),
                &move || {
                    service_called_ref.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                },
                &move || Ok(vec![binary_path_for_closure.clone()]),
            )
            .unwrap();

            assert!(service_called.load(std::sync::atomic::Ordering::SeqCst));
            assert!(!codex_hooks::are_hooks_installed_at(repo.path()));
            assert!(repo.path().join(".codex/config.toml").exists());
            assert!(!git_hooks::is_git_hook_installed(repo.path()));
            assert!(!repo.path().join(SETTINGS_DIR).exists());
            assert!(!bitloops_config_dir().unwrap().exists());
            assert!(!bitloops_data_dir().unwrap().exists());
            assert!(!bitloops_cache_dir().unwrap().exists());
            assert!(!bitloops_state_dir().unwrap().exists());
            assert!(!home.path().join(".bitloops").join("certs").exists());
            assert!(!home.path().join(".zshrc").exists());
            assert!(!binary_path.exists());
        },
    );
}

#[test]
fn service_uninstall_stops_daemon_best_effort_then_runs_service_uninstaller() {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();

    with_platform_dirs(&config, &data, &cache, &state, &home, None, || {
        let events = Arc::new(Mutex::new(Vec::new()));
        let stop_events = events.clone();
        let service_events = events.clone();

        run_uninstall_for_test(
            UninstallArgs {
                service: true,
                force: true,
                ..UninstallArgs::default()
            },
            None,
            None,
            &move || {
                let stop_events = stop_events.clone();
                Box::pin(async move {
                    stop_events.lock().unwrap().push("stop");
                    Ok(())
                })
            },
            &move || {
                service_events.lock().unwrap().push("service");
                Ok(())
            },
            &|| Ok(Vec::new()),
        )
        .unwrap();

        let recorded = events.lock().unwrap().clone();
        assert_eq!(recorded, vec!["stop", "service"]);
    });
}
