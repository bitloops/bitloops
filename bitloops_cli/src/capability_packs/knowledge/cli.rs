use std::path::Path;

use anyhow::Result;
use serde_json::json;

use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::devql::RepoIdentity;

pub async fn run_knowledge_add_via_host(
    repo_root: &Path,
    repo: &RepoIdentity,
    url: &str,
    commit: Option<&str>,
) -> Result<()> {
    let mut host = DevqlCapabilityHost::builtin(repo_root, repo.clone())?;
    let result = host
        .invoke_ingester(
            "knowledge",
            "knowledge.add",
            json!({ "url": url, "commit": commit }),
        )
        .await?;

    println!("{}", result.render_human());
    Ok(())
}

pub async fn run_knowledge_associate_via_host(
    repo_root: &Path,
    repo: &RepoIdentity,
    source_ref: &str,
    target_ref: &str,
) -> Result<()> {
    let mut host = DevqlCapabilityHost::builtin(repo_root, repo.clone())?;
    let result = host
        .invoke_ingester(
            "knowledge",
            "knowledge.associate",
            json!({ "source_ref": source_ref, "target_ref": target_ref }),
        )
        .await?;

    println!("{}", result.render_human());
    Ok(())
}

pub async fn run_knowledge_refresh_via_host(
    repo_root: &Path,
    repo: &RepoIdentity,
    knowledge_ref: &str,
) -> Result<()> {
    let mut host = DevqlCapabilityHost::builtin(repo_root, repo.clone())?;
    let result = host
        .invoke_ingester(
            "knowledge",
            "knowledge.refresh",
            json!({ "knowledge_ref": knowledge_ref }),
        )
        .await?;

    println!("{}", result.render_human());
    Ok(())
}

pub async fn run_knowledge_versions_via_host(
    repo_root: &Path,
    repo: &RepoIdentity,
    knowledge_ref: &str,
) -> Result<()> {
    let mut host = DevqlCapabilityHost::builtin(repo_root, repo.clone())?;
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
