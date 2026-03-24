pub(crate) use super::*;
pub(crate) use crate::test_support::process_state::{
    enter_process_state, with_cwd, with_process_state,
};
pub(crate) use std::fs;
pub(crate) use std::path::{Path, PathBuf};

mod backend;
mod blob;
mod dashboard;
mod embedding;
mod events;
mod knowledge_providers;
mod providerless;
mod semantic;
mod sqlite_path;
mod watch;

pub(crate) fn write_repo_config(repo_root: &Path, value: serde_json::Value) {
    let config_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let parent = config_path.parent().expect("config parent");
    fs::create_dir_all(parent).expect("create config dir");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&value).expect("serialize config"),
    )
    .expect("write config");
}

pub(crate) fn write_envelope_config(repo_root: &Path, settings: serde_json::Value) {
    let envelope = serde_json::json!({
        "version": "1.0",
        "scope": "project",
        "settings": settings
    });
    write_repo_config(repo_root, envelope);
}
