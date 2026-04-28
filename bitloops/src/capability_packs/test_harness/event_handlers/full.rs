use anyhow::Result;

use crate::capability_packs::test_harness::mapping;
use crate::host::capability_host::{CurrentStateConsumerContext, CurrentStateConsumerRequest};

use super::persistence::replace_repo_state;

pub(super) async fn reconcile_full(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
) -> Result<()> {
    let production = context
        .relational
        .load_current_production_artefacts(&request.repo_id)?;
    let mapping = mapping::execute(
        &request.repo_id,
        &request.repo_root,
        request.head_commit_sha.as_deref().unwrap_or("current"),
        &production,
        context.language_services.as_ref(),
    )?;

    replace_repo_state(
        &context.storage,
        &request.repo_id,
        &mapping.test_artefacts,
        &mapping.test_edges,
    )
    .await
}
