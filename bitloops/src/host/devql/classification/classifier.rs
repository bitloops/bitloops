use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use crate::config::discover_repo_policy_optional;
use crate::host::devql::{
    PLAIN_TEXT_LANGUAGE_ID, core_extension_host, resolve_language_id_for_file_path,
};
use crate::host::extension_host::LanguagePackResolutionInput;

use super::context::{
    AutoScopePolicy, ResolvedContext, detect_auto_contexts, parse_manual_context_specs,
    parse_scope_string_list,
};
use super::path_rules::{
    compute_sha256_json, is_auto_text_path, lower_extension, normalise_candidate_paths,
};
use super::patterns::PathPatternMatcher;
use super::repo_view::{FsRepoContentView, RepoContentView, RevisionRepoContentView};
use super::types::{
    AnalysisMode, FileRole, ProjectContext, ResolvedFileClassification, TextIndexMode,
};

pub(crate) const TRACK_ONLY_LANGUAGE_ID: &str = "track_only";

#[derive(Debug, Clone)]
pub(crate) struct ProjectAwareClassifier {
    parser_version: String,
    extractor_version: String,
    auto_scope: AutoScopePolicy,
    include_as_text: Option<PathPatternMatcher>,
    contexts: Vec<ResolvedContext>,
}

struct ClassificationInput<'a> {
    analysis_mode: AnalysisMode,
    file_role: FileRole,
    text_index_mode: TextIndexMode,
    compatibility_language: &'a str,
    dialect: Option<String>,
    contexts: &'a [&'a ResolvedContext],
    primary: Option<&'a ResolvedContext>,
    classification_reason: String,
    excluded_by_policy: bool,
}

impl ProjectAwareClassifier {
    pub(crate) fn discover_for_worktree<I, S>(
        repo_root: &Path,
        candidate_paths: I,
        parser_version: &str,
        extractor_version: &str,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let candidate_paths = normalise_candidate_paths(candidate_paths);
        let view = FsRepoContentView::new(repo_root.to_path_buf());
        Self::discover_with_view(
            repo_root,
            &view,
            &candidate_paths,
            parser_version,
            extractor_version,
        )
    }

    pub(crate) fn discover_for_revision<I, S>(
        repo_root: &Path,
        revision: &str,
        candidate_paths: I,
        parser_version: &str,
        extractor_version: &str,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let candidate_paths = normalise_candidate_paths(candidate_paths);
        let view = RevisionRepoContentView::new(
            repo_root.to_path_buf(),
            revision.to_string(),
            &candidate_paths,
        );
        Self::discover_with_view(
            repo_root,
            &view,
            &candidate_paths,
            parser_version,
            extractor_version,
        )
    }

    pub(crate) fn classify_repo_relative_path(
        &self,
        path: &str,
        excluded_by_policy: bool,
    ) -> Result<ResolvedFileClassification> {
        let path = crate::host::devql::normalize_repo_path(path);
        if path.is_empty() {
            return Ok(self.build_classification(ClassificationInput {
                analysis_mode: AnalysisMode::TrackOnly,
                file_role: FileRole::Other,
                text_index_mode: TextIndexMode::None,
                compatibility_language: TRACK_ONLY_LANGUAGE_ID,
                dialect: None,
                contexts: &[],
                primary: None,
                classification_reason: "empty_path".to_string(),
                excluded_by_policy: false,
            }));
        }

        if excluded_by_policy {
            return Ok(self.build_classification(ClassificationInput {
                analysis_mode: AnalysisMode::Excluded,
                file_role: FileRole::for_track_only_path(&path, "excluded_by_policy"),
                text_index_mode: TextIndexMode::None,
                compatibility_language: TRACK_ONLY_LANGUAGE_ID,
                dialect: None,
                contexts: &[],
                primary: None,
                classification_reason: "excluded_by_policy".to_string(),
                excluded_by_policy: true,
            }));
        }

        let manual_code_matches = self.matching_code_contexts(&path, true);
        if !manual_code_matches.is_empty() {
            let primary =
                choose_primary_context(&manual_code_matches).expect("non-empty manual matches");
            return Ok(self.build_classification(ClassificationInput {
                analysis_mode: AnalysisMode::Code,
                file_role: FileRole::SourceCode,
                text_index_mode: TextIndexMode::None,
                compatibility_language: &resolved_language_for_context(&path, primary)?,
                dialect: resolved_dialect_for_path(&path),
                contexts: &manual_code_matches,
                primary: Some(primary),
                classification_reason: format!("manual_context:{}", primary.context.context_id),
                excluded_by_policy: false,
            }));
        }

        if self
            .include_as_text
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(&path))
        {
            let affiliations = self.matching_affiliated_contexts(&path);
            let primary = choose_primary_context(&affiliations);
            let file_role = FileRole::for_text_path(&path);
            return Ok(self.build_classification(ClassificationInput {
                analysis_mode: AnalysisMode::Text,
                file_role,
                text_index_mode: TextIndexMode::for_role(file_role),
                compatibility_language: PLAIN_TEXT_LANGUAGE_ID,
                dialect: None,
                contexts: &affiliations,
                primary,
                classification_reason: "manual_text".to_string(),
                excluded_by_policy: false,
            }));
        }

