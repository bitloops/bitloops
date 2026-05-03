pub mod blob_payloads;
pub mod documents;
pub mod relational;
pub mod sqlite_relational;

use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

pub use crate::adapters::connectors::{
    ConnectorContext, ConnectorRegistry, ExternalKnowledgeRecord, KnowledgeConnectorAdapter,
};
use crate::host::language_adapter::{
    LanguageEntryPointArtefact, LanguageEntryPointCandidate, LanguageEntryPointFile,
    LanguageTestSupport,
};
pub use blob_payloads::{BlobPayloadGateway, BlobPayloadRef};
pub use documents::DocumentStoreGateway;
pub use relational::RelationalGateway;
pub use sqlite_relational::SqliteRelationalGateway;

pub trait CanonicalGraphGateway: Send + Sync {}

pub trait ProvenanceBuilder: Send + Sync {
    fn build(&self, capability_id: &str, operation: &str, details: Value) -> Value;
}

pub trait StoreHealthGateway: Send + Sync {
    fn check_relational(&self) -> Result<()>;
    fn check_documents(&self) -> Result<()>;
    fn check_blobs(&self) -> Result<()>;
}

pub trait LanguageServicesGateway: Send + Sync {
    fn test_supports(&self) -> Vec<Arc<dyn LanguageTestSupport>> {
        Vec::new()
    }

    fn resolve_test_support_for_path(
        &self,
        relative_path: &str,
    ) -> Option<Arc<dyn LanguageTestSupport>> {
        let _ = relative_path;
        None
    }

    fn entry_point_candidates_for_file(
        &self,
        file: &LanguageEntryPointFile,
        artefacts: &[LanguageEntryPointArtefact],
    ) -> Vec<LanguageEntryPointCandidate> {
        let _ = (file, artefacts);
        Vec::new()
    }
}

pub struct EmptyLanguageServicesGateway;

