use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};

use crate::cli::embeddings::{
    PulledEmbeddingProfileOutcome, embedding_capability_for_config_path,
    ensure_managed_embeddings_runtime_with_progress, managed_runtime_command_is_eligible,
    managed_runtime_version_for_command, pull_profile_with_config_path_and_progress,
    selected_inference_profile_name,
};
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, DaemonEmbeddingsInstallMode, prepare_daemon_embeddings_install,
};
use crate::daemon::{
    EmbeddingsBootstrapGateEntry, EmbeddingsBootstrapGateStatus, EmbeddingsBootstrapPhase,
    EmbeddingsBootstrapProgress, EmbeddingsBootstrapReadiness, EmbeddingsBootstrapResult,
    EmbeddingsBootstrapTaskSpec, PersistedEmbeddingsBootstrapState, unix_timestamp_now,
};
use crate::host::inference::{BITLOOPS_EMBEDDINGS_IPC_DRIVER, BITLOOPS_EMBEDDINGS_RUNTIME_ID};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

pub(crate) fn execute_task_with_progress<R>(
    runtime_store: &DaemonSqliteRuntimeStore,
    repo_root: &Path,
    task_id: &str,
    spec: &EmbeddingsBootstrapTaskSpec,
    mut report: R,
) -> Result<EmbeddingsBootstrapResult>
where
    R: FnMut(EmbeddingsBootstrapProgress) -> Result<()>,
{
    let config_path = canonical_config_path(&spec.config_path);
    let config_key = config_path_key(&config_path);
    let lock = config_lock_for(&config_key);
    let _guard = lock
        .lock()
        .map_err(|_| anyhow!("embeddings bootstrap lock poisoned"))?;

    mark_gate_pending(
        runtime_store,
        &config_path,
        &spec.profile_name,
        Some(task_id.to_string()),
    )?;
    report(EmbeddingsBootstrapProgress {
        phase: EmbeddingsBootstrapPhase::PreparingConfig,
        message: Some(format!(
            "Preparing embeddings bootstrap for `{}`",
            spec.profile_name
        )),
        ..Default::default()
    })?;

    if let Some(result) = already_ready_result(runtime_store, &config_path, &spec.profile_name)? {
        mark_gate_ready(runtime_store, &config_path, &spec.profile_name)?;
        return Ok(result);
    }

    let execution =
        execute_bootstrap_flow(repo_root, &config_path, &spec.profile_name, &mut report);
    match execution {
        Ok(result) => {
            mark_gate_ready(runtime_store, &config_path, &spec.profile_name)?;
            Ok(result)
        }
        Err(err) => {
            mark_gate_failed(
                runtime_store,
                &config_path,
                &spec.profile_name,
                task_id,
                &err,
            )?;
            Err(err)
        }
    }
}

