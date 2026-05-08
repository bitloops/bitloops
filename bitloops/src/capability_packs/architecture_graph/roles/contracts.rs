use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjudicationReason {
    Unknown,
    LowConfidence,
    Conflict,
    HighImpact,
    NovelPattern,
    ManualReview,
}

impl AdjudicationReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::LowConfidence => "low_confidence",
            Self::Conflict => "conflict",
            Self::HighImpact => "high_impact",
            Self::NovelPattern => "novel_pattern",
            Self::ManualReview => "manual_review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdjudicationOutcome {
    Assigned,
    Unknown,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAdjudicationRequest {
    pub repo_id: String,
    pub generation: u64,
    #[serde(default)]
    pub target_kind: Option<String>,
    #[serde(default)]
    pub artefact_id: Option<String>,
    #[serde(default)]
    pub symbol_id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub canonical_kind: Option<String>,
    pub reason: AdjudicationReason,
    #[serde(default)]
    pub deterministic_confidence: Option<f64>,
    #[serde(default)]
    pub candidate_role_ids: Vec<String>,
    #[serde(default)]
    pub current_assignment: Option<RoleCurrentAssignmentSnapshot>,
}

impl RoleAdjudicationRequest {
    pub fn scope_key(&self) -> String {
        let target_kind = self.target_kind.as_deref().unwrap_or("target");
        let target = self
            .symbol_id
            .as_deref()
            .or(self.artefact_id.as_deref())
            .or(self.path.as_deref())
            .unwrap_or("<unknown>");
        format!(
            "{}:{}:{}:{}:{}",
            self.repo_id,
            self.generation,
            target_kind,
            target,
            self.reason.as_str()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleCurrentAssignmentSnapshot {
    pub role_id: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAdjudicationMailboxPayload {
    pub request: RoleAdjudicationRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAssignmentDecision {
    pub role_id: String,
    #[serde(default)]
    pub primary: bool,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAdjudicationRuleSuggestion {
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAdjudicationResult {
    pub outcome: AdjudicationOutcome,
    #[serde(default)]
    pub assignments: Vec<RoleAssignmentDecision>,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Value,
    pub reasoning_summary: String,
    #[serde(default)]
    pub rule_suggestions: Vec<RoleAdjudicationRuleSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleAdjudicationValidationError {
    Schema(String),
    UnknownRoleId(String),
    InvalidConfidence(String),
    InvalidOutcome(String),
}

impl std::fmt::Display for RoleAdjudicationValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Schema(msg) => write!(f, "schema validation failed: {msg}"),
            Self::UnknownRoleId(role_id) => {
                write!(
                    f,
                    "adjudication response references unknown role id `{role_id}`"
                )
            }
            Self::InvalidConfidence(msg) => write!(f, "invalid confidence: {msg}"),
            Self::InvalidOutcome(msg) => write!(f, "invalid adjudication outcome: {msg}"),
        }
    }
}

impl std::error::Error for RoleAdjudicationValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAdjudicationFailure {
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAdjudicationProvenance {
    pub source: String,
    pub model_descriptor: String,
    pub slot_name: String,
    pub packet_sha256: String,
    pub adjudication_reason: AdjudicationReason,
    pub adjudicated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoleAssignmentWriteEvent {
    pub request: RoleAdjudicationRequest,
    pub result: RoleAdjudicationResult,
    pub provenance: RoleAdjudicationProvenance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoleAssignmentWriteOutcome {
    pub source: &'static str,
    pub persisted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleQueueEnqueueResult {
    Enqueued,
    AlreadyQueued,
    AlreadyCompleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleQueueJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

pub trait RoleAdjudicationQueueStore: Send + Sync {
    fn enqueue(
        &self,
        request: &RoleAdjudicationRequest,
        dedupe_key: &str,
    ) -> Result<RoleQueueEnqueueResult>;

    fn claim(&self, dedupe_key: &str) -> Result<Option<RoleQueueJobStatus>>;

    fn complete(
        &self,
        dedupe_key: &str,
        result: &RoleAdjudicationResult,
        provenance: &RoleAdjudicationProvenance,
    ) -> Result<()>;

    fn fail(&self, dedupe_key: &str, failure: &RoleAdjudicationFailure) -> Result<()>;

    fn retry(&self, dedupe_key: &str) -> Result<bool>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSignalFact {
    pub rule_id: String,
    pub polarity: String,
    pub weight: f64,
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleFactsBundle {
    #[serde(default)]
    pub facts: Vec<Value>,
    #[serde(default)]
    pub rule_signals: Vec<RuleSignalFact>,
    #[serde(default)]
    pub dependency_context: Vec<Value>,
    #[serde(default)]
    pub related_artefacts: Vec<Value>,
    #[serde(default)]
    pub source_snippets: Vec<String>,
    #[serde(default)]
    pub reachability: Option<Value>,
}

pub type RoleBoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait RoleTaxonomyReader: Send + Sync {
    fn load_active_role_ids<'a>(
        &'a self,
        repo_id: &'a str,
        generation: u64,
    ) -> RoleBoxFuture<'a, BTreeSet<String>>;
}

pub trait RoleFactsReader: Send + Sync {
    fn load_facts<'a>(
        &'a self,
        request: &'a RoleAdjudicationRequest,
    ) -> RoleBoxFuture<'a, RoleFactsBundle>;
}

pub trait RoleAssignmentWriter: Send + Sync {
    fn apply_llm_assignment<'a>(
        &'a self,
        event: RoleAssignmentWriteEvent,
    ) -> RoleBoxFuture<'a, RoleAssignmentWriteOutcome>;

    fn mark_needs_review<'a>(
        &'a self,
        request: &'a RoleAdjudicationRequest,
        failure: &'a RoleAdjudicationFailure,
        provenance: &'a RoleAdjudicationProvenance,
    ) -> RoleBoxFuture<'a, RoleAssignmentWriteOutcome>;
}
