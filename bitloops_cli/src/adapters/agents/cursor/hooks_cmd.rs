use std::path::Path;

use anyhow::Result;

use crate::host::hooks::runtime::agent_runtime::{
    CURSOR_HOOK_AGENT_PROFILE, SessionInfoInput, UserPromptSubmitInput, handle_session_end,
    handle_session_start, handle_stop_with_profile,
    handle_user_prompt_submit_with_strategy_and_profile,
};
use crate::host::session::backend::SessionBackend;
use crate::host::strategy::Strategy;

pub fn handle_session_start_cursor(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_session_start(input, backend, repo_root)
}

pub fn handle_before_submit_prompt_cursor(
    input: UserPromptSubmitInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_user_prompt_submit_with_strategy_and_profile(
        input,
        backend,
        strategy,
        repo_root,
        CURSOR_HOOK_AGENT_PROFILE,
    )
}

pub fn handle_stop_cursor(
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
        CURSOR_HOOK_AGENT_PROFILE,
    )
}

pub fn handle_session_end_cursor(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
) -> Result<()> {
    handle_session_end(input, backend)
}