pub(crate) fn gate_status_for_config_path(
    runtime_store: &DaemonSqliteRuntimeStore,
    config_path: &Path,
) -> Result<EmbeddingsBootstrapGateStatus> {
    let config_path = canonical_config_path(config_path);
    let config_key = config_path_key(&config_path);
    let persisted_state = runtime_store
        .load_embeddings_bootstrap_state()?
        .unwrap_or_else(PersistedEmbeddingsBootstrapState::default);
    let persisted_entry = persisted_state.entries.get(&config_key);
    let last_updated_unix = persisted_entry
        .map(|entry| entry.last_updated_unix)
        .unwrap_or_default();
    let capability = match embedding_capability_for_config_path(&config_path) {
        Ok(capability) => capability,
        Err(err) => {
            return Ok(EmbeddingsBootstrapGateStatus {
                blocked: true,
                readiness: persisted_entry.map(|entry| entry.readiness),
                reason: Some(format!(
                    "Failed to resolve embeddings configuration: {err:#}"
                )),
                active_task_id: persisted_entry.and_then(|entry| entry.active_task_id.clone()),
                profile_name: persisted_entry.map(|entry| entry.profile_name.clone()),
                config_path: Some(config_path),
                last_error: persisted_entry.and_then(|entry| entry.last_error.clone()),
                last_updated_unix,
            });
        }
    };

    let Some(profile_name) = selected_inference_profile_name(&capability).map(str::to_string)
    else {
        return Ok(EmbeddingsBootstrapGateStatus {
            blocked: false,
            readiness: Some(EmbeddingsBootstrapReadiness::Ready),
            reason: Some("Embeddings are not configured for this daemon".to_string()),
            active_task_id: None,
            profile_name: None,
            config_path: Some(config_path),
            last_error: None,
            last_updated_unix,
        });
    };
    let Some(profile) = capability.inference.profiles.get(&profile_name) else {
        return Ok(EmbeddingsBootstrapGateStatus {
            blocked: true,
            readiness: persisted_entry.map(|entry| entry.readiness),
            reason: Some(format!(
                "Configured embeddings profile `{profile_name}` was not found"
            )),
            active_task_id: persisted_entry.and_then(|entry| entry.active_task_id.clone()),
            profile_name: Some(profile_name),
            config_path: Some(config_path),
            last_error: persisted_entry.and_then(|entry| entry.last_error.clone()),
            last_updated_unix,
        });
    };

    if profile.driver != BITLOOPS_EMBEDDINGS_IPC_DRIVER
        || profile.runtime.as_deref() != Some(BITLOOPS_EMBEDDINGS_RUNTIME_ID)
    {
        return Ok(EmbeddingsBootstrapGateStatus {
            blocked: false,
            readiness: Some(EmbeddingsBootstrapReadiness::Ready),
            reason: Some(
                "Embeddings bootstrap is not required for the configured profile".to_string(),
            ),
            active_task_id: None,
            profile_name: Some(profile_name),
            config_path: Some(config_path),
            last_error: None,
            last_updated_unix,
        });
    }

    if !managed_runtime_command_is_eligible(&config_path)? {
        return Ok(EmbeddingsBootstrapGateStatus {
            blocked: false,
            readiness: Some(EmbeddingsBootstrapReadiness::Ready),
            reason: Some("Embeddings runtime command is managed externally".to_string()),
            active_task_id: None,
            profile_name: Some(profile_name),
            config_path: Some(config_path),
            last_error: None,
            last_updated_unix,
        });
    }

    if let Some(entry) = persisted_entry
        && entry.readiness == EmbeddingsBootstrapReadiness::Pending
        && entry.active_task_id.is_some()
    {
        return Ok(EmbeddingsBootstrapGateStatus {
            blocked: true,
            readiness: Some(EmbeddingsBootstrapReadiness::Pending),
            reason: Some(format!(
                "Managed embeddings runtime is still bootstrapping via task `{}`",
                entry.active_task_id.as_deref().unwrap_or_default()
            )),
            active_task_id: entry.active_task_id.clone(),
            profile_name: Some(entry.profile_name.clone()),
            config_path: Some(config_path),
            last_error: entry.last_error.clone(),
            last_updated_unix,
        });
    }

    if let Some(runtime_name) = profile.runtime.as_deref()
        && let Some(runtime) = capability.inference.runtimes.get(runtime_name)
        && let Some(version) = managed_runtime_version_for_command(&runtime.command)?
    {
        return Ok(EmbeddingsBootstrapGateStatus {
            blocked: false,
            readiness: Some(EmbeddingsBootstrapReadiness::Ready),
            reason: Some(format!("Managed embeddings runtime {version} is ready")),
            active_task_id: None,
            profile_name: Some(profile_name),
            config_path: Some(config_path),
            last_error: None,
            last_updated_unix,
        });
    }

    let readiness = persisted_entry
        .map(|entry| entry.readiness)
        .unwrap_or(EmbeddingsBootstrapReadiness::Pending);
    let active_task_id = persisted_entry.and_then(|entry| entry.active_task_id.clone());
    let last_error = persisted_entry.and_then(|entry| entry.last_error.clone());
    let reason = match readiness {
        EmbeddingsBootstrapReadiness::Ready => {
            Some("Managed embeddings runtime is ready".to_string())
        }
        EmbeddingsBootstrapReadiness::Pending => Some(match active_task_id.as_deref() {
            Some(task_id) => {
                format!("Managed embeddings runtime is still bootstrapping via task `{task_id}`")
            }
            None => "Managed embeddings runtime is not ready yet".to_string(),
        }),
        EmbeddingsBootstrapReadiness::Failed => Some(
            last_error
                .clone()
                .unwrap_or_else(|| "Managed embeddings bootstrap previously failed".to_string()),
        ),
    };

    Ok(EmbeddingsBootstrapGateStatus {
        blocked: readiness != EmbeddingsBootstrapReadiness::Ready,
        readiness: Some(readiness),
        reason,
        active_task_id,
        profile_name: Some(profile_name),
        config_path: Some(config_path),
        last_error,
        last_updated_unix,
    })
}

