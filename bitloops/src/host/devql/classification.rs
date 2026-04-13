use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::config::discover_repo_policy_optional;
use crate::host::extension_host::LanguagePackResolutionInput;

use super::PLAIN_TEXT_LANGUAGE_ID;

pub(crate) const TRACK_ONLY_LANGUAGE_ID: &str = "track_only";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalysisMode {
    Code,
    Text,
    TrackOnly,
    Excluded,
}

impl AnalysisMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Text => "text",
            Self::TrackOnly => "track_only",
            Self::Excluded => "excluded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileRole {
    SourceCode,
    ProjectManifest,
    ContextSeed,
    Configuration,
    Documentation,
    Lockfile,
    Generated,
    DependencyTree,
    Other,
}

impl FileRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SourceCode => "source_code",
            Self::ProjectManifest => "project_manifest",
            Self::ContextSeed => "context_seed",
            Self::Configuration => "configuration",
            Self::Documentation => "documentation",
            Self::Lockfile => "lockfile",
            Self::Generated => "generated",
            Self::DependencyTree => "dependency_tree",
            Self::Other => "other",
        }
    }

    fn for_auto_ignored_path(path: &str) -> Self {
        let path = super::normalize_repo_path(path);
        if path.contains("/node_modules/")
            || path.starts_with("node_modules/")
            || path.contains("/vendor/")
            || path.starts_with("vendor/")
        {
            Self::DependencyTree
        } else {
            Self::Generated
        }
    }

    fn for_text_path(path: &str) -> Self {
        let file_name = file_name(path);
        if file_name.starts_with("README")
            || file_name.starts_with("CHANGELOG")
            || file_name.starts_with("LICENSE")
            || matches!(
                lower_extension(path).as_deref(),
                Some("md" | "mdx" | "txt" | "rst" | "adoc")
            )
        {
            return Self::Documentation;
        }
        if matches!(
            file_name,
            "Cargo.toml"
                | "package.json"
                | "pyproject.toml"
                | "setup.py"
                | "setup.cfg"
                | "Pipfile"
                | "go.mod"
                | "pom.xml"
        ) {
            return Self::ProjectManifest;
        }
        if is_tsconfig_file(file_name)
            || is_jsconfig_file(file_name)
            || file_name.starts_with("build.gradle")
        {
            return Self::ContextSeed;
        }
        Self::Configuration
    }

    fn for_track_only_path(path: &str, reason: &str) -> Self {
        let file_name = file_name(path);
        if is_lockfile_name(file_name) {
            return Self::Lockfile;
        }
        match reason {
            "auto_ignored" => Self::for_auto_ignored_path(path),
            "contextless_code_like" => Self::SourceCode,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextIndexMode {
    Embed,
    StoreOnly,
    None,
}

impl TextIndexMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Embed => "embed",
            Self::StoreOnly => "store_only",
            Self::None => "none",
        }
    }

    fn for_role(role: FileRole) -> Self {
        match role {
            FileRole::Documentation => Self::Embed,
            FileRole::ProjectManifest | FileRole::ContextSeed | FileRole::Configuration => {
                Self::StoreOnly
            }
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectContext {
    pub(crate) context_id: String,
    pub(crate) root: String,
    pub(crate) kind: String,
    pub(crate) detection_source: String,
    pub(crate) config_files: Vec<String>,
    pub(crate) config_fingerprint: String,
    pub(crate) base_languages: Vec<String>,
    pub(crate) frameworks: Vec<String>,
    pub(crate) runtime_profile: Option<String>,
    pub(crate) source_versions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedFileClassification {
    pub(crate) analysis_mode: AnalysisMode,
    pub(crate) file_role: FileRole,
    pub(crate) text_index_mode: TextIndexMode,
    pub(crate) language: String,
    pub(crate) resolved_language: String,
    pub(crate) dialect: Option<String>,
    pub(crate) primary_context_id: Option<String>,
    pub(crate) secondary_context_ids: Vec<String>,
    pub(crate) frameworks: Vec<String>,
    pub(crate) runtime_profile: Option<String>,
    pub(crate) classification_reason: String,
    pub(crate) context_fingerprint: Option<String>,
    pub(crate) extraction_fingerprint: String,
    pub(crate) excluded_by_policy: bool,
}

impl ResolvedFileClassification {
    pub(crate) fn should_persist_current_state(&self) -> bool {
        self.analysis_mode != AnalysisMode::Excluded
    }

    pub(crate) fn should_extract(&self) -> bool {
        matches!(self.analysis_mode, AnalysisMode::Code)
            || (self.analysis_mode == AnalysisMode::Text
                && self.text_index_mode != TextIndexMode::None)
    }
}

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
        let path = super::normalize_repo_path(path);
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
        } else if super::resolve_language_id_for_file_path(&path).is_some() {
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

        let auto_scope = AutoScopePolicy::from_scope(
            repo_root,
            &repo_root_abs,
            &policy_root_abs,
            &policy.scope,
        )?;
        let include_as_text = parse_scope_string_list(&policy.scope, "include_as_text")?;
        let include_as_text = if include_as_text.is_empty() {
            None
        } else {
            Some(PathPatternMatcher::new(include_as_text)?)
        };

        let manual_specs =
            parse_manual_context_specs(&policy.contexts, &repo_root_abs, &policy_root_abs)?;
        let manual_contexts = manual_specs
            .iter()
            .map(build_manual_context)
            .collect::<Result<Vec<_>>>()?;

        let mut contexts = detect_auto_contexts(view, candidate_paths, &auto_scope)?;
        contexts.extend(manual_contexts);
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

trait RepoContentView {
    fn list_dir_entries(&self, dir: &str) -> Result<Vec<String>>;
    fn read_text(&self, path: &str) -> Result<Option<String>>;
}

struct FsRepoContentView {
    repo_root: PathBuf,
}

impl FsRepoContentView {
    fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }
}

impl RepoContentView for FsRepoContentView {
    fn list_dir_entries(&self, dir: &str) -> Result<Vec<String>> {
        let full = if dir.is_empty() {
            self.repo_root.clone()
        } else {
            self.repo_root.join(dir)
        };
        let entries = match fs::read_dir(&full) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(anyhow::Error::from(err))
                    .with_context(|| format!("listing directory {}", full.display()));
            }
        };
        let mut out = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        out.sort();
        out.dedup();
        Ok(out)
    }

    fn read_text(&self, path: &str) -> Result<Option<String>> {
        let full = self.repo_root.join(path);
        match fs::read_to_string(&full) {
            Ok(content) => Ok(Some(content)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => Ok(None),
            Err(err) => {
                Err(anyhow::Error::from(err)).with_context(|| format!("reading {}", full.display()))
            }
        }
    }
}

struct RevisionRepoContentView {
    repo_root: PathBuf,
    revision: String,
    dir_entries: HashMap<String, Vec<String>>,
}

impl RevisionRepoContentView {
    fn new(repo_root: PathBuf, revision: String, candidate_paths: &[String]) -> Self {
        let mut dir_entries = HashMap::<String, BTreeSet<String>>::new();
        for path in candidate_paths {
            let parent = parent_dir(path);
            let file_name = Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            if !file_name.is_empty() {
                dir_entries.entry(parent).or_default().insert(file_name);
            }
        }
        Self {
            repo_root,
            revision,
            dir_entries: dir_entries
                .into_iter()
                .map(|(dir, entries)| (dir, entries.into_iter().collect()))
                .collect(),
        }
    }
}

impl RepoContentView for RevisionRepoContentView {
    fn list_dir_entries(&self, dir: &str) -> Result<Vec<String>> {
        Ok(self.dir_entries.get(dir).cloned().unwrap_or_default())
    }

    fn read_text(&self, path: &str) -> Result<Option<String>> {
        let spec = format!("{}:{}", self.revision, path);
        match crate::host::checkpoints::strategy::manual_commit::run_git(
            &self.repo_root,
            &["show", &spec],
        ) {
            Ok(content) => Ok(Some(content)),
            Err(_) => Ok(None),
        }
    }
}

#[derive(Debug, Clone)]
struct AutoScopePolicy {
    root_rel: Option<String>,
    include_matcher: Option<PathPatternMatcher>,
}

impl AutoScopePolicy {
    fn from_scope(
        _repo_root: &Path,
        repo_root_abs: &Path,
        policy_root_abs: &Path,
        scope: &Value,
    ) -> Result<Self> {
        let project_root = scope
            .as_object()
            .and_then(|map| map.get("project_root"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let include = parse_scope_string_list(scope, "include")?;
        let root_rel = project_root
            .map(|value| resolve_policy_relative_path(repo_root_abs, policy_root_abs, value))
            .transpose()?
            .map(|path| {
                path.strip_prefix(repo_root_abs)
                    .unwrap_or(path.as_path())
                    .to_path_buf()
            })
            .map(|path| pathbuf_to_repo_relative(&path));
        let include_matcher = if include.is_empty() {
            None
        } else {
            Some(PathPatternMatcher::new(include)?)
        };
        Ok(Self {
            root_rel,
            include_matcher,
        })
    }

    fn allows_path(&self, path: &str) -> bool {
        let Some(relative_to_root) = self.relative_to_auto_root(path) else {
            return false;
        };
        self.include_matcher
            .as_ref()
            .is_none_or(|matcher| matcher.is_match(&relative_to_root))
    }

    fn allows_context_root(&self, path: &str) -> bool {
        self.relative_to_auto_root(path).is_some()
    }

    fn relative_to_auto_root(&self, path: &str) -> Option<String> {
        let path = super::normalize_repo_path(path);
        if let Some(root_rel) = &self.root_rel {
            if root_rel.is_empty() {
                return Some(path);
            }
            if path == *root_rel {
                return Some(String::new());
            }
            let prefix = format!("{root_rel}/");
            return path.strip_prefix(&prefix).map(str::to_string);
        }
        Some(path)
    }
}

#[derive(Debug, Clone)]
struct ResolvedContext {
    context: ProjectContext,
    root_rel: String,
    root_depth: usize,
    manual: bool,
    include_matcher: Option<PathPatternMatcher>,
    exclude_matcher: Option<PathPatternMatcher>,
    code_extensions: HashSet<String>,
    profile_id: Option<String>,
    auto_ignored_dirs: Vec<String>,
}

impl ResolvedContext {
    fn matches_code_path(&self, path: &str) -> bool {
        if !self.matches_affiliated_path(path) {
            return false;
        }
        let extension = lower_extension(path);
        extension
            .as_deref()
            .is_some_and(|extension| self.code_extensions.contains(extension))
    }

    fn matches_affiliated_path(&self, path: &str) -> bool {
        let Some(relative) = relative_to_root(path, &self.root_rel) else {
            return false;
        };
        if self.is_auto_ignored(path) {
            return false;
        }
        if self
            .include_matcher
            .as_ref()
            .is_some_and(|matcher| !matcher.is_match(&relative))
        {
            return false;
        }
        if self
            .exclude_matcher
            .as_ref()
            .is_some_and(|matcher| matcher.is_match(&relative))
        {
            return false;
        }
        true
    }

    fn is_auto_ignored(&self, path: &str) -> bool {
        let Some(relative) = relative_to_root(path, &self.root_rel) else {
            return false;
        };
        self.auto_ignored_dirs
            .iter()
            .any(|dir| relative == *dir || relative.starts_with(&format!("{dir}/")))
    }
}

#[derive(Debug, Clone)]
struct ManualContextSpec {
    id: String,
    kind: String,
    root_rel: String,
    include: Vec<String>,
    exclude: Vec<String>,
    frameworks: Vec<String>,
    runtime_profile: Option<String>,
    source_version_override: Option<String>,
    profile: Option<String>,
}

struct AutoContextBuildSpec<'a> {
    kind: &'a str,
    dir: &'a str,
    config_files: Vec<String>,
    base_languages: Vec<String>,
    frameworks: Vec<String>,
    runtime_profile: Option<String>,
    source_versions: BTreeMap<String, String>,
    code_extensions: HashSet<String>,
    profile_id: Option<String>,
}

fn detect_auto_contexts<V: RepoContentView>(
    view: &V,
    candidate_paths: &[String],
    auto_scope: &AutoScopePolicy,
) -> Result<Vec<ResolvedContext>> {
    let mut directories = BTreeSet::new();
    for path in candidate_paths {
        for ancestor in ancestor_dirs(path) {
            directories.insert(ancestor);
        }
    }
    directories.insert(String::new());

    let mut contexts = Vec::new();
    for dir in directories {
        let entries = view.list_dir_entries(&dir)?;
        if entries.is_empty() && !dir.is_empty() {
            continue;
        }

        if let Some(context) = build_rust_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_node_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_typescript_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_python_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_go_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
        if let Some(context) = build_java_context(view, auto_scope, &dir, &entries)? {
            contexts.push(context);
        }
    }

    Ok(contexts)
}

fn build_manual_context(spec: &ManualContextSpec) -> Result<ResolvedContext> {
    let (base_language, code_extensions) = match spec.kind.as_str() {
        "rust" => ("rust".to_string(), vec!["rs".to_string()]),
        "node" => (
            "javascript".to_string(),
            ["js", "jsx", "mjs", "cjs"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        ),
        "python" => ("python".to_string(), vec!["py".to_string()]),
        "go" => ("go".to_string(), vec!["go".to_string()]),
        "java" => ("java".to_string(), vec!["java".to_string()]),
        "standalone" => {
            let profile = spec.profile.as_deref().ok_or_else(|| {
                anyhow::anyhow!("manual standalone context `{}` requires `profile`", spec.id)
            })?;
            let host = super::core_extension_host()?;
            let resolved = host
                .language_packs()
                .resolve(LanguagePackResolutionInput::for_profile(profile))
                .with_context(|| format!("resolving standalone context profile `{profile}`"))?;
            (
                resolved.profile.language_id.to_string(),
                resolved
                    .profile
                    .file_extensions
                    .iter()
                    .map(|value| value.to_ascii_lowercase())
                    .collect(),
            )
        }
        other => bail!("unsupported manual context kind `{other}`"),
    };

    let mut source_versions = BTreeMap::new();
    if let Some(source_version) = spec.source_version_override.as_deref() {
        source_versions.insert(base_language.clone(), source_version.to_string());
    }
    let config_value = Value::Object(Map::from_iter([
        ("id".into(), Value::String(spec.id.clone())),
        ("kind".into(), Value::String(spec.kind.clone())),
        ("root".into(), Value::String(spec.root_rel.clone())),
        (
            "include".into(),
            Value::Array(spec.include.iter().cloned().map(Value::String).collect()),
        ),
        (
            "exclude".into(),
            Value::Array(spec.exclude.iter().cloned().map(Value::String).collect()),
        ),
        (
            "frameworks".into(),
            Value::Array(spec.frameworks.iter().cloned().map(Value::String).collect()),
        ),
        (
            "runtime_profile".into(),
            spec.runtime_profile
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "source_versions".into(),
            Value::Object(Map::from_iter(
                source_versions
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone()))),
            )),
        ),
        (
            "profile".into(),
            spec.profile
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
    ]));

    Ok(ResolvedContext {
        context: ProjectContext {
            context_id: spec.id.clone(),
            root: display_root(&spec.root_rel),
            kind: spec.kind.clone(),
            detection_source: "manual".to_string(),
            config_files: Vec::new(),
            config_fingerprint: compute_sha256_json(&config_value),
            base_languages: vec![base_language],
            frameworks: sorted_dedup(spec.frameworks.clone()),
            runtime_profile: spec.runtime_profile.clone(),
            source_versions,
        },
        root_rel: spec.root_rel.clone(),
        root_depth: path_depth(&spec.root_rel),
        manual: true,
        include_matcher: (!spec.include.is_empty())
            .then(|| PathPatternMatcher::new(spec.include.clone()))
            .transpose()?,
        exclude_matcher: (!spec.exclude.is_empty())
            .then(|| PathPatternMatcher::new(spec.exclude.clone()))
            .transpose()?,
        code_extensions: code_extensions.into_iter().collect(),
        profile_id: spec.profile.clone(),
        auto_ignored_dirs: auto_ignored_dirs_for_kind(&spec.kind),
    })
}

fn build_rust_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    if !entries.iter().any(|entry| entry == "Cargo.toml") {
        return Ok(None);
    }
    let cargo_path = join_dir_file(dir, "Cargo.toml");
    if !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let cargo_content = view.read_text(&cargo_path)?.unwrap_or_default();
    let toolchain_path = join_dir_file(dir, "rust-toolchain.toml");
    let toolchain_content = view.read_text(&toolchain_path)?;
    let mut config_files = vec![cargo_path.clone()];
    let mut source_versions = BTreeMap::new();
    if let Some(version) = toolchain_content
        .as_deref()
        .and_then(parse_rust_toolchain_channel)
        .or_else(|| parse_cargo_rust_version(&cargo_content))
    {
        source_versions.insert("rust".to_string(), version);
    }
    if toolchain_content.is_some() {
        config_files.push(toolchain_path.clone());
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "rust",
        dir,
        config_files,
        base_languages: vec!["rust".to_string()],
        frameworks: Vec::new(),
        runtime_profile: None,
        source_versions,
        code_extensions: HashSet::from(["rs".to_string()]),
        profile_id: None,
    })))
}

