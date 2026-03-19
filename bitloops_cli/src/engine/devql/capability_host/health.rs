use anyhow::Result;

use super::contexts::CapabilityHealthContext;

#[derive(Debug, Clone, Copy)]
pub struct CapabilityHealthCheck {
    pub name: &'static str,
    pub run: fn(&dyn CapabilityHealthContext) -> CapabilityHealthResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityHealthResult {
    pub healthy: bool,
    pub message: String,
    pub details: Option<String>,
}

impl CapabilityHealthResult {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            healthy: true,
            message: message.into(),
            details: None,
        }
    }

    pub fn failed(message: impl Into<String>, details: impl Into<String>) -> Self {
        Self {
            healthy: false,
            message: message.into(),
            details: Some(details.into()),
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy
    }

    pub fn into_result(self) -> Result<()> {
        if self.healthy {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "{}",
                self.details.unwrap_or(self.message)
            ))
        }
    }
}
