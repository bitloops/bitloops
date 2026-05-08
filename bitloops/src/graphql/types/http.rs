use async_graphql::{InputObject, SimpleObject};

use super::JsonScalar;

#[derive(Debug, Clone, InputObject)]
pub struct HttpLossyTransformAroundInput {
    #[graphql(default)]
    pub symbol_fqn: Option<String>,
    #[graphql(default)]
    pub symbol_id: Option<String>,
    #[graphql(default)]
    pub artefact_id: Option<String>,
    #[graphql(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, InputObject)]
pub struct HttpPatchImpactInput {
    pub patch_fingerprint: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpConfidence {
    pub level: String,
    pub score: f64,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpEvidence {
    pub kind: String,
    pub path: Option<String>,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub content_id: Option<String>,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub dependency_package: Option<String>,
    pub dependency_version: Option<String>,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpPrimitive {
    pub id: String,
    pub owner: String,
    pub primitive_type: String,
    pub subject: String,
    pub roles: Vec<String>,
    pub terms: Vec<String>,
    pub status: String,
    pub confidence: HttpConfidence,
    pub evidence: Vec<HttpEvidence>,
    pub properties: JsonScalar,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpCausalChainLink {
    pub owner: String,
    pub fact_id: String,
    pub role: String,
    pub primitive_type: Option<String>,
    pub subject: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpUpstreamFact {
    pub owner: String,
    pub fact_id: String,
    pub primitive_type: Option<String>,
    pub subject: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpInvalidatedAssumption {
    pub id: String,
    pub assumption: String,
    pub invalidated_by_primitive_ids: Vec<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpPropagationObligation {
    pub id: String,
    pub required_follow_up: String,
    pub target_symbols: Vec<String>,
    pub blocking: bool,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpBundle {
    pub bundle_id: String,
    pub kind: String,
    pub risk_kind: Option<String>,
    pub severity: Option<String>,
    pub matched_roles: Vec<String>,
    pub status: String,
    pub confidence: HttpConfidence,
    pub upstream_facts: Vec<HttpUpstreamFact>,
    pub causal_chain: Vec<HttpCausalChainLink>,
    pub invalidated_assumptions: Vec<HttpInvalidatedAssumption>,
    pub obligations: Vec<HttpPropagationObligation>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpSearchResult {
    pub overview: JsonScalar,
    pub bundles: Vec<HttpBundle>,
    pub matched_facts: Vec<HttpPrimitive>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpContextResult {
    pub overview: JsonScalar,
    pub bundles: Vec<HttpBundle>,
    pub primitives: Vec<HttpPrimitive>,
    pub obligations: Vec<HttpPropagationObligation>,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpHeaderProducer {
    pub primitive_id: String,
    pub producer_kind: String,
    pub source_signal: Option<String>,
    pub phase: Option<String>,
    pub preconditions: Vec<String>,
    pub confidence: HttpConfidence,
}

#[derive(Debug, Clone, SimpleObject)]
pub struct HttpPatchImpactResult {
    pub patch_fingerprint: String,
    pub invalidated_assumptions: Vec<HttpInvalidatedAssumption>,
    pub propagation_obligations: Vec<HttpPropagationObligation>,
}