pub(crate) fn gate_status_for_enrichment_queue(
    runtime_store: &DaemonSqliteRuntimeStore,
    config_roots: impl IntoIterator<Item = PathBuf>,
) -> Result<Option<EmbeddingsBootstrapGateStatus>> {
    let mut unique = BTreeMap::new();
    for config_root in config_roots {
        let config_path = canonical_config_path(&config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH));
        unique.insert(config_path_key(&config_path), config_path);
    }

    let Some((_, config_path)) = unique.into_iter().next() else {
        let state = runtime_store
            .load_embeddings_bootstrap_state()?
            .unwrap_or_else(PersistedEmbeddingsBootstrapState::default);
        return match state.entries.values().next() {
            Some(entry) => gate_status_for_config_path(runtime_store, &entry.config_path).map(Some),
            None => Ok(None),
        };
    };

    gate_status_for_config_path(runtime_store, &config_path).map(Some)
}

fn execute_bootstrap_flow<R>(
    repo_root: &Path,
    config_path: &Path,
    requested_profile_name: &str,
    report: &mut R,
) -> Result<EmbeddingsBootstrapResult>
where
    R: FnMut(EmbeddingsBootstrapProgress) -> Result<()>,
{
    if let Ok(capability) = embedding_capability_for_config_path(config_path)
        && capability
            .inference
            .profiles
            .get(requested_profile_name)
            .is_some_and(|profile| profile.driver == BITLOOPS_EMBEDDINGS_IPC_DRIVER)
    {
        return warm_existing_profile(
            repo_root,
            config_path,
            &capability,
            requested_profile_name,
            report,
            None,
        );
    }

    let install_plan = prepare_daemon_embeddings_install(config_path)?;
    let target_profile_name = install_plan.profile_name.clone();
    let result = (|| -> Result<EmbeddingsBootstrapResult> {
        match install_plan.mode {
            DaemonEmbeddingsInstallMode::SkipHosted => Ok(EmbeddingsBootstrapResult {
                version: None,
                binary_path: None,
                cache_dir: None,
                runtime_name: install_plan.profile_driver.clone(),
                model_name: Some(target_profile_name.clone()),
                freshly_installed: false,
                message: format!(
                    "Embeddings already use profile `{}`; no local bootstrap was required.",
                    target_profile_name
                ),
            }),
            DaemonEmbeddingsInstallMode::WarmExisting => {
                let capability = embedding_capability_for_config_path(config_path)?;
                warm_existing_profile(
                    repo_root,
                    config_path,
                    &capability,
                    &target_profile_name,
                    report,
                    Some(install_plan.mode),
                )
            }
            DaemonEmbeddingsInstallMode::Bootstrap => {
                let ensure =
                    ensure_managed_embeddings_runtime_with_progress(repo_root, None, &mut *report)?;
                report(EmbeddingsBootstrapProgress {
                    phase: EmbeddingsBootstrapPhase::RewritingRuntime,
                    version: Some(ensure.install.version.clone()),
                    message: Some(format!(
                        "Applying embeddings config in {}",
                        config_path.display()
                    )),
                    ..Default::default()
                })?;
                install_plan
                    .apply_with_managed_runtime_path(&ensure.install.binary_path)
                    .with_context(|| {
                        format!(
                            "applying staged embeddings config in {}",
                            config_path.display()
                        )
                    })?;
                let capability = embedding_capability_for_config_path(config_path)?;
                let pulled = pull_profile_with_config_path_and_progress(
                    repo_root,
                    config_path,
                    &capability,
                    &target_profile_name,
                    &mut *report,
                )?;
                Ok(EmbeddingsBootstrapResult {
                    version: Some(ensure.install.version.clone()),
                    binary_path: Some(ensure.install.binary_path.clone()),
                    cache_dir: Some(pulled.cache_dir),
                    runtime_name: Some(pulled.runtime_name),
                    model_name: Some(pulled.model_name),
                    freshly_installed: ensure.install.freshly_installed,
                    message: format!(
                        "Configured embeddings and warmed profile `{target_profile_name}`."
                    ),
                })
            }
        }
    })();

    match result {
        Ok(result) => Ok(result),
        Err(err) => {
            if install_plan.config_modified {
                install_plan.rollback()?;
            }
            Err(err)
        }
    }
}

