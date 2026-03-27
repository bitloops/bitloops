use anyhow::{Result, bail};

use crate::capability_packs::knowledge::run_knowledge_versions_via_host;
use crate::host::devql::{
    CheckpointFileSnapshotBackfillOptions, DevqlConfig, resolve_repo_identity,
    run_capability_packs_report, run_checkpoint_file_snapshot_backfill, run_query,
};
use crate::utils::paths;

mod args;
mod graphql;
mod knowledge;

#[cfg(test)]
mod tests;

pub use crate::host::devql::run_connection_status;
pub use args::{
    DevqlArgs, DevqlCheckpointFileSnapshotsArgs, DevqlCommand, DevqlConnectionStatusArgs,
    DevqlIngestArgs, DevqlInitArgs, DevqlKnowledgeAddArgs, DevqlKnowledgeArgs,
    DevqlKnowledgeAssociateArgs, DevqlKnowledgeCommand, DevqlKnowledgeRefArgs, DevqlPacksArgs,
    DevqlProjectionArgs, DevqlProjectionCommand, DevqlQueryArgs,
};

pub(crate) const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops devql init`, `bitloops devql ingest`, `bitloops devql projection checkpoint-file-snapshots`, `bitloops devql query`, `bitloops devql connection-status`, `bitloops devql packs`, `bitloops devql knowledge add`, `bitloops devql knowledge associate`, `bitloops devql knowledge refresh`, `bitloops devql knowledge versions`";

pub async fn run(args: DevqlArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    if matches!(&command, DevqlCommand::ConnectionStatus(_)) {
        return run_connection_status().await;
    }

    let repo_root = paths::repo_root()?;
    let repo = resolve_repo_identity(&repo_root)?;

    if let DevqlCommand::Knowledge(args) = command {
        return match args.command {
            DevqlKnowledgeCommand::Add(add) => {
                knowledge::run_knowledge_add_via_graphql(
                    &repo_root,
                    &repo.identity,
                    &add.url,
                    add.commit.as_deref(),
                )
                .await
            }
            DevqlKnowledgeCommand::Associate(associate) => {
                knowledge::run_knowledge_associate_via_graphql(
                    &repo_root,
                    &associate.source_ref,
                    &associate.target_ref,
                )
                .await
            }
            DevqlKnowledgeCommand::Refresh(refresh) => {
                knowledge::run_knowledge_refresh_via_graphql(&repo_root, &refresh.knowledge_ref)
                    .await
            }
            DevqlKnowledgeCommand::Versions(versions) => {
                run_knowledge_versions_via_host(&repo_root, &repo, &versions.knowledge_ref).await
            }
        };
    }

    let cfg = DevqlConfig::from_env(repo_root, repo)?;

    match command {
        DevqlCommand::Init(_) => graphql::run_init_via_graphql(&cfg.repo_root).await,
        DevqlCommand::Ingest(args) => {
            graphql::run_ingest_via_graphql(&cfg.repo_root, args.init, args.max_checkpoints).await
        }
        DevqlCommand::Projection(args) => match args.command {
            DevqlProjectionCommand::CheckpointFileSnapshots(backfill) => {
                run_checkpoint_file_snapshot_backfill(
                    &cfg,
                    CheckpointFileSnapshotBackfillOptions {
                        batch_size: backfill.batch_size,
                        max_checkpoints: backfill.max_checkpoints,
                        resume_after: backfill.resume_after,
                        dry_run: backfill.dry_run,
                        emit_progress: true,
                    },
                )
                .await
            }
        },
        DevqlCommand::Query(args) => run_query(&cfg, &args.query, args.compact, args.graphql).await,
        DevqlCommand::Packs(args) => run_capability_packs_report(
            &cfg,
            args.json,
            args.apply_migrations,
            args.with_health,
            args.with_extensions,
        ),
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
        DevqlCommand::Knowledge(_) => unreachable!("handled before cfg setup"),
    }
}
