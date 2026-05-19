use anyhow::Result;

use crate::capability_packs::test_harness::mapping;
use crate::host::capability_host::{CurrentStateConsumerContext, CurrentStateConsumerRequest};

use super::persistence::{load_existing_test_artefact_identity_rows, replace_repo_state};

pub(super) async fn reconcile_full(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
) -> Result<()> {
    let production = context
        .relational
        .load_current_production_artefacts(&request.repo_id)?;
    let existing_test_artefacts =
        load_existing_test_artefact_identity_rows(&context.storage, &request.repo_id, None).await?;
    let mapping = mapping::execute_with_existing(
        &request.repo_id,
        &request.repo_root,
        request.head_commit_sha.as_deref().unwrap_or("current"),
        &production,
        context.language_services.as_ref(),
        &existing_test_artefacts,
    )?;

    replace_repo_state(
        &context.storage,
        &request.repo_id,
        &mapping.test_artefacts,
        &mapping.test_edges,
    )
    .await
}
