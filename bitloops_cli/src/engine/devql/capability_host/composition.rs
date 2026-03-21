use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::engine::devql::{
    DevqlConfig, RegisteredStageCompositionContext, execute_query_json_with_composition,
};

use super::contexts::CapabilityExecutionContext;
use super::policy::with_timeout;
use super::registrar::StageRequest;

pub const DEFAULT_DEVQL_SUBQUERY_MAX_DEPTH: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevqlSubqueryOptions {
    pub caller_capability_id: &'static str,
    pub max_depth: usize,
    pub subquery_timeout: Duration,
}

impl DevqlSubqueryOptions {
    pub fn new(caller_capability_id: &'static str) -> Self {
        Self {
            caller_capability_id,
            max_depth: DEFAULT_DEVQL_SUBQUERY_MAX_DEPTH,
            subquery_timeout: Duration::from_secs(60),
        }
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth.max(1);
        self
    }

    pub fn with_subquery_timeout(mut self, subquery_timeout: Duration) -> Self {
        self.subquery_timeout = subquery_timeout;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InheritedCompositionContext {
    caller_capability_id: String,
    depth: usize,
    max_depth: usize,
}

pub async fn execute_devql_subquery(
    ctx: &mut dyn CapabilityExecutionContext,
    request: &StageRequest,
    query: &str,
    options: DevqlSubqueryOptions,
) -> Result<Value> {
    let inherited = inherited_composition_context(request);
    let current_depth = inherited.as_ref().map(|ctx| ctx.depth).unwrap_or(0);
    let max_depth = inherited
        .as_ref()
        .map(|ctx| ctx.max_depth)
        .unwrap_or(options.max_depth.max(1));
    let caller_capability_id = inherited
        .as_ref()
        .map(|ctx| ctx.caller_capability_id.as_str())
        .unwrap_or(options.caller_capability_id);
    let next_depth = current_depth.saturating_add(1);
    if next_depth > max_depth {
        bail!(
            "[capability_pack:{caller_capability_id}] [devql_subquery] depth {next_depth} exceeds max_depth {max_depth}"
        );
    }

    let cfg = DevqlConfig::from_env(ctx.repo_root().to_path_buf(), ctx.repo().clone())?;
    let composition = RegisteredStageCompositionContext {
        caller_capability_id: caller_capability_id.to_string(),
        depth: next_depth,
        max_depth,
    };
    let fut = execute_query_json_with_composition(&cfg, query, Some(composition));
    with_timeout("DevQL subquery", options.subquery_timeout, fut).await
}

fn inherited_composition_context(request: &StageRequest) -> Option<InheritedCompositionContext> {
    let composition = request.payload.get("query_context")?.get("composition")?;
    let caller_capability_id = composition
        .get("caller_capability_id")
        .and_then(Value::as_str)?
        .to_string();
    let depth = composition
        .get("depth")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())?;
    let max_depth = composition
        .get("max_depth")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(DEFAULT_DEVQL_SUBQUERY_MAX_DEPTH)
        .max(1);

    Some(InheritedCompositionContext {
        caller_capability_id,
        depth,
        max_depth,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn inherited_composition_context_reads_query_context_payload() {
        let request = StageRequest::new(json!({
            "query_context": {
                "composition": {
                    "caller_capability_id": "knowledge",
                    "depth": 2,
                    "max_depth": 4
                }
            }
        }));

        let parsed = inherited_composition_context(&request).expect("composition context");
        assert_eq!(parsed.caller_capability_id, "knowledge");
        assert_eq!(parsed.depth, 2);
        assert_eq!(parsed.max_depth, 4);
    }

    #[test]
    fn inherited_composition_context_returns_none_when_payload_is_missing() {
        let request =
            StageRequest::new(json!({ "query_context": { "resolved_commit_sha": "abc" } }));
        assert!(inherited_composition_context(&request).is_none());
    }
}