fn build_node_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    let jsconfigs = matching_entries(entries, is_jsconfig_file);
    let has_package_json = entries.iter().any(|entry| entry == "package.json");
    if !has_package_json && jsconfigs.is_empty() {
        return Ok(None);
    }
    let package_path = join_dir_file(dir, "package.json");
    let jsconfig_paths = jsconfigs
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    let seed_path = if has_package_json {
        package_path.as_str()
    } else {
        jsconfig_paths
            .first()
            .map(String::as_str)
            .unwrap_or_default()
    };
    if seed_path.is_empty() || !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let package_content = if has_package_json {
        view.read_text(&package_path)?
    } else {
        None
    };
    let next_configs = matching_entries(entries, |entry| entry.starts_with("next.config."));
    let next_config_paths = next_configs
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    let frameworks = package_content
        .as_deref()
        .map(parse_package_frameworks)
        .transpose()?
        .unwrap_or_default();
    let runtime_profile = frameworks
        .iter()
        .find(|value| value.starts_with("next"))
        .map(|_| "next".to_string())
        .or_else(|| {
            frameworks
                .iter()
                .find(|value| value.starts_with("react"))
                .map(|_| "react".to_string())
        });
    let mut config_files = Vec::new();
    if has_package_json {
        config_files.push(package_path.clone());
    }
    config_files.extend(jsconfig_paths.clone());
    config_files.extend(next_config_paths.clone());
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "node",
        dir,
        config_files,
        base_languages: vec!["javascript".to_string()],
        frameworks,
        runtime_profile,
        source_versions: BTreeMap::new(),
        code_extensions: HashSet::from_iter(
            ["js", "jsx", "mjs", "cjs"].into_iter().map(str::to_string),
        ),
        profile_id: None,
    })))
}

