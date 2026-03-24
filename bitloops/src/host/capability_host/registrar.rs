use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Result, bail};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::contexts::{
    CapabilityExecutionContext, CapabilityIngestContext, KnowledgeExecutionContext,
    KnowledgeIngestContext,
};
use super::descriptor::CapabilityDescriptor;
use super::health::CapabilityHealthCheck;
use super::migrations::CapabilityMigration;

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

    fn register_knowledge_stage(&mut self, _stage: KnowledgeStageRegistration) -> Result<()> {
        bail!("knowledge stage registration is not supported by this registrar")
    }

    fn register_knowledge_ingester(
        &mut self,
        _ingester: KnowledgeIngesterRegistration,
    ) -> Result<()> {
        bail!("knowledge ingester registration is not supported by this registrar")
    }

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

pub trait KnowledgeStageHandler: Send + Sync {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn KnowledgeExecutionContext,
    ) -> BoxFuture<'a, Result<StageResponse>>;
}

pub trait KnowledgeIngesterHandler: Send + Sync {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn KnowledgeIngestContext,
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

#[derive(Clone)]
pub struct KnowledgeStageRegistration {
    pub capability_id: &'static str,
    pub stage_name: &'static str,
    pub handler: Arc<dyn KnowledgeStageHandler>,
}

impl KnowledgeStageRegistration {
    pub fn new(
        capability_id: &'static str,
        stage_name: &'static str,
        handler: Arc<dyn KnowledgeStageHandler>,
    ) -> Self {
        Self {
            capability_id,
            stage_name,
            handler,
        }
    }
}

#[derive(Clone)]
pub struct KnowledgeIngesterRegistration {
    pub capability_id: &'static str,
    pub ingester_name: &'static str,
    pub handler: Arc<dyn KnowledgeIngesterHandler>,
}

impl KnowledgeIngesterRegistration {
    pub fn new(
        capability_id: &'static str,
        ingester_name: &'static str,
        handler: Arc<dyn KnowledgeIngesterHandler>,
    ) -> Self {
        Self {
            capability_id,
            ingester_name,
            handler,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct HelperPayload {
        name: String,
        limit: Option<usize>,
    }

    #[test]
    fn stage_request_helpers_parse_and_limit() {
        let request = StageRequest::new(json!({ "name": "alpha", "limit": 7 }));

        assert_eq!(request.limit(), Some(7));
        let parsed: HelperPayload = request.parse_json().expect("parse stage request");
        assert_eq!(
            parsed,
            HelperPayload {
                name: "alpha".to_string(),
                limit: Some(7),
            }
        );
    }

    #[test]
    fn stage_request_parse_json_rejects_wrong_shape() {
        let request = StageRequest::new(json!({ "limit": "not-a-number" }));

        let err = request
            .parse_json::<HelperPayload>()
            .expect_err("invalid stage payload must fail");

        assert!(err.to_string().contains("invalid type"));
    }

    #[test]
    fn stage_response_and_ingest_result_render_helpers() {
        let stage = StageResponse::json(json!({ "ok": true }));
        assert!(stage.render_human().contains("\"ok\": true"));

        let ingest = IngestResult::new(json!({ "created": true }), "created");
        assert_eq!(ingest.render_human(), "created");
        assert_eq!(ingest.payload, json!({ "created": true }));
    }

    #[test]
    fn ingest_request_parse_json_and_result_json_roundtrip() {
        let request = IngestRequest::new(json!({ "name": "beta", "limit": 3 }));
        let parsed: HelperPayload = request.parse_json().expect("parse ingest request");

        assert_eq!(
            parsed,
            HelperPayload {
                name: "beta".to_string(),
                limit: Some(3),
            }
        );

        let result = IngestResult::json(json!({ "status": "ok" }));
        assert!(result.render_human().contains("\"status\": \"ok\""));
    }

    #[test]
    fn registration_constructors_store_values() {
        struct DummyStageHandler;
        impl StageHandler for DummyStageHandler {
            fn execute<'a>(
                &'a self,
                _request: StageRequest,
                _ctx: &'a mut dyn CapabilityExecutionContext,
            ) -> BoxFuture<'a, Result<StageResponse>> {
                Box::pin(async move { Ok(StageResponse::new(json!({}), "")) })
            }
        }

        struct DummyIngesterHandler;
        impl IngesterHandler for DummyIngesterHandler {
            fn ingest<'a>(
                &'a self,
                _request: IngestRequest,
                _ctx: &'a mut dyn CapabilityIngestContext,
            ) -> BoxFuture<'a, Result<IngestResult>> {
                Box::pin(async move { Ok(IngestResult::new(json!({}), "")) })
            }
        }

        struct DummyKnowledgeStageHandler;
        impl KnowledgeStageHandler for DummyKnowledgeStageHandler {
            fn execute<'a>(
                &'a self,
                _request: StageRequest,
                _ctx: &'a mut dyn KnowledgeExecutionContext,
            ) -> BoxFuture<'a, Result<StageResponse>> {
                Box::pin(async move { Ok(StageResponse::new(json!({}), "")) })
            }
        }

        struct DummyKnowledgeIngesterHandler;
        impl KnowledgeIngesterHandler for DummyKnowledgeIngesterHandler {
            fn ingest<'a>(
                &'a self,
                _request: IngestRequest,
                _ctx: &'a mut dyn KnowledgeIngestContext,
            ) -> BoxFuture<'a, Result<IngestResult>> {
                Box::pin(async move { Ok(IngestResult::new(json!({}), "")) })
            }
        }

        let stage =
            StageRegistration::new("knowledge", "knowledge.stage", Arc::new(DummyStageHandler));
        let ingester = IngesterRegistration::new(
            "knowledge",
            "knowledge.ingest",
            Arc::new(DummyIngesterHandler),
        );
        let knowledge_stage = KnowledgeStageRegistration::new(
            "knowledge",
            "knowledge.stage",
            Arc::new(DummyKnowledgeStageHandler),
        );
        let knowledge_ingester = KnowledgeIngesterRegistration::new(
            "knowledge",
            "knowledge.ingest",
            Arc::new(DummyKnowledgeIngesterHandler),
        );

        assert_eq!(stage.capability_id, "knowledge");
        assert_eq!(stage.stage_name, "knowledge.stage");
        assert_eq!(ingester.capability_id, "knowledge");
        assert_eq!(ingester.ingester_name, "knowledge.ingest");
        assert_eq!(knowledge_stage.capability_id, "knowledge");
        assert_eq!(knowledge_stage.stage_name, "knowledge.stage");
        assert_eq!(knowledge_ingester.capability_id, "knowledge");
        assert_eq!(knowledge_ingester.ingester_name, "knowledge.ingest");
    }
}
