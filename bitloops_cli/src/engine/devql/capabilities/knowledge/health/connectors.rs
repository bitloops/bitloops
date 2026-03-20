use crate::engine::devql::capability_host::{CapabilityHealthContext, CapabilityHealthResult};

pub fn check_knowledge_connectors(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    let config = match ctx.config_view("knowledge") {
        Ok(view) => view,
        Err(err) => {
            return CapabilityHealthResult::failed(
                "knowledge connector config unavailable",
                err.to_string(),
            );
        }
    };

    let has_provider_config = config
        .scoped()
        .or_else(|| config.root().get("knowledge"))
        .and_then(|knowledge| knowledge.get("providers"))
        .and_then(serde_json::Value::as_object)
        .map(|providers| {
            ["github", "jira", "confluence", "atlassian"]
                .iter()
                .any(|key| providers.get(*key).is_some_and(|value| !value.is_null()))
        })
        .unwrap_or(false);

    if !has_provider_config {
        return CapabilityHealthResult::failed(
            "knowledge connectors not configured",
            "no knowledge provider configuration found".to_string(),
        );
    }

    let _ = ctx.connectors();
    CapabilityHealthResult::ok("knowledge connectors configured")
}