        let auto_code_matches = self.matching_code_contexts(&path, false);
        if !auto_code_matches.is_empty() {
            let primary =
                choose_primary_context(&auto_code_matches).expect("non-empty automatic matches");
            return Ok(self.build_classification(ClassificationInput {
                analysis_mode: AnalysisMode::Code,
                file_role: FileRole::SourceCode,
                text_index_mode: TextIndexMode::None,
                compatibility_language: &resolved_language_for_context(&path, primary)?,
                dialect: resolved_dialect_for_path(&path),
                contexts: &auto_code_matches,
                primary: Some(primary),
                classification_reason: format!("auto_context:{}", primary.context.context_id),
                excluded_by_policy: false,
            }));
        }

        let path_is_auto_allowed = self.auto_scope.allows_path(&path);
        if path_is_auto_allowed && self.should_classify_as_text(&path) {
            let affiliations = self.matching_affiliated_contexts(&path);
            let primary = choose_primary_context(&affiliations);
            let file_role = FileRole::for_text_path(&path);
            return Ok(self.build_classification(ClassificationInput {
                analysis_mode: AnalysisMode::Text,
                file_role,
                text_index_mode: TextIndexMode::for_role(file_role),
                compatibility_language: PLAIN_TEXT_LANGUAGE_ID,
                dialect: None,
                contexts: &affiliations,
                primary,
                classification_reason: "auto_text".to_string(),
                excluded_by_policy: false,
            }));
        }

        let reason = if self.is_auto_ignored(&path) {
            "auto_ignored".to_string()
        } else if resolve_language_id_for_file_path(&path).is_some() {
            "contextless_code_like".to_string()
        } else {
            "track_only".to_string()
        };
        let affiliations = self.matching_affiliated_contexts(&path);
        let primary = choose_primary_context(&affiliations);
        let file_role = FileRole::for_track_only_path(&path, &reason);
        Ok(self.build_classification(ClassificationInput {
            analysis_mode: AnalysisMode::TrackOnly,
            file_role,
            text_index_mode: TextIndexMode::None,
            compatibility_language: TRACK_ONLY_LANGUAGE_ID,
            dialect: None,
            contexts: &affiliations,
            primary,
            classification_reason: reason,
            excluded_by_policy: false,
        }))
    }

    pub(crate) fn contexts(&self) -> Vec<ProjectContext> {
        self.contexts
            .iter()
            .map(|context| context.context.clone())
            .collect()
    }

    fn discover_with_view<V: RepoContentView>(
        repo_root: &Path,
        view: &V,
        candidate_paths: &[String],
        parser_version: &str,
        extractor_version: &str,
    ) -> Result<Self> {
        let policy = discover_repo_policy_optional(repo_root)
            .with_context(|| format!("loading repo policy from {}", repo_root.display()))?;
        let policy_root_abs = policy.root.unwrap_or_else(|| repo_root.to_path_buf());
        let policy_root_abs = policy_root_abs
            .canonicalize()
            .unwrap_or(policy_root_abs.clone());
        let repo_root_abs = repo_root
            .canonicalize()
            .unwrap_or_else(|_| repo_root.to_path_buf());

        let auto_scope =
            AutoScopePolicy::from_scope(&repo_root_abs, &policy_root_abs, &policy.scope)?;
        let include_as_text = parse_scope_string_list(&policy.scope, "include_as_text")?;
        let include_as_text = if include_as_text.is_empty() {
            None
        } else {
            Some(PathPatternMatcher::new(include_as_text)?)
        };

        let mut contexts = detect_auto_contexts(view, candidate_paths, &auto_scope)?;
        contexts.extend(parse_manual_context_specs(
            &policy.contexts,
            &repo_root_abs,
            &policy_root_abs,
        )?);
        contexts.sort_by(compare_context_priority);
        contexts.dedup_by(|lhs, rhs| lhs.context.context_id == rhs.context.context_id);

        Ok(Self {
            parser_version: parser_version.to_string(),
            extractor_version: extractor_version.to_string(),
            auto_scope,
            include_as_text,
            contexts,
        })
    }

    fn matching_code_contexts(&self, path: &str, manual_only: bool) -> Vec<&ResolvedContext> {
        let mut matches = self
            .contexts
            .iter()
            .filter(|context| {
                (context.manual || !manual_only && self.auto_scope.allows_path(path))
                    && context.matches_code_path(path)
            })
            .collect::<Vec<_>>();
        matches.sort_by(compare_context_refs);
        matches
    }

    fn matching_affiliated_contexts(&self, path: &str) -> Vec<&ResolvedContext> {
        let mut matches = self
            .contexts
            .iter()
            .filter(|context| {
                (context.manual || self.auto_scope.allows_path(path))
                    && context.matches_affiliated_path(path)
            })
            .collect::<Vec<_>>();
        matches.sort_by(compare_context_refs);
        matches
    }

    fn is_auto_ignored(&self, path: &str) -> bool {
        self.contexts
            .iter()
            .filter(|context| !context.manual)
            .any(|context| context.is_auto_ignored(path))
    }

    fn should_classify_as_text(&self, path: &str) -> bool {
        if self.is_auto_ignored(path) {
            return false;
        }
        is_auto_text_path(path)
    }

    fn build_classification(&self, input: ClassificationInput<'_>) -> ResolvedFileClassification {
        let ClassificationInput {
            analysis_mode,
            file_role,
            text_index_mode,
            compatibility_language,
            dialect,
            contexts,
            primary,
            classification_reason,
            excluded_by_policy,
        } = input;
        let primary_context_id = primary.map(|context| context.context.context_id.clone());
        let secondary_context_ids = contexts
            .iter()
            .copied()
            .filter(|context| {
                Some(context.context.context_id.as_str()) != primary_context_id.as_deref()
            })
            .map(|context| context.context.context_id.clone())
            .collect::<Vec<_>>();
        let frameworks = primary
            .map(|context| context.context.frameworks.clone())
            .unwrap_or_default();
        let runtime_profile = primary.and_then(|context| context.context.runtime_profile.clone());
        let context_fingerprint = primary.map(|context| context.context.config_fingerprint.clone());

        let extraction_fingerprint = compute_sha256_json(&Value::Object(Map::from_iter([
            (
                "analysis_mode".into(),
                Value::String(analysis_mode.as_str().to_string()),
            ),
            (
                "file_role".into(),
                Value::String(file_role.as_str().to_string()),
            ),
            (
                "text_index_mode".into(),
                Value::String(text_index_mode.as_str().to_string()),
            ),
            (
                "resolved_language".into(),
                Value::String(compatibility_language.to_string()),
            ),
            (
                "dialect".into(),
                dialect.clone().map(Value::String).unwrap_or(Value::Null),
            ),
            (
                "frameworks".into(),
                Value::Array(
                    frameworks
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect::<Vec<_>>(),
                ),
            ),
            (
                "runtime_profile".into(),
                runtime_profile
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "context_fingerprint".into(),
                context_fingerprint
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            ),
            (
                "parser_version".into(),
                Value::String(self.parser_version.clone()),
            ),
            (
                "extractor_version".into(),
                Value::String(self.extractor_version.clone()),
            ),
        ])));

        ResolvedFileClassification {
            analysis_mode,
            file_role,
            text_index_mode,
            language: compatibility_language.to_string(),
            resolved_language: compatibility_language.to_string(),
            dialect,
            primary_context_id,
            secondary_context_ids,
            frameworks,
            runtime_profile,
            classification_reason,
            context_fingerprint,
            extraction_fingerprint,
            excluded_by_policy,
        }
    }
}