fn warm_existing_profile<R>(
    repo_root: &Path,
    config_path: &Path,
    capability: &crate::config::EmbeddingCapabilityConfig,
    profile_name: &str,
    report: &mut R,
    install_mode: Option<DaemonEmbeddingsInstallMode>,
) -> Result<EmbeddingsBootstrapResult>
where
    R: FnMut(EmbeddingsBootstrapProgress) -> Result<()>,
{
    let install_needed = should_install_managed_runtime(capability, config_path, profile_name)?;
    let mut version = None;
    let mut binary_path = None;
    let mut freshly_installed = false;
    if install_needed {
        let ensure = ensure_managed_embeddings_runtime_with_progress(
            repo_root,
            Some(config_path),
            &mut *report,
        )?;
        version = Some(ensure.install.version.clone());
        binary_path = Some(ensure.install.binary_path.clone());
        freshly_installed = ensure.install.freshly_installed;
    }

    let pulled: PulledEmbeddingProfileOutcome = pull_profile_with_config_path_and_progress(
        repo_root,
        config_path,
        capability,
        profile_name,
        report,
    )?;

    let message = match install_mode {
        Some(DaemonEmbeddingsInstallMode::Bootstrap) => {
            format!("Configured embeddings and warmed profile `{profile_name}`.")
        }
        Some(DaemonEmbeddingsInstallMode::WarmExisting) => {
            format!("Warmed configured embeddings profile `{profile_name}`.")
        }
        _ => format!("Pulled embedding profile `{profile_name}`."),
    };

    Ok(EmbeddingsBootstrapResult {
        version,
        binary_path,
        cache_dir: Some(pulled.cache_dir),
        runtime_name: Some(pulled.runtime_name),
        model_name: Some(pulled.model_name),
        freshly_installed,
        message,
    })
}

fn should_install_managed_runtime(
    capability: &crate::config::EmbeddingCapabilityConfig,
    config_path: &Path,
    profile_name: &str,
) -> Result<bool> {
    let profile = capability
        .inference
        .profiles
        .get(profile_name)
        .ok_or_else(|| anyhow!("embedding profile `{profile_name}` was not found"))?;
    Ok(profile.driver == BITLOOPS_EMBEDDINGS_IPC_DRIVER
        && profile.runtime.as_deref() == Some(BITLOOPS_EMBEDDINGS_RUNTIME_ID)
        && managed_runtime_command_is_eligible(config_path)?)
}

fn already_ready_result(
    _runtime_store: &DaemonSqliteRuntimeStore,
    config_path: &Path,
    profile_name: &str,
) -> Result<Option<EmbeddingsBootstrapResult>> {
    let capability = match embedding_capability_for_config_path(config_path) {
        Ok(capability) => capability,
        Err(_) => return Ok(None),
    };
    if selected_inference_profile_name(&capability) != Some(profile_name) {
        return Ok(None);
    }
    let Some(profile) = capability.inference.profiles.get(profile_name) else {
        return Ok(None);
    };
    if profile.driver != BITLOOPS_EMBEDDINGS_IPC_DRIVER
        || profile.runtime.as_deref() != Some(BITLOOPS_EMBEDDINGS_RUNTIME_ID)
    {
        return Ok(Some(EmbeddingsBootstrapResult {
            version: None,
            binary_path: None,
            cache_dir: None,
            runtime_name: None,
            model_name: Some(profile_name.to_string()),
            freshly_installed: false,
            message: format!("Embeddings bootstrap already ready for profile `{profile_name}`."),
        }));
    }
    if !managed_runtime_command_is_eligible(config_path)? {
        return Ok(Some(EmbeddingsBootstrapResult {
            version: None,
            binary_path: None,
            cache_dir: None,
            runtime_name: None,
            model_name: Some(profile_name.to_string()),
            freshly_installed: false,
            message: format!("Embeddings bootstrap already ready for profile `{profile_name}`."),
        }));
    }
    let Some(runtime_name) = profile.runtime.as_deref() else {
        return Ok(None);
    };
    let Some(runtime) = capability.inference.runtimes.get(runtime_name) else {
        return Ok(None);
    };
    if managed_runtime_version_for_command(&runtime.command)?.is_none() {
        return Ok(None);
    }
    Ok(Some(EmbeddingsBootstrapResult {
        version: None,
        binary_path: None,
        cache_dir: None,
        runtime_name: None,
        model_name: Some(profile_name.to_string()),
        freshly_installed: false,
        message: format!("Embeddings bootstrap already ready for profile `{profile_name}`."),
    }))
}

