use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;
pub const RUNTIME_NAME: &str = "bitloops-embeddings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingInputType {
    #[default]
    Document,
    Query,
}

impl EmbeddingInputType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Query => "query",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingInput {
    pub id: Option<String>,
    pub text: String,
    #[serde(default)]
    pub input_type: EmbeddingInputType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub kind: String,
    pub provider_name: String,
    pub model_name: String,
    pub output_dimension: Option<usize>,
    pub cache_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDescriptor {
    pub protocol_version: u32,
    pub runtime_name: String,
    pub runtime_version: String,
    pub profile_name: String,
    pub provider: ProviderDescriptor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescribeRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescribeResponse {
    pub request_id: String,
    pub protocol_version: u32,
    pub runtime: RuntimeDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedBatchRequest {
    pub request_id: String,
    pub inputs: Vec<EmbeddingInput>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingVector {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedBatchResponse {
    pub request_id: String,
    pub protocol_version: u32,
    pub vectors: Vec<EmbeddingVector>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownResponse {
    pub request_id: String,
    pub protocol_version: u32,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Describe(DescribeRequest),
    EmbedBatch(EmbedBatchRequest),
    Shutdown(ShutdownRequest),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Describe(DescribeResponse),
    EmbedBatch(EmbedBatchResponse),
    Shutdown(ShutdownResponse),
    Error(ErrorResponse),
}

impl Request {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Describe(request) => &request.request_id,
            Self::EmbedBatch(request) => &request.request_id,
            Self::Shutdown(request) => &request.request_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_tag_and_request_id() {
        let json = serde_json::to_string(&Request::Describe(DescribeRequest {
            request_id: "req-1".to_string(),
        }))
        .expect("serialize request");

        assert!(json.contains(r#""type":"describe""#));
        assert!(json.contains(r#""request_id":"req-1""#));
    }

    #[test]
    fn response_round_trips_embed_batch_vectors() {
        let response = Response::EmbedBatch(EmbedBatchResponse {
            request_id: "req-2".to_string(),
            protocol_version: PROTOCOL_VERSION,
            vectors: vec![EmbeddingVector {
                index: 0,
                id: Some("symbol-1".to_string()),
                values: vec![0.1, 0.2, 0.3],
            }],
        });

        let json = serde_json::to_string(&response).expect("serialize response");
        let parsed: Response = serde_json::from_str(&json).expect("deserialize response");
        assert_eq!(parsed, response);
    }
}
