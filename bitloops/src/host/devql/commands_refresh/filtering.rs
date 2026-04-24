use anyhow::{Context, Result};

use crate::host::devql::{
    AnalysisMode, DevqlConfig, ProjectAwareClassifier, load_repo_exclusion_matcher,
};

use super::super::core_extension_host;

pub(super) fn filter_refresh_paths_for_sync(
    cfg: &DevqlConfig,
    paths: &[String],
    source_hook: &str,
) -> Result<Vec<String>> {
    let exclusion_matcher = load_repo_exclusion_matcher(&cfg.repo_root).with_context(|| {
        format!("loading repo policy exclusions for {source_hook} path refresh")
    })?;
    let (parser_version, extractor_version) =
        resolve_pack_versions_for_refresh().with_context(|| {
            format!("resolving language pack versions for {source_hook} path refresh")
        })?;
    let classifier = ProjectAwareClassifier::discover_for_worktree(
        &cfg.repo_root,
        paths.iter().map(String::as_str),
        &parser_version,
        &extractor_version,
    )
    .with_context(|| format!("building classifier for {source_hook} path refresh"))?;
    let mut filtered = Vec::new();
    for path in paths {
        let classification = classifier
            .classify_repo_relative_path(path, exclusion_matcher.excludes_repo_relative_path(path))
            .with_context(|| format!("classifying {source_hook} refresh path `{path}`"))?;
        if classification.analysis_mode != AnalysisMode::Excluded {
            filtered.push(path.clone());
        }
    }
    Ok(filtered)
}

fn resolve_pack_versions_for_refresh() -> Result<(String, String)> {
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