fn mark_gate_pending(
    runtime_store: &DaemonSqliteRuntimeStore,
    config_path: &Path,
    profile_name: &str,
    active_task_id: Option<String>,
) -> Result<()> {
    mutate_gate_entry(runtime_store, config_path, profile_name, |entry| {
        entry.readiness = EmbeddingsBootstrapReadiness::Pending;
        entry.active_task_id = active_task_id;
        entry.last_error = None;
    })
}

fn mark_gate_ready(
    runtime_store: &DaemonSqliteRuntimeStore,
    config_path: &Path,
    profile_name: &str,
) -> Result<()> {
    mutate_gate_entry(runtime_store, config_path, profile_name, |entry| {
        entry.readiness = EmbeddingsBootstrapReadiness::Ready;
        entry.active_task_id = None;
        entry.last_error = None;
    })
}

fn mark_gate_failed(
    runtime_store: &DaemonSqliteRuntimeStore,
    config_path: &Path,
    profile_name: &str,
    task_id: &str,
    err: &anyhow::Error,
) -> Result<()> {
    mutate_gate_entry(runtime_store, config_path, profile_name, |entry| {
        entry.readiness = EmbeddingsBootstrapReadiness::Failed;
        entry.active_task_id = None;
        entry.last_error = Some(format!("task `{task_id}`: {err:#}"));
    })
}

fn mutate_gate_entry(
    runtime_store: &DaemonSqliteRuntimeStore,
    config_path: &Path,
    profile_name: &str,
    mutate: impl FnOnce(&mut EmbeddingsBootstrapGateEntry),
) -> Result<()> {
    let config_path = canonical_config_path(config_path);
    let config_key = config_path_key(&config_path);
    runtime_store.mutate_embeddings_bootstrap_state(|state| {
        let entry = state.entries.entry(config_key.clone()).or_insert_with(|| {
            EmbeddingsBootstrapGateEntry {
                config_path: config_path.clone(),
                profile_name: profile_name.to_string(),
                readiness: EmbeddingsBootstrapReadiness::Pending,
                active_task_id: None,
                last_error: None,
                last_updated_unix: 0,
            }
        });
        entry.config_path = config_path.clone();
        entry.profile_name = profile_name.to_string();
        mutate(entry);
        entry.last_updated_unix = unix_timestamp_now();
        state.last_action = Some("embeddings_bootstrap_gate_updated".to_string());
        state.updated_at_unix = entry.last_updated_unix;
        Ok(())
    })?;
    Ok(())
}

fn config_lock_for(config_key: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<BTreeMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        locks
            .entry(config_key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(()))),
    )
}

fn canonical_config_path(config_path: &Path) -> PathBuf {
    config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf())
}

fn config_path_key(config_path: &Path) -> String {
    canonical_config_path(config_path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrichment_queue_gate_status_recomputes_from_config_when_no_jobs_are_present() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let runtime_store = DaemonSqliteRuntimeStore::open_at(temp.path().join("runtime.sqlite"))
            .expect("open daemon runtime store");
        let config_path = temp.path().join("config.toml");

        std::fs::write(&config_path, "[runtime]\nlocal_dev = false\n")
            .expect("write minimal daemon config");

        runtime_store
            .mutate_embeddings_bootstrap_state(|state| {
                state.entries.insert(
                    config_path_key(&config_path),
                    EmbeddingsBootstrapGateEntry {
                        config_path: config_path.clone(),
                        profile_name: "local_code".to_string(),
                        readiness: EmbeddingsBootstrapReadiness::Pending,
                        active_task_id: Some("bootstrap-task-1".to_string()),
                        last_error: None,
                        last_updated_unix: unix_timestamp_now(),
                    },
                );
                Ok(())
            })
            .expect("persist stale bootstrap gate state");

        let status =
            gate_status_for_enrichment_queue(&runtime_store, std::iter::empty::<PathBuf>())
                .expect("load enrichment queue gate status")
                .expect("gate status should exist");

        assert!(!status.blocked);
        assert_eq!(status.readiness, Some(EmbeddingsBootstrapReadiness::Ready));
        assert_eq!(
            status.reason.as_deref(),
            Some("Embeddings are not configured for this daemon")
        );
    }
}
