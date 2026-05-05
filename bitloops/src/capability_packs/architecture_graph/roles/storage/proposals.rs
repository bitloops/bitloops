use anyhow::{Context, Result};

use crate::host::devql::{RelationalStorage, sql_json_value};

use super::rows::sql_text;
use crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureRoleChangeProposal;

pub async fn insert_change_proposal(
    relational: &RelationalStorage,
    proposal: &ArchitectureRoleChangeProposal,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO architecture_role_change_proposals (
            repo_id, proposal_id, proposal_kind, status, payload_json, impact_preview_json, provenance_json
         ) VALUES ({repo_id}, {proposal_id}, {proposal_kind}, {status}, {payload}, {impact_preview}, {provenance})
         ON CONFLICT(repo_id, proposal_id) DO UPDATE SET
            status = excluded.status,
            payload_json = excluded.payload_json,
            impact_preview_json = excluded.impact_preview_json,
            provenance_json = excluded.provenance_json;",
        repo_id = sql_text(&proposal.repo_id),
        proposal_id = sql_text(&proposal.proposal_id),
        proposal_kind = sql_text(&proposal.proposal_kind),
        status = sql_text(proposal.status.as_db()),
        payload = sql_json_value(relational, &proposal.payload),
        impact_preview = sql_json_value(relational, &proposal.impact_preview),
        provenance = sql_json_value(relational, &proposal.provenance),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("inserting architecture role change proposal")
}
