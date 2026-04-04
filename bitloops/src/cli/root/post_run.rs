use std::env;
use std::io;
use std::time::Duration;

use crate::cli::enable;
use crate::cli::versioncheck;

use super::build::build_version;

pub(crate) fn run_persistent_post_run(
    action: Option<&crate::telemetry::analytics::ActionDescriptor>,
    duration: Duration,
    success: bool,
) {
    let Some(action) = action else {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        versioncheck::check_and_notify(&mut out, build_version());
        return;
    };

    let repo_root = env::current_dir()
        .ok()
        .and_then(|cwd| enable::find_repo_root(&cwd).ok());

    let dispatch_context = repo_root
        .as_ref()
        .and_then(|repo_root| {
            crate::telemetry::analytics::load_dispatch_context_for_repo(repo_root)
        })
        .or_else(crate::telemetry::analytics::load_global_dispatch_context);

    if let Some(ctx) = dispatch_context {
        crate::telemetry::analytics::track_action_detached(
            Some(action),
            &ctx,
            build_version(),
            repo_root.as_deref(),
            success,
            duration.as_millis(),
        );
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    versioncheck::check_and_notify(&mut out, build_version());
}