fn build_typescript_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    let tsconfigs = matching_entries(entries, |entry| {
        is_tsconfig_file(entry) || is_jsconfig_file(entry)
    });
    let package_path = join_dir_file(dir, "package.json");
    let package_content = view.read_text(&package_path)?;
    let has_typescript_dependency = package_content
        .as_deref()
        .and_then(parse_package_dependency_version)
        .is_some();
    if tsconfigs.is_empty() && !has_typescript_dependency {
        return Ok(None);
    }
    let seed_path = tsconfigs
        .first()
        .map(|entry| join_dir_file(dir, entry))
        .or_else(|| has_typescript_dependency.then_some(package_path.clone()))
        .unwrap_or_default();
    if seed_path.is_empty() || !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let next_configs = matching_entries(entries, |entry| entry.starts_with("next.config."));
    let mut config_files = tsconfigs
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    if package_content.is_some() {
        config_files.push(package_path.clone());
    }
    config_files.extend(next_configs.iter().map(|entry| join_dir_file(dir, entry)));
    let frameworks = package_content
        .as_deref()
        .map(parse_package_frameworks)
        .transpose()?
        .unwrap_or_default();
    let runtime_profile = frameworks
        .iter()
        .find(|value| value.starts_with("next"))
        .map(|_| "next".to_string())
        .or_else(|| {
            frameworks
                .iter()
                .find(|value| value.starts_with("react"))
                .map(|_| "react".to_string())
        });
    let mut source_versions = BTreeMap::new();
    if let Some(version) = package_content
        .as_deref()
        .and_then(parse_package_dependency_version)
    {
        source_versions.insert("typescript".to_string(), version);
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "typescript",
        dir,
        config_files,
        base_languages: vec!["typescript".to_string()],
        frameworks,
        runtime_profile,
        source_versions,
        code_extensions: HashSet::from_iter(
            ["ts", "tsx", "mts", "cts"].into_iter().map(str::to_string),
        ),
        profile_id: None,
    })))
}

