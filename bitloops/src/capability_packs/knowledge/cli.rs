use anyhow::Result;
use serde_json::json;
use std::path::Path;

use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::devql::RepoIdentity;

pub async fn run_knowledge_versions_via_host(
    repo_root: &Path,
    repo: &RepoIdentity,
    knowledge_ref: &str,
) -> Result<()> {
    let host = DevqlCapabilityHost::builtin(repo_root, repo.clone())?;
    let result = host
        .invoke_ingester(
            "knowledge",
            "knowledge.versions",
            json!({ "knowledge_ref": knowledge_ref }),
        )
        .await?;

    println!("{}", result.render_human());
    Ok(())
}
