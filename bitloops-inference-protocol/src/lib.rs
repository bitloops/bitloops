use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeRequest {
    Describe(DescribeRequest),
    Infer(InferRequest),
    Shutdown(ShutdownRequest),
}

impl RuntimeRequest {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Describe(request) => &request.request_id,
            Self::Infer(request) => &request.request_id,
            Self::Shutdown(request) => &request.request_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescribeRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferRequest {
    pub request_id: String,
    pub system_prompt: String,
    pub user_prompt: String,
    pub response_mode: ResponseMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    Text,
    JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownRequest {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeResponse {
    Describe(DescribeResponse),
    Infer(InferResponse),
    Shutdown(ShutdownResponse),
    Error(ErrorResponse),
}

impl RuntimeResponse {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Describe(response) => &response.request_id,
            Self::Infer(response) => &response.request_id,
            Self::Shutdown(response) => &response.request_id,
            Self::Error(response) => &response.request_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescribeResponse {
    pub request_id: String,
    pub protocol_version: u32,
    pub runtime_name: String,
    pub runtime_version: String,
    pub profile_name: String,
    pub provider: ProviderDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub kind: String,
    pub provider_name: String,
    pub model_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, deserialize_with = "deserialize_provider_capabilities")]
    pub capabilities: Vec<String>,
}

fn deserialize_provider_capabilities<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    enum ProviderCapabilitiesWire {
        Legacy(Vec<String>),
        Structured(StructuredProviderCapabilitiesWire),
    }

    #[derive(Debug, Deserialize)]
    struct StructuredProviderCapabilitiesWire {
        #[serde(default)]
        response_modes: Vec<ResponseMode>,
        #[serde(default)]
        usage_reporting: bool,
    }

    let capabilities = ProviderCapabilitiesWire::deserialize(deserializer)?;
    Ok(match capabilities {
        ProviderCapabilitiesWire::Legacy(capabilities) => capabilities,
        ProviderCapabilitiesWire::Structured(capabilities) => {
            let _ = capabilities.usage_reporting;
            capabilities
                .response_modes
                .into_iter()
                .map(|mode| match mode {
                    ResponseMode::Text => "text".to_string(),
                    ResponseMode::JsonObject => "json_object".to_string(),
                })
                .collect()
        }
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferResponse {
    pub request_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed_json: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    pub provider_name: String,
    pub model_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownResponse {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub request_id: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_response_accepts_legacy_provider_capabilities_array() {
        let payload = r#"{
            "type":"describe",
            "request_id":"text-generation-1",
            "protocol_version":1,
            "runtime_name":"bitloops-inference",
            "runtime_version":"0.1.1",
            "profile_name":"summary_local",
            "provider":{
                "kind":"ollama_chat",
                "provider_name":"ollama",
                "model_name":"ministral-3:3b",
                "endpoint":"http://127.0.0.1:11434/api/chat",
                "capabilities":["text","json_object"]
            }
        }"#;

        let response =
            serde_json::from_str::<RuntimeResponse>(payload).expect("legacy capabilities array");
        let RuntimeResponse::Describe(response) = response else {
            panic!("expected describe response");
        };

        assert_eq!(
            response.provider.capabilities,
            vec!["text".to_string(), "json_object".to_string()]
        );
    }

    #[test]
    fn describe_response_accepts_structured_provider_capabilities() {
        let payload = r#"{
            "type":"describe",
            "request_id":"text-generation-1",
            "protocol_version":1,
            "runtime_name":"bitloops-inference",
            "runtime_version":"0.1.1",
            "profile_name":"summary_local",
            "provider":{
                "kind":"ollama_chat",
                "provider_name":"ollama",
                "model_name":"ministral-3:3b",
                "endpoint":"http://127.0.0.1:11434/api/chat",
                "capabilities":{
                    "response_modes":["text","json_object"],
                    "usage_reporting":true
                }
            }
        }"#;

        let response = serde_json::from_str::<RuntimeResponse>(payload)
            .expect("structured capabilities should deserialize");
        let RuntimeResponse::Describe(response) = response else {
            panic!("expected describe response");
        };

        assert_eq!(response.profile_name, "summary_local");
        assert_eq!(response.provider.provider_name, "ollama");
        assert_eq!(response.provider.model_name, "ministral-3:3b");
        assert_eq!(
            response.provider.capabilities,
            vec!["text".to_string(), "json_object".to_string()]
        );
    }
}