fn build_python_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    let seeds = matching_entries(entries, |entry| {
        entry == "pyproject.toml"
            || entry == "setup.py"
            || entry == "setup.cfg"
            || entry == "Pipfile"
            || is_requirements_file(entry)
    });
    if seeds.is_empty() {
        return Ok(None);
    }
    let seed_path = join_dir_file(dir, &seeds[0]);
    if seed_path.is_empty() || !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let mut config_files = seeds
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    let python_version_path = join_dir_file(dir, ".python-version");
    let python_version_content = view.read_text(&python_version_path)?;
    if python_version_content.is_some() {
        config_files.push(python_version_path.clone());
    }
    let pyproject_path = join_dir_file(dir, "pyproject.toml");
    let pyproject_content = view.read_text(&pyproject_path)?;
    let mut source_versions = BTreeMap::new();
    if let Some(version) = pyproject_content
        .as_deref()
        .and_then(parse_pyproject_requires_python)
        .or_else(|| {
            python_version_content
                .as_deref()
                .map(str::trim)
                .map(str::to_string)
        })
        .filter(|value| !value.is_empty())
    {
        source_versions.insert("python".to_string(), version);
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "python",
        dir,
        config_files,
        base_languages: vec!["python".to_string()],
        frameworks: Vec::new(),
        runtime_profile: None,
        source_versions,
        code_extensions: HashSet::from(["py".to_string()]),
        profile_id: None,
    })))
}

fn build_go_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    if !entries.iter().any(|entry| entry == "go.mod") {
        return Ok(None);
    }
    let go_mod_path = join_dir_file(dir, "go.mod");
    if !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let go_mod = view.read_text(&go_mod_path)?.unwrap_or_default();
    let mut source_versions = BTreeMap::new();
    if let Some(version) = parse_go_mod_version(&go_mod) {
        source_versions.insert("go".to_string(), version);
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "go",
        dir,
        config_files: vec![go_mod_path],
        base_languages: vec!["go".to_string()],
        frameworks: Vec::new(),
        runtime_profile: None,
        source_versions,
        code_extensions: HashSet::from(["go".to_string()]),
        profile_id: None,
    })))
}