impl LanguageServicesGateway for EmptyLanguageServicesGateway {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHistoryRequest<'a> {
    pub paths: &'a [String],
    pub since_unix: i64,
    pub until_commit_sha: Option<&'a str>,
    pub bug_patterns: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHistoryEvent {
    pub path: String,
    pub commit_sha: String,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub committed_at_unix: i64,
    pub message: String,
    pub is_bug_fix: bool,
    pub changed_ranges: Vec<ChangedLineRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedLineRange {
    pub start_line: i64,
    pub end_line: i64,
}

pub trait GitHistoryGateway: Send + Sync {
    fn available(&self) -> bool {
        false
    }

    fn resolve_head(&self, _repo_root: &Path) -> Result<Option<String>> {
        Ok(None)
    }

    fn load_file_history(
        &self,
        _repo_root: &Path,
        _request: GitHistoryRequest<'_>,
    ) -> Result<Vec<FileHistoryEvent>> {
        Ok(Vec::new())
    }
}

pub struct EmptyGitHistoryGateway;

impl GitHistoryGateway for EmptyGitHistoryGateway {}

pub struct SymbolIdentityInput<'a> {
    pub path: &'a str,
    pub canonical_kind: &'a str,
    pub language_kind: &'a str,
    pub name: &'a str,
    pub parent_symbol_id: Option<&'a str>,
    pub signature: &'a str,
    pub modifiers: &'a [String],
}

pub trait HostServicesGateway: Send + Sync {
    fn derive_symbol_id(&self, input: &SymbolIdentityInput<'_>) -> String;

    fn derive_artefact_id(&self, content_id: &str, symbol_id: &str) -> String;

    fn derive_edge_id(
        &self,
        repo_id: &str,
        from_symbol_id: &str,
        edge_kind: &str,
        to_symbol_id_or_ref: &str,
    ) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityWorkplaneJob {
    pub mailbox_name: String,
    pub dedupe_key: Option<String>,
    pub payload: Value,
}

impl CapabilityWorkplaneJob {
    pub fn new(
        mailbox_name: impl Into<String>,
        dedupe_key: Option<String>,
        payload: Value,
    ) -> Self {
        Self {
            mailbox_name: mailbox_name.into(),
            dedupe_key,
            payload,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityWorkplaneEnqueueResult {
    pub inserted_jobs: u64,
    pub updated_jobs: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityMailboxStatus {
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
    pub pending_cursor_runs: u64,
    pub running_cursor_runs: u64,
    pub failed_cursor_runs: u64,
    pub completed_recent_cursor_runs: u64,
    pub intent_active: bool,
    pub blocked_reason: Option<String>,
}

pub trait CapabilityWorkplaneGateway: Send + Sync {
    fn enqueue_jobs(
        &self,
        jobs: Vec<CapabilityWorkplaneJob>,
    ) -> Result<CapabilityWorkplaneEnqueueResult>;

    fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>>;
}

pub struct DefaultHostServicesGateway {
    repo_id: String,
}

impl DefaultHostServicesGateway {
    pub fn new(repo_id: impl Into<String>) -> Self {
        Self {
            repo_id: repo_id.into(),
        }
    }
}

impl HostServicesGateway for DefaultHostServicesGateway {
    fn derive_symbol_id(&self, input: &SymbolIdentityInput<'_>) -> String {
        let normalized_signature =
            normalize_identity_fragment(&identity_signature(input.signature, input.modifiers));
        let semantic_name = if has_positional_identity_name(input.name) {
            normalized_signature.clone()
        } else {
            normalize_identity_fragment(input.name)
        };
        let canonical_kind = if input.canonical_kind.trim().is_empty() {
            "<null>"
        } else {
            input.canonical_kind
        };

        crate::host::devql::deterministic_uuid(&format!(
            "{}|{}|{}|{}|{}|{}",
            input.path,
            canonical_kind,
            input.language_kind,
            input.parent_symbol_id.unwrap_or(""),
            semantic_name,
            normalized_signature,
        ))
    }

    fn derive_artefact_id(&self, content_id: &str, symbol_id: &str) -> String {
        crate::host::devql::deterministic_uuid(&format!(
            "{}|{}|{}",
            self.repo_id, content_id, symbol_id
        ))
    }

    fn derive_edge_id(
        &self,
        repo_id: &str,
        from_symbol_id: &str,
        edge_kind: &str,
        to_symbol_id_or_ref: &str,
    ) -> String {
        crate::host::devql::deterministic_uuid(&format!(
            "{}|{}|{}|{}",
            repo_id, from_symbol_id, edge_kind, to_symbol_id_or_ref
        ))
    }
}

fn has_positional_identity_name(name: &str) -> bool {
    name.rsplit_once('@')
        .map(|(_, suffix)| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
        .unwrap_or(false)
}

fn normalize_identity_fragment(input: &str) -> String {
    let normalized = input
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if normalized.is_empty() {
        input.trim().to_string()
    } else {
        normalized
    }
}

fn identity_signature(signature: &str, modifiers: &[String]) -> String {
    let mut normalized_signature = signature.to_string();
    let mut filtered_modifiers = modifiers
        .iter()
        .filter(|modifier| !matches!(modifier.as_str(), "get" | "set"))
        .collect::<Vec<_>>();
    filtered_modifiers.sort_by_key(|modifier| std::cmp::Reverse(modifier.len()));

    for modifier in filtered_modifiers {
        let escaped = regex::escape(modifier);
        let pattern = Regex::new(&format!(r"(^|[\s(]){}($|[\s(])", escaped))
            .expect("modifier regex should compile");
        normalized_signature = pattern
            .replace_all(&normalized_signature, "$1$2")
            .to_string();
    }

    normalized_signature
}

#[cfg(test)]
mod tests {
    use super::{DefaultHostServicesGateway, HostServicesGateway, SymbolIdentityInput};

    #[test]
    fn derive_symbol_id_is_stable_for_identical_inputs() {
        let gateway = DefaultHostServicesGateway::new("repo-1");
        let modifiers = vec!["async".to_string(), "pub".to_string()];
        let input = SymbolIdentityInput {
            path: "src/user/service.rs",
            canonical_kind: "function",
            language_kind: "function_item",
            name: "create_user",
            parent_symbol_id: None,
            signature: "pub async fn create_user(name: &str) -> User",
            modifiers: &modifiers,
        };
        let first = gateway.derive_symbol_id(&input);
        let second = gateway.derive_symbol_id(&input);
        assert_eq!(first, second);
    }

    #[test]
    fn derive_artefact_id_depends_on_repo_content_and_symbol() {
        let gateway = DefaultHostServicesGateway::new("repo-1");
        let a = gateway.derive_artefact_id("blob-a", "symbol-1");
        let b = gateway.derive_artefact_id("blob-b", "symbol-1");
        assert_ne!(a, b);
    }
}
