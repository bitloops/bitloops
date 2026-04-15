use super::*;

pub(super) fn active_branch_name(repo_root: &Path) -> String {
    checked_out_branch_name(repo_root).unwrap_or_else(|| "main".to_string())
}

pub(super) fn resolve_pack_versions_for_ingest() -> Result<(String, String)> {
    let host = core_extension_host()?;
    let mut packs = host
        .language_packs()
        .registered_pack_ids()
        .into_iter()
        .filter_map(|pack_id| host.language_packs().resolve_pack(pack_id))
        .map(|descriptor| format!("{}@{}", descriptor.id, descriptor.version))
        .collect::<Vec<_>>();
    packs.sort();
    let joined = packs.join("+");
    Ok((
        format!("devql-sync-parser@{joined}"),
        format!("devql-sync-extractor@{joined}"),
    ))
}

pub(super) fn tracked_paths_at_revision(repo_root: &Path, revision: &str) -> Result<Vec<String>> {
    let output = run_git(
        repo_root,
        &["ls-tree", "-r", "--full-tree", "--name-only", revision],
    )
    .with_context(|| format!("listing tracked files at revision `{revision}`"))?;
    Ok(output
        .lines()
        .map(normalize_repo_path)
        .filter(|path| !path.is_empty())
        .collect())
}

pub(super) async fn promote_temporary_current_rows_for_head_commit(
    _cfg: &DevqlConfig,
    _relational: &RelationalStorage,
) -> Result<usize> {
    // Current-state ingestion now writes directly into the sync-shaped tables, so the legacy
    // temporary-row promotion step is intentionally a no-op until a concrete replacement exists.
    Ok(0)
}