fn build_java_context<V: RepoContentView>(
    view: &V,
    auto_scope: &AutoScopePolicy,
    dir: &str,
    entries: &[String],
) -> Result<Option<ResolvedContext>> {
    let seeds = matching_entries(entries, |entry| {
        matches!(
            entry,
            "pom.xml"
                | "build.gradle"
                | "build.gradle.kts"
                | "settings.gradle"
                | "settings.gradle.kts"
        )
    });
    if seeds.is_empty() {
        return Ok(None);
    }
    let seed_path = join_dir_file(dir, &seeds[0]);
    if seed_path.is_empty() || !auto_scope.allows_context_root(dir) {
        return Ok(None);
    }
    let config_files = seeds
        .iter()
        .map(|entry| join_dir_file(dir, entry))
        .collect::<Vec<_>>();
    let mut source_versions = BTreeMap::new();
    for config_file in &config_files {
        let Some(content) = view.read_text(config_file)? else {
            continue;
        };
        if let Some(version) = parse_java_version(&content).filter(|value| !value.trim().is_empty())
        {
            source_versions.insert("java".to_string(), version);
            break;
        }
    }
    if !source_versions.contains_key("java") {
        source_versions.insert("java".to_string(), "unknown".to_string());
    }
    Ok(Some(build_auto_context(AutoContextBuildSpec {
        kind: "java",
        dir,
        config_files,
        base_languages: vec!["java".to_string()],
        frameworks: Vec::new(),
        runtime_profile: None,
        source_versions,
        code_extensions: HashSet::from(["java".to_string()]),
        profile_id: None,
    })))
}

fn build_auto_context(spec: AutoContextBuildSpec<'_>) -> ResolvedContext {
    let AutoContextBuildSpec {
        kind,
        dir,
        config_files,
        base_languages,
        frameworks,
        runtime_profile,
        source_versions,
        code_extensions,
        profile_id,
    } = spec;
    let config_files = sorted_dedup(config_files);
    let frameworks = sorted_dedup(frameworks);
    let config_value = Value::Object(Map::from_iter([
        ("kind".into(), Value::String(kind.to_string())),
        ("root".into(), Value::String(display_root(dir))),
        (
            "config_files".into(),
            Value::Array(config_files.iter().cloned().map(Value::String).collect()),
        ),
        (
            "frameworks".into(),
            Value::Array(frameworks.iter().cloned().map(Value::String).collect()),
        ),
        (
            "runtime_profile".into(),
            runtime_profile
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
        ),
        (
            "source_versions".into(),
            Value::Object(Map::from_iter(
                source_versions
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone()))),
            )),
        ),
    ]));
    let context_id = format!("auto:{kind}:{}", display_root(dir));
    let context_fingerprint = compute_sha256_json(&config_value);
    ResolvedContext {
        context: ProjectContext {
            context_id,
            root: display_root(dir),
            kind: kind.to_string(),
            detection_source: "auto".to_string(),
            config_files,
            config_fingerprint: context_fingerprint,
            base_languages,
            frameworks,
            runtime_profile,
            source_versions,
        },
        root_rel: dir.to_string(),
        root_depth: path_depth(dir),
        manual: false,
        include_matcher: None,
        exclude_matcher: None,
        code_extensions,
        profile_id,
        auto_ignored_dirs: auto_ignored_dirs_for_kind(kind),
    }
}

fn parse_manual_context_specs(
    contexts: &Value,
    repo_root_abs: &Path,
    policy_root_abs: &Path,
) -> Result<Vec<ManualContextSpec>> {
    let Some(items) = contexts.as_array() else {
        if contexts.is_null() || contexts.as_object().is_some_and(serde_json::Map::is_empty) {
            return Ok(Vec::new());
        }
        bail!("`contexts` must be an array of tables");
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let Some(map) = item.as_object() else {
            bail!("`contexts` entries must be objects");
        };
        let id = required_string(map, "id")?;
        if !seen.insert(id.clone()) {
            bail!("duplicate manual context id `{id}`");
        }
        let kind = required_string(map, "kind")?;
        let root_raw = required_string(map, "root")?;
        let root_abs = resolve_policy_relative_path(repo_root_abs, policy_root_abs, &root_raw)?;
        let root_rel =
            pathbuf_to_repo_relative(root_abs.strip_prefix(repo_root_abs).unwrap_or(&root_abs));
        out.push(ManualContextSpec {
            id,
            kind,
            root_rel,
            include: optional_string_list(map, "include")?,
            exclude: optional_string_list(map, "exclude")?,
            frameworks: optional_string_list(map, "frameworks")?,
            runtime_profile: optional_string(map, "runtime_profile")?,
            source_version_override: optional_string(map, "source_version_override")?,
            profile: optional_string(map, "profile")?,
        });
    }
    Ok(out)
}

fn required_string(map: &Map<String, Value>, key: &str) -> Result<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .with_context(|| format!("`contexts.{key}` must be a non-empty string"))
}

