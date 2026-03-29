use super::*;

pub(super) fn read_runtime_state(repo_root: &Path) -> Result<Option<DaemonRuntimeState>> {
    let path = runtime_state_path(repo_root);
    let state = read_runtime_state_for_path(&path)?;
    if let Some(state) = state
        && process_is_running(state.pid)?
    {
        return Ok(Some(state));
    }
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    Ok(None)
}

pub(super) fn read_runtime_state_for_path(path: &Path) -> Result<Option<DaemonRuntimeState>> {
    read_json(path)
}

pub(super) fn read_service_metadata(repo_root: &Path) -> Result<Option<DaemonServiceMetadata>> {
    read_service_metadata_for_path(&service_metadata_path(repo_root))
}

pub(super) fn read_service_metadata_for_path(path: &Path) -> Result<Option<DaemonServiceMetadata>> {
    read_json(path)
}

pub(super) fn read_supervisor_service_metadata() -> Result<Option<SupervisorServiceMetadata>> {
    read_json(&supervisor_service_metadata_path()?)
}

pub(super) fn read_supervisor_runtime_state() -> Result<Option<SupervisorRuntimeState>> {
    let path = supervisor_runtime_state_path()?;
    let state = read_json::<SupervisorRuntimeState>(&path)?;
    if let Some(state) = state
        && process_is_running(state.pid)?
    {
        return Ok(Some(state));
    }
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    Ok(None)
}

pub(super) fn write_runtime_state(path: &Path, state: &DaemonRuntimeState) -> Result<()> {
    write_json(path, state)
}

pub(super) fn write_service_metadata(path: &Path, state: &DaemonServiceMetadata) -> Result<()> {
    write_json(path, state)
}

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
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}
