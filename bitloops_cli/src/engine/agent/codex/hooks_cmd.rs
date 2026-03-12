use std::path::Path;

use anyhow::Result;

use crate::engine::hooks::runtime::agent_runtime::{
    CODEX_HOOK_AGENT_PROFILE, SessionInfoInput, handle_session_start, handle_stop_with_profile,
};
use crate::engine::session::backend::SessionBackend;
use crate::engine::strategy::Strategy;

pub fn handle_session_start_codex(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_session_start(input, backend, repo_root)
}

pub fn handle_stop_codex(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_stop_with_profile(
        input,
        backend,
        strategy,
        repo_root,
        CODEX_HOOK_AGENT_PROFILE,
    )
}