fn optional_string(map: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    Ok(value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn optional_string_list(map: &Map<String, Value>, key: &str) -> Result<Vec<String>> {
    let Some(value) = map.get(key) else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        bail!("`contexts.{key}` must be an array of strings");
    };
    Ok(values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect())
}

fn parse_scope_string_list(scope: &Value, key: &str) -> Result<Vec<String>> {
    let Some(scope_map) = scope.as_object() else {
        return Ok(Vec::new());
    };
    let Some(raw) = scope_map.get(key) else {
        return Ok(Vec::new());
    };
    let raw_values = raw
        .as_array()
        .with_context(|| format!("`scope.{key}` must be an array of strings"))?;
    Ok(raw_values
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect())
}

fn resolved_language_for_context(path: &str, context: &ResolvedContext) -> Result<String> {
    if let Some(profile_id) = context.profile_id.as_deref() {
        let host = super::core_extension_host()?;
        let resolved = host
            .language_packs()
            .resolve(LanguagePackResolutionInput::for_profile(profile_id).with_file_path(path))
            .with_context(|| format!("resolving language profile `{profile_id}` for `{path}`"))?;
        return Ok(resolved.profile.language_id.to_string());
    }

    Ok(super::resolve_language_id_for_file_path(path)
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

fn normalise_candidate_paths<I, S>(candidate_paths: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = candidate_paths
        .into_iter()
        .map(|value| super::normalize_repo_path(value.as_ref()))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn compute_sha256_json(value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(value).unwrap_or_else(|_| value.to_string().into_bytes()));
    hex::encode(hasher.finalize())
}

fn join_dir_file(dir: &str, file_name: &str) -> String {
    if dir.is_empty() {
        file_name.to_string()
    } else {
        format!("{dir}/{file_name}")
    }
}

fn parent_dir(path: &str) -> String {
    Path::new(path)
        .parent()
        .and_then(|value| value.to_str())
        .map(super::normalize_repo_path)
        .unwrap_or_default()
}

fn ancestor_dirs(path: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut current = parent_dir(path);
    loop {
        dirs.push(current.clone());
        if current.is_empty() {
            break;
        }
        current = parent_dir(&current);
    }
    dirs
}

fn relative_to_root(path: &str, root_rel: &str) -> Option<String> {
    let path = super::normalize_repo_path(path);
    if root_rel.is_empty() {
        return Some(path);
    }
    if path == root_rel {
        return Some(String::new());
    }
    let prefix = format!("{root_rel}/");
    path.strip_prefix(&prefix).map(str::to_string)
}

fn lower_extension(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
}

fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
}

fn path_depth(path: &str) -> usize {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .count()
}

fn matching_entries<F>(entries: &[String], predicate: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    entries
        .iter()
        .filter(|entry| predicate(entry))
        .cloned()
        .collect()
}

fn is_tsconfig_file(entry: &str) -> bool {
    entry == "tsconfig.json" || (entry.starts_with("tsconfig.") && entry.ends_with(".json"))
}

fn is_jsconfig_file(entry: &str) -> bool {
    entry == "jsconfig.json" || (entry.starts_with("jsconfig.") && entry.ends_with(".json"))
}

fn is_requirements_file(entry: &str) -> bool {
    entry == "requirements.txt" || (entry.starts_with("requirements") && entry.ends_with(".txt"))
}

fn is_auto_text_path(path: &str) -> bool {
    let path = super::normalize_repo_path(path);
    let file_name = Path::new(&path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let file_name_lower = file_name.to_ascii_lowercase();
    if file_name.starts_with("README")
        || file_name.starts_with("CHANGELOG")
        || file_name.starts_with("LICENSE")
        || file_name_lower.starts_with(".env")
    {
        return !is_lockfile_name(file_name);
    }
    if matches!(
        file_name,
        "Cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "setup.py"
            | "setup.cfg"
            | "Pipfile"
            | "go.mod"
            | "pom.xml"
    ) {
        return true;
    }
    if is_tsconfig_file(file_name) || is_jsconfig_file(file_name) || is_requirements_file(file_name)
    {
        return true;
    }
    if file_name.starts_with("build.gradle") {
        return true;
    }
    if is_lockfile_name(file_name) {
        return false;
    }
    matches!(
        lower_extension(&path).as_deref(),
        Some(
            "md" | "mdx"
                | "txt"
                | "rst"
                | "adoc"
                | "toml"
                | "yaml"
                | "yml"
                | "json"
                | "jsonc"
                | "ini"
                | "cfg"
                | "conf"
        )
    )
}

fn is_lockfile_name(file_name: &str) -> bool {
    matches!(
        file_name,
        "Cargo.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "poetry.lock"
            | "Pipfile.lock"
    ) || file_name.ends_with(".lock")
}

fn auto_ignored_dirs_for_kind(kind: &str) -> Vec<String> {
    match kind {
        "rust" => vec!["target".to_string()],
        "node" | "typescript" => vec![
            "node_modules".to_string(),
            ".next".to_string(),
            "dist".to_string(),
            "build".to_string(),
            "coverage".to_string(),
        ],
        "python" => vec![
            ".venv".to_string(),
            "venv".to_string(),
            "__pycache__".to_string(),
            ".pytest_cache".to_string(),
            ".mypy_cache".to_string(),
        ],
        "go" => vec!["vendor".to_string(), "bin".to_string()],
        "java" => vec![
            "target".to_string(),
            "build".to_string(),
            ".gradle".to_string(),
        ],
        _ => Vec::new(),
    }
}

fn resolve_policy_relative_path(
    repo_root_abs: &Path,
    policy_root_abs: &Path,
    raw_value: &str,
) -> Result<PathBuf> {
    let candidate = PathBuf::from(raw_value);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        policy_root_abs.join(candidate)
    };
    let resolved = resolved.canonicalize().unwrap_or(resolved);
    if !resolved.starts_with(repo_root_abs) {
        bail!(
            "path `{}` resolves outside repo root {}",
            raw_value,
            repo_root_abs.display()
        );
    }
    Ok(resolved)
}

fn pathbuf_to_repo_relative(path: &Path) -> String {
    normalize_lexical_path(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

fn display_root(root_rel: &str) -> String {
    if root_rel.is_empty() {
        ".".to_string()
    } else {
        root_rel.to_string()
    }
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn sorted_dedup(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn parse_package_dependency_version(content: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(content).ok()?;
    parsed
        .get("dependencies")
        .and_then(Value::as_object)
        .and_then(|deps| deps.get("typescript"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            parsed
                .get("devDependencies")
                .and_then(Value::as_object)
                .and_then(|deps| deps.get("typescript"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn parse_package_frameworks(content: &str) -> Result<Vec<String>> {
    let parsed = serde_json::from_str::<Value>(content).context("parsing package.json")?;
    let mut frameworks = Vec::new();
    for dependency in ["react", "next"] {
        let version = parsed
            .get("dependencies")
            .and_then(Value::as_object)
            .and_then(|deps| deps.get(dependency))
            .and_then(Value::as_str)
            .or_else(|| {
                parsed
                    .get("devDependencies")
                    .and_then(Value::as_object)
                    .and_then(|deps| deps.get(dependency))
                    .and_then(Value::as_str)
            });
        if let Some(version) = version {
            frameworks.push(format!("{dependency}@{version}"));
        }
    }
    Ok(sorted_dedup(frameworks))
}

fn parse_rust_toolchain_channel(content: &str) -> Option<String> {
    Regex::new(r#"(?m)^\s*channel\s*=\s*"([^"]+)""#)
        .ok()?
        .captures(content)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_cargo_rust_version(content: &str) -> Option<String> {
    Regex::new(r#"(?m)^\s*rust-version\s*=\s*"([^"]+)""#)
        .ok()?
        .captures(content)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_pyproject_requires_python(content: &str) -> Option<String> {
    Regex::new(r#"(?m)^\s*requires-python\s*=\s*"([^"]+)""#)
        .ok()?
        .captures(content)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_go_mod_version(content: &str) -> Option<String> {
    Regex::new(r#"(?m)^\s*go\s+([^\s]+)\s*$"#)
        .ok()?
        .captures(content)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

fn parse_java_version(content: &str) -> Option<String> {
    for pattern in [
        r#"(?s)<maven\.compiler\.release>\s*([^<\s]+)\s*</maven\.compiler\.release>"#,
        r#"(?s)<maven\.compiler\.source>\s*([^<\s]+)\s*</maven\.compiler\.source>"#,
        r#"(?s)<java\.version>\s*([^<\s]+)\s*</java\.version>"#,
        r#"(?m)^\s*sourceCompatibility\s*=\s*['"]?([^'"\s]+)"#,
        r#"(?m)^\s*targetCompatibility\s*=\s*['"]?([^'"\s]+)"#,
    ] {
        if let Some(version) = Regex::new(pattern)
            .ok()
            .and_then(|regex| regex.captures(content))
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string())
        {
            return Some(version);
        }
    }
    None
}

#[derive(Debug, Clone)]
struct PathPatternMatcher {
    patterns: Vec<Regex>,
}

impl PathPatternMatcher {
    fn new(patterns: Vec<String>) -> Result<Self> {
        let patterns = patterns
            .into_iter()
            .map(|pattern| normalize_pattern(&pattern))
            .filter(|pattern| !pattern.is_empty())
            .collect::<Vec<_>>();
        let compiled = patterns
            .iter()
            .map(|pattern| compile_pattern(pattern))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { patterns: compiled })
    }

    fn is_match(&self, path: &str) -> bool {
        let path = normalize_relative_path(path);
        !path.is_empty() && self.patterns.iter().any(|pattern| pattern.is_match(&path))
    }
}

fn normalize_pattern(pattern: &str) -> String {
    let mut normalized = pattern.trim().replace('\\', "/");
    let anchored_to_root = normalized.starts_with("./") || normalized.starts_with('/');
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    while normalized.starts_with('/') {
        normalized.remove(0);
    }
    if normalized.ends_with('/') {
        normalized.push_str("**");
    }
    if anchored_to_root && !normalized.is_empty() {
        normalized.insert(0, '/');
    }
    normalized
}

fn normalize_relative_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    while normalized.starts_with('/') {
        normalized.remove(0);
    }
    let mut segments = Vec::new();
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }
    segments.join("/")
}

fn compile_pattern(pattern: &str) -> Result<Regex> {
    let (anchored_to_root, pattern) = split_root_anchor(pattern);
    if pattern.is_empty() {
        return Regex::new(r"^$").with_context(|| format!("compiling path pattern `{pattern}`"));
    }
    if is_literal_pattern(pattern) {
        let escaped = regex::escape(pattern);
        let prefix = if !anchored_to_root && is_basename_pattern(pattern) {
            "(?:.*/)?"
        } else {
            ""
        };
        let regex = format!("^{prefix}{escaped}(?:/.*)?$");
        return Regex::new(&regex).with_context(|| format!("compiling path pattern `{pattern}`"));
    }

    let mut regex = String::with_capacity(pattern.len() * 2 + 8);
    regex.push('^');
    if !anchored_to_root
        && (is_basename_pattern(pattern) || is_single_dir_descendant_pattern(pattern))
    {
        regex.push_str("(?:.*/)?");
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '*' {
            if index + 1 < chars.len() && chars[index + 1] == '*' {
                while index + 1 < chars.len() && chars[index + 1] == '*' {
                    index += 1;
                }
                if index + 1 < chars.len() && chars[index + 1] == '/' {
                    regex.push_str("(?:.*/)?");
                    index += 2;
                    continue;
                }
                regex.push_str(".*");
                index += 1;
                continue;
            }
            regex.push_str("[^/]*");
            index += 1;
            continue;
        }
        match chars[index] {
            '?' => regex.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                regex.push('\\');
                regex.push(chars[index]);
            }
            other => regex.push(other),
        }
        index += 1;
    }
    if should_match_folder_descendants(pattern) {
        regex.push_str("(?:/.*)?");
    }
    regex.push('$');
    Regex::new(&regex).with_context(|| format!("compiling path pattern `{pattern}`"))
}

fn split_root_anchor(pattern: &str) -> (bool, &str) {
    if let Some(stripped) = pattern.strip_prefix('/') {
        (true, stripped)
    } else {
        (false, pattern)
    }
}

fn is_literal_pattern(pattern: &str) -> bool {
    !pattern.contains('*') && !pattern.contains('?')
}

fn is_basename_pattern(pattern: &str) -> bool {
    !pattern.contains('/')
}

fn is_single_dir_descendant_pattern(pattern: &str) -> bool {
    pattern.ends_with("/**") && !pattern[..pattern.len().saturating_sub(3)].contains('/')
}

fn should_match_folder_descendants(pattern: &str) -> bool {
    if pattern.ends_with("/**") {
        return false;
    }
    let Some(last_segment) = pattern.rsplit('/').next() else {
        return false;
    };
    !last_segment.is_empty() && !last_segment.contains('*') && !last_segment.contains('?')
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
    use tempfile::tempdir;

    fn classifier_for(repo: &Path, paths: &[&str]) -> ProjectAwareClassifier {
        ProjectAwareClassifier::discover_for_worktree(repo, paths, "parser-v1", "extractor-v1")
            .expect("build classifier")
    }

    #[test]
    fn contextless_typescript_file_defaults_to_track_only() {
        let dir = tempdir().expect("temp dir");
        let classifier = classifier_for(dir.path(), &["src/main.ts"]);

        let classification = classifier
            .classify_repo_relative_path("src/main.ts", false)
            .expect("classify path");

        assert_eq!(classification.analysis_mode, AnalysisMode::TrackOnly);
        assert_eq!(classification.file_role, FileRole::SourceCode);
        assert_eq!(classification.text_index_mode, TextIndexMode::None);
        assert_eq!(classification.language, TRACK_ONLY_LANGUAGE_ID);
        assert_eq!(
            classification.classification_reason,
            "contextless_code_like"
        );
    }

    #[test]
    fn markdown_and_manifests_classify_as_text() {
        let dir = tempdir().expect("temp dir");
        let classifier = classifier_for(dir.path(), &["README.md", "Cargo.toml"]);

        let readme = classifier
            .classify_repo_relative_path("README.md", false)
            .expect("classify README");
        let cargo = classifier
            .classify_repo_relative_path("Cargo.toml", false)
            .expect("classify Cargo.toml");

        assert_eq!(readme.analysis_mode, AnalysisMode::Text);
        assert_eq!(cargo.analysis_mode, AnalysisMode::Text);
        assert_eq!(readme.file_role, FileRole::Documentation);
        assert_eq!(readme.text_index_mode, TextIndexMode::Embed);
        assert_eq!(cargo.file_role, FileRole::ProjectManifest);
        assert_eq!(cargo.text_index_mode, TextIndexMode::StoreOnly);
        assert_eq!(readme.language, PLAIN_TEXT_LANGUAGE_ID);
        assert_eq!(cargo.language, PLAIN_TEXT_LANGUAGE_ID);
    }

    #[test]
    fn typescript_context_activates_near_tsconfig() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("web/src")).expect("create web/src");
        fs::write(dir.path().join("web/tsconfig.json"), "{}").expect("write tsconfig");

        let classifier = classifier_for(dir.path(), &["web/tsconfig.json", "web/src/app.tsx"]);
        let classification = classifier
            .classify_repo_relative_path("web/src/app.tsx", false)
            .expect("classify tsx");

        assert_eq!(classification.analysis_mode, AnalysisMode::Code);
        assert_eq!(classification.file_role, FileRole::SourceCode);
        assert_eq!(classification.text_index_mode, TextIndexMode::None);
        assert_eq!(classification.language, "typescript");
        assert_eq!(classification.dialect.as_deref(), Some("tsx"));
        assert_eq!(
            classification.primary_context_id.as_deref(),
            Some("auto:typescript:web")
        );
    }

    #[test]
    fn scope_include_as_text_promotes_otherwise_track_only_paths() {
        let dir = tempdir().expect("temp dir");
        fs::write(
            dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
            r#"
[scope]
include_as_text = ["notes/**/*.foo"]
"#,
        )
        .expect("write policy");

        let classifier = classifier_for(dir.path(), &["notes/tmp/example.foo"]);
        let classification = classifier
            .classify_repo_relative_path("notes/tmp/example.foo", false)
            .expect("classify");

        assert_eq!(classification.analysis_mode, AnalysisMode::Text);
        assert_eq!(classification.file_role, FileRole::Configuration);
        assert_eq!(classification.text_index_mode, TextIndexMode::StoreOnly);
        assert_eq!(classification.classification_reason, "manual_text");
    }

    #[test]
    fn manual_context_promotes_contextless_code() {
        let dir = tempdir().expect("temp dir");
        fs::write(
            dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
            r#"
[[contexts]]
id = "scripts"
kind = "standalone"
root = "tools"
profile = "typescript-standard"
include = ["**/*.ts"]
"#,
        )
        .expect("write policy");
        fs::create_dir_all(dir.path().join("tools")).expect("create tools");

        let classifier = classifier_for(dir.path(), &["tools/run.ts"]);
        let classification = classifier
            .classify_repo_relative_path("tools/run.ts", false)
            .expect("classify");

        assert_eq!(classification.analysis_mode, AnalysisMode::Code);
        assert_eq!(classification.file_role, FileRole::SourceCode);
        assert_eq!(classification.text_index_mode, TextIndexMode::None);
        assert_eq!(
            classification.primary_context_id.as_deref(),
            Some("scripts")
        );
        assert_eq!(classification.language, "typescript");
    }

    #[test]
    fn auto_scope_project_root_and_include_constrain_auto_detection() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join("packages/app/src")).expect("create app/src");
        fs::create_dir_all(dir.path().join("packages/other/src")).expect("create other/src");
        fs::write(
            dir.path().join(REPO_POLICY_LOCAL_FILE_NAME),
            r#"
[scope]
project_root = "packages/app"
include = ["src/**"]
"#,
        )
        .expect("write policy");
        fs::write(dir.path().join("packages/app/tsconfig.json"), "{}").expect("write app tsconfig");
        fs::write(dir.path().join("packages/other/tsconfig.json"), "{}")
            .expect("write other tsconfig");

        let classifier = classifier_for(
            dir.path(),
            &[
                "packages/app/tsconfig.json",
                "packages/app/src/app.ts",
                "packages/other/tsconfig.json",
                "packages/other/src/skip.ts",
            ],
        );

        let allowed = classifier
            .classify_repo_relative_path("packages/app/src/app.ts", false)
            .expect("classify allowed");
        let denied = classifier
            .classify_repo_relative_path("packages/other/src/skip.ts", false)
            .expect("classify denied");

        assert_eq!(allowed.analysis_mode, AnalysisMode::Code);
        assert_eq!(denied.analysis_mode, AnalysisMode::TrackOnly);
    }
}