fn resolved_language_for_context(path: &str, context: &ResolvedContext) -> Result<String> {
    if let Some(profile_id) = context.profile_id.as_deref() {
        let host = core_extension_host()?;
        let resolved = host
            .language_packs()
            .resolve(LanguagePackResolutionInput::for_profile(profile_id).with_file_path(path))
            .with_context(|| format!("resolving language profile `{profile_id}` for `{path}`"))?;
        return Ok(resolved.profile.language_id.to_string());
    }

    Ok(resolve_language_id_for_file_path(path)
        .unwrap_or(PLAIN_TEXT_LANGUAGE_ID)
        .to_string())
}

fn resolved_dialect_for_path(path: &str) -> Option<String> {
    match lower_extension(path).as_deref() {
        Some("tsx") => Some("tsx".to_string()),
        Some("jsx") => Some("jsx".to_string()),
        Some("ts" | "mts" | "cts") => Some("ts".to_string()),
        Some("js" | "mjs" | "cjs") => Some("js".to_string()),
        Some("py") => Some("py".to_string()),
        Some("go") => Some("go".to_string()),
        Some("java") => Some("java".to_string()),
        _ => None,
    }
}

fn choose_primary_context<'a>(contexts: &'a [&'a ResolvedContext]) -> Option<&'a ResolvedContext> {
    contexts.first().copied()
}

fn compare_context_priority(lhs: &ResolvedContext, rhs: &ResolvedContext) -> std::cmp::Ordering {
    compare_context_refs(&lhs, &rhs)
}

fn compare_context_refs(lhs: &&ResolvedContext, rhs: &&ResolvedContext) -> std::cmp::Ordering {
    rhs.root_depth
        .cmp(&lhs.root_depth)
        .then_with(|| rhs.manual.cmp(&lhs.manual))
        .then_with(|| {
            rhs.context
                .config_files
                .len()
                .cmp(&lhs.context.config_files.len())
        })
        .then_with(|| lhs.context.context_id.cmp(&rhs.context.context_id))
}
