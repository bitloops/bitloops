use std::time::Duration;

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

/// Trust tier for a registered pack (e.g. first-party vs third-party). Reserved for future
/// stricter defaults when loading external packs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackTrustTier {
    #[default]
    FirstParty,
    #[allow(dead_code)]
    ThirdParty,
}

/// Wall-clock limits for host-orchestrated pack entrypoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInvocationPolicy {
    pub stage_timeout: Duration,
    pub ingester_timeout: Duration,
    pub subquery_timeout: Duration,
    pub trust_tier: PackTrustTier,
}

impl Default for HostInvocationPolicy {
    fn default() -> Self {
        Self {
            stage_timeout: Duration::from_secs(120),
            ingester_timeout: Duration::from_secs(300),
            subquery_timeout: Duration::from_secs(60),
            trust_tier: PackTrustTier::FirstParty,
        }
    }
}

#[derive(Debug, Deserialize)]
struct HostInvocationConfig {
    #[serde(default)]
    stage_timeout_secs: Option<u64>,
    #[serde(default)]
    ingester_timeout_secs: Option<u64>,
    #[serde(default)]
    subquery_timeout_secs: Option<u64>,
}

impl HostInvocationPolicy {
    pub fn from_config_root(root: &Value) -> Self {
        let mut policy = Self::default();
        let Some(host) = root.get("host") else {
            return policy;
        };
        let Some(inv) = host.get("invocation") else {
            return policy;
        };
        let Ok(cfg) = serde_json::from_value::<HostInvocationConfig>(inv.clone()) else {
            return policy;
        };
        if let Some(s) = cfg.stage_timeout_secs.filter(|&v| v > 0) {
            policy.stage_timeout = Duration::from_secs(s);
        }
        if let Some(s) = cfg.ingester_timeout_secs.filter(|&v| v > 0) {
            policy.ingester_timeout = Duration::from_secs(s);
        }
        if let Some(s) = cfg.subquery_timeout_secs.filter(|&v| v > 0) {
            policy.subquery_timeout = Duration::from_secs(s);
        }
        policy
    }
}

/// User-visible grant: allow `from_capability` read access to `to_capability`'s data for a logical resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossPackGrant {
    pub from_capability: String,
    pub to_capability: String,
    /// e.g. `devql_registered_stage` for registered-stage composition.
    pub resource: String,
}

#[derive(Debug, Deserialize)]
struct CrossPackGrantRaw {
    from_capability: String,
    to_capability: String,
    resource: String,
    #[serde(default)]
    mode: Option<String>,
}

/// Parsed `host.cross_pack_access` entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrossPackAccessPolicy {
    pub grants: Vec<CrossPackGrant>,
}

impl CrossPackAccessPolicy {
    pub const RESOURCE_DEVQL_REGISTERED_STAGE: &'static str = "devql_registered_stage";

    pub fn from_config_root(root: &Value) -> Self {
        let Some(host) = root.get("host") else {
            return Self::default();
        };
        let Some(arr) = host.get("cross_pack_access").and_then(Value::as_array) else {
            return Self::default();
        };
        let mut grants = Vec::new();
        for value in arr {
            let Ok(raw) = serde_json::from_value::<CrossPackGrantRaw>(value.clone()) else {
                continue;
            };
            let mode = raw.mode.as_deref().unwrap_or("read");
            if mode != "read" {
                continue;
            }
            grants.push(CrossPackGrant {
                from_capability: raw.from_capability,
                to_capability: raw.to_capability,
                resource: raw.resource,
            });
        }
        Self { grants }
    }

    pub fn allows_registered_stage_invocation(
        &self,
        from_capability: &str,
        to_capability: &str,
    ) -> bool {
        self.grants.iter().any(|g| {
            g.from_capability == from_capability
                && g.to_capability == to_capability
                && g.resource == Self::RESOURCE_DEVQL_REGISTERED_STAGE
        })
    }
}

pub async fn with_timeout<T, F>(label: &str, limit: Duration, fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(limit, fut).await {
        Ok(inner) => inner,
        Err(_) => bail!("[capability_host:timeout] {label} timed out after {limit:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_pack_grant_allows_registered_stage_when_configured() {
        let policy = CrossPackAccessPolicy {
            grants: vec![CrossPackGrant {
                from_capability: "test_harness".to_string(),
                to_capability: "knowledge".to_string(),
                resource: CrossPackAccessPolicy::RESOURCE_DEVQL_REGISTERED_STAGE.to_string(),
            }],
        };
        assert!(policy.allows_registered_stage_invocation("test_harness", "knowledge"));
        assert!(!policy.allows_registered_stage_invocation("knowledge", "test_harness"));
    }

    #[test]
    fn host_invocation_policy_reads_host_invocation_json() {
        let root = serde_json::json!({
            "host": {
                "invocation": {
                    "stage_timeout_secs": 7,
                    "ingester_timeout_secs": 8,
                    "subquery_timeout_secs": 9
                }
            }
        });
        let p = HostInvocationPolicy::from_config_root(&root);
        assert_eq!(p.stage_timeout, Duration::from_secs(7));
        assert_eq!(p.ingester_timeout, Duration::from_secs(8));
        assert_eq!(p.subquery_timeout, Duration::from_secs(9));
    }
}
