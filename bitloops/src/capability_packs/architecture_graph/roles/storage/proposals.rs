use anyhow::{Context, Result};

use crate::host::devql::{RelationalStorage, sql_json_value, sql_now};

use super::rows::sql_text;
use crate::capability_packs::architecture_graph::roles::taxonomy::ArchitectureRoleChangeProposal;

pub async fn insert_change_proposal(
    relational: &RelationalStorage,
    proposal: &ArchitectureRoleChangeProposal,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO architecture_role_change_proposals (
            repo_id, proposal_id, proposal_type, status,
            request_payload_json, preview_payload_json, result_payload_json, provenance_json
         ) VALUES ({repo_id}, {proposal_id}, {proposal_type}, {status}, {payload}, {impact_preview}, {result_payload}, {provenance})
         ON CONFLICT(repo_id, proposal_id) DO UPDATE SET
            proposal_type = excluded.proposal_type,
            status = excluded.status,
            request_payload_json = excluded.request_payload_json,
            preview_payload_json = excluded.preview_payload_json,
            result_payload_json = excluded.result_payload_json,
            provenance_json = excluded.provenance_json,
            updated_at = {now};",
        repo_id = sql_text(&proposal.repo_id),
        proposal_id = sql_text(&proposal.proposal_id),
        proposal_type = sql_text(&proposal.proposal_kind),
        status = sql_text(proposal.status.as_db()),
        payload = sql_json_value(relational, &proposal.payload),
        impact_preview = sql_json_value(relational, &proposal.impact_preview),
        result_payload = sql_json_value(relational, &serde_json::json!({})),
        provenance = sql_json_value(relational, &proposal.provenance),
        now = sql_now(relational),
    );
    relational
        .exec_serialized(&sql)
        .await
        .context("inserting architecture role change proposal")
}
