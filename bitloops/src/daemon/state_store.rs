use super::*;

pub(super) fn read_runtime_state(repo_root: &Path) -> Result<Option<DaemonRuntimeState>> {
    let _ = repo_root;
    let state = read_runtime_state_for_path(&runtime_state_path(repo_root))?;
    if let Some(state_ref) = state.as_ref()
        && process_is_running(state_ref.pid)?
    {
        return Ok(state);
    }
    if state.is_some() {
        let _ = delete_runtime_state();
    }
    Ok(None)
}

pub(super) fn read_runtime_state_for_path(path: &Path) -> Result<Option<DaemonRuntimeState>> {
    let _ = path;
    daemon_runtime_store()?.load_runtime_state()
}

pub(super) fn read_service_metadata(repo_root: &Path) -> Result<Option<DaemonServiceMetadata>> {
    read_service_metadata_for_path(&service_metadata_path(repo_root))
}

pub(super) fn read_service_metadata_for_path(path: &Path) -> Result<Option<DaemonServiceMetadata>> {
    let _ = path;
    daemon_runtime_store()?.load_service_metadata()
}

pub(super) fn read_supervisor_service_metadata() -> Result<Option<SupervisorServiceMetadata>> {
    daemon_runtime_store()?.load_supervisor_service_metadata()
}

pub(super) fn read_supervisor_runtime_state() -> Result<Option<SupervisorRuntimeState>> {
    let state = daemon_runtime_store()?.load_supervisor_runtime_state()?;
    if let Some(state_ref) = state.as_ref()
        && process_is_running(state_ref.pid)?
    {
        return Ok(state);
    }
    if state.is_some() {
        let _ = delete_supervisor_runtime_state();
    }
    Ok(None)
}

pub(super) fn write_runtime_state(path: &Path, state: &DaemonRuntimeState) -> Result<()> {
    let _ = path;
    daemon_runtime_store()?.save_runtime_state(state)
}

pub(super) fn write_service_metadata(path: &Path, state: &DaemonServiceMetadata) -> Result<()> {
    let _ = path;
    daemon_runtime_store()?.save_service_metadata(state)
}

pub(super) fn write_supervisor_runtime_state(state: &SupervisorRuntimeState) -> Result<()> {
    daemon_runtime_store()?.save_supervisor_runtime_state(state)
}

pub(super) fn write_supervisor_service_metadata(state: &SupervisorServiceMetadata) -> Result<()> {
    daemon_runtime_store()?.save_supervisor_service_metadata(state)
}

pub(super) fn delete_runtime_state() -> Result<()> {
    daemon_runtime_store()?.delete_runtime_state()
}

pub(super) fn delete_supervisor_runtime_state() -> Result<()> {
    daemon_runtime_store()?.delete_supervisor_runtime_state()
}

#[cfg(test)]
pub(super) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let value =
        serde_json::from_slice(&data).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(value))
}

#[cfg(test)]
pub(super) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("resolving daemon state parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating daemon state directory {}", parent.display()))?;
    let mut bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("serialising {}", path.display()))?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

#[cfg(test)]
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("resolving daemon state parent directory")?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp_path = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        std::process::id(),
        nanos
    ));

    {
        let mut file = fs::File::create(&temp_path).with_context(|| {
            format!(
                "creating temporary daemon state file {}",
                temp_path.display()
            )
        })?;
        std::io::Write::write_all(&mut file, bytes).with_context(|| {
            format!(
                "writing temporary daemon state file {}",
                temp_path.display()
            )
        })?;
        file.sync_all().with_context(|| {
            format!(
                "syncing temporary daemon state file {}",
                temp_path.display()
            )
        })?;
    }

    if let Err(err) = fs::rename(&temp_path, path) {
        #[cfg(windows)]
        {
            if path.exists() {
                fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
                fs::rename(&temp_path, path).with_context(|| {
                    format!("renaming {} to {}", temp_path.display(), path.display())
                })?;
                return Ok(());
            }
        }
        let _ = fs::remove_file(&temp_path);
        return Err(err)
            .with_context(|| format!("renaming {} to {}", temp_path.display(), path.display()));
    }

    Ok(())
}

fn daemon_runtime_store() -> Result<crate::host::runtime_store::DaemonSqliteRuntimeStore> {
    crate::host::runtime_store::DaemonSqliteRuntimeStore::open()
}

#[cfg(test)]
mod tests {
    use super::{read_json, write_json};
    use serde::{Deserialize, Serialize};
    use tempfile::tempdir;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct TestState {
        value: String,
    }

    #[test]
    fn write_json_replaces_existing_file_atomically() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("runtime.json");

        write_json(
            &path,
            &TestState {
                value: "first".to_string(),
            },
        )
        .expect("write initial state");
        write_json(
            &path,
            &TestState {
                value: "second".to_string(),
            },
        )
        .expect("replace state");

        let state = read_json::<TestState>(&path)
            .expect("read state")
            .expect("state must exist");
        assert_eq!(
            state,
            TestState {
                value: "second".to_string(),
            }
        );
    }
}
