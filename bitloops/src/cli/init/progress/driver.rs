use std::io::Write;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::devql_transport::SlimCliRepoScope;

use super::renderer::RuntimeInitRenderer;
use super::viewport::fit_line;

const INIT_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct InitProgressOptions {
    pub(crate) start_input: crate::cli::devql::graphql::RuntimeStartInitInput,
    pub(crate) show_live_progress_notice: bool,
}

pub(crate) async fn run_dual_init_progress(
    out: &mut dyn Write,
    scope: &SlimCliRepoScope,
    options: InitProgressOptions,
) -> Result<()> {
    let start =
        crate::cli::devql::graphql::start_init_via_runtime_graphql(scope, &options.start_input)
            .await?;
    let repo_id = options.start_input.repo_id.clone();
    let session_id = start.init_session_id;
    let mut renderer = RuntimeInitRenderer::new();
    let mut polling_only = false;

    if options.show_live_progress_notice {
        writeln!(out)?;
        writeln!(
            out,
            "──────────────────────────────────────────────────────────────────"
        )?;
        writeln!(out, "                   🔍 Live Progress")?;
        writeln!(
            out,
            " Feel free to close this terminal and continue with your day! 🌟"
        )?;
        writeln!(
            out,
            "──────────────────────────────────────────────────────────────────"
        )?;
        writeln!(out)?;
    }

    writeln!(
        out,
        "{}",
        fit_line(
            "This may take a few minutes depending on your codebase size.",
            renderer.terminal_width,
        )
    )?;
    writeln!(out)?;
    out.flush()?;

    loop {
        let snapshot = crate::cli::devql::graphql::runtime_snapshot_via_graphql(scope, &repo_id)
            .await
            .with_context(|| format!("loading runtime snapshot for repo `{repo_id}`"))?;
        renderer.render(out, &snapshot, session_id.as_str())?;

        if let Some(session) = snapshot.current_init_session.as_ref()
            && session.init_session_id == session_id
        {
            match session.status.to_ascii_lowercase().as_str() {
                "completed" | "completed_with_warnings" => {
                    renderer.finish(out)?;
                    return Ok(());
                }
                "failed" => {
                    renderer.finish(out)?;
                    bail!(
                        "{}",
                        session
                            .terminal_error
                            .clone()
                            .unwrap_or_else(|| "init session failed".to_string())
                    );
                }
                _ => {}
            }
        }

        renderer.advance_spinner();
        if polling_only {
            tokio::time::sleep(INIT_PROGRESS_POLL_INTERVAL).await;
            continue;
        }

        match tokio::time::timeout(
            INIT_PROGRESS_POLL_INTERVAL,
            crate::cli::devql::graphql::next_runtime_event_via_subscription(
                scope,
                repo_id.as_str(),
                Some(session_id.as_str()),
            ),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                log::debug!("runtime subscription unavailable; falling back to polling: {err:#}");
                polling_only = true;
            }
            Err(_) => {}
        }
    }
}
