//! Root command helpers and metadata.

mod args;
mod build;
mod completion;
mod handlers;
mod help;
mod metadata;
mod post_run;
mod settings;
mod telemetry_actions;
mod version;

pub(crate) use args::*;
pub(crate) use build::{build_commit, build_date, build_target, build_version};
pub(crate) use completion::write_completion;
pub(crate) use handlers::{
    run_clean_command, run_completion_command, run_curl_bash_post_install_command,
    run_disable_command, run_doctor_command, run_help_command, run_reset_command,
    run_resume_command, run_root_default_help, run_send_analytics_command,
    run_uninstall_command, run_version_command,
};
pub(crate) use help::write_help;
pub(crate) use metadata::{ROOT_LONG_ABOUT, ROOT_NAME, ROOT_SHORT_ABOUT};
pub(crate) use post_run::run_persistent_post_run;
pub(crate) use settings::load_settings_once;
pub(crate) use telemetry_actions::{
    should_attempt_watcher_autostart, telemetry_action_for_command,
    telemetry_action_for_connection_status, telemetry_action_for_version,
};
pub(crate) use version::write_version;

#[cfg(test)]
pub(crate) use handlers::run_curl_bash_post_install_command_with_io;

#[cfg(test)]
pub(crate) use settings::has_hidden_in_chain;
