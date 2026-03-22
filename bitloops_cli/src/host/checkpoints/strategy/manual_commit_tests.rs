#![allow(unused_imports)]

use super::*;

#[path = "manual_commit_tests/checkpoint_core.rs"]
mod checkpoint_core;
#[path = "manual_commit_tests/checkpoint_redaction.rs"]
mod checkpoint_redaction;
#[path = "manual_commit_tests/checkpoint_temporary.rs"]
mod checkpoint_temporary;
#[path = "manual_commit_tests/checkpoint_views.rs"]
mod checkpoint_views;
#[path = "manual_commit_tests/commit_hooks.rs"]
mod commit_hooks;
#[path = "manual_commit_tests/common.rs"]
mod common;
#[path = "manual_commit_tests/git_sequence.rs"]
mod git_sequence;
#[path = "manual_commit_tests/post_commit.rs"]
mod post_commit;
#[path = "manual_commit_tests/session_state.rs"]
mod session_state;
#[path = "manual_commit_tests/shadow_branch.rs"]
mod shadow_branch;
#[path = "manual_commit_tests/update_committed.rs"]
mod update_committed;

pub(crate) use self::checkpoint_core::*;
pub(crate) use self::checkpoint_redaction::*;
pub(crate) use self::checkpoint_temporary::*;
pub(crate) use self::checkpoint_views::*;
pub(crate) use self::commit_hooks::*;
pub(crate) use self::common::*;
pub(crate) use self::git_sequence::*;
pub(crate) use self::post_commit::*;
pub(crate) use self::session_state::*;
pub(crate) use self::shadow_branch::*;
pub(crate) use self::update_committed::*;
