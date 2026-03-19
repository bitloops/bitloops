use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::contexts::{CapabilityExecutionContext, CapabilityIngestContext};
use super::descriptor::CapabilityDescriptor;
use super::migrations::CapabilityMigration;
use super::health::CapabilityHealthCheck;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait CapabilityPack: Send + Sync {
    fn descriptor(&self) -> &'static CapabilityDescriptor;

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()>;

    fn migrations(&self) -> &'static [CapabilityMigration] {
        &[]
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        &[]
    }
}

pub trait CapabilityRegistrar {
    fn register_stage(&mut self, stage: StageRegistration) -> Result<()>;

    fn register_ingester(&mut self, ingester: IngesterRegistration) -> Result<()>;

    fn register_schema_module(&mut self, module: SchemaModule) -> Result<()>;

    fn register_query_examples(&mut self, examples: &'static [QueryExample]) -> Result<()>;
}

pub trait StageHandler: Send + Sync {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, Result<StageResponse>>;
}

pub trait IngesterHandler: Send + Sync {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageRequest {
    pub payload: Value,
}

impl StageRequest {
    pub fn new(payload: Value) -> Self {
        Self { payload }
    }

    pub fn parse_json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.payload.clone()).map_err(anyhow::Error::from)
    }

    pub fn limit(&self) -> Option<usize> {
        self.payload
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageResponse {
    pub payload: Value,
    pub human_output: String,
}

impl StageResponse {
    pub fn json(payload: Value) -> Self {
        let human_output =
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
        Self {
            payload,
            human_output,
        }
    }

    pub fn new(payload: Value, human_output: impl Into<String>) -> Self {
        Self {
            payload,
            human_output: human_output.into(),
        }
    }

    pub fn render_human(&self) -> String {
        self.human_output.clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IngestRequest {
    pub payload: Value,
}

impl IngestRequest {
    pub fn new(payload: Value) -> Self {
        Self { payload }
    }

    pub fn parse_json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.payload.clone()).map_err(anyhow::Error::from)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IngestResult {
    pub payload: Value,
    pub human_output: String,
}

impl IngestResult {
    pub fn json(payload: Value) -> Self {
        let human_output =
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
        Self {
            payload,
            human_output,
        }
    }

    pub fn new(payload: Value, human_output: impl Into<String>) -> Self {
        Self {
            payload,
            human_output: human_output.into(),
        }
    }

    pub fn render_human(&self) -> String {
        self.human_output.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaModule {
    pub capability_id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryExample {
    pub capability_id: &'static str,
    pub name: &'static str,
    pub query: &'static str,
    pub description: &'static str,
}

#[derive(Clone)]
pub struct StageRegistration {
    pub capability_id: &'static str,
    pub stage_name: &'static str,
    pub handler: Arc<dyn StageHandler>,
}

impl StageRegistration {
    pub fn new(
        capability_id: &'static str,
        stage_name: &'static str,
        handler: Arc<dyn StageHandler>,
    ) -> Self {
        Self {
            capability_id,
            stage_name,
            handler,
        }
    }
}

#[derive(Clone)]
pub struct IngesterRegistration {
    pub capability_id: &'static str,
    pub ingester_name: &'static str,
    pub handler: Arc<dyn IngesterHandler>,
}

impl IngesterRegistration {
    pub fn new(
        capability_id: &'static str,
        ingester_name: &'static str,
        handler: Arc<dyn IngesterHandler>,
    ) -> Self {
        Self {
            capability_id,
            ingester_name,
            handler,
        }
    }
}
