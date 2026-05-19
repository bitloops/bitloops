use async_graphql::{Enum, ID, InputObject, SimpleObject};

use crate::daemon::{
    EmbeddingsBootstrapMode, InitEmbeddingsBootstrapRequest, StartInitSessionSelections,
    SummaryBootstrapAction, SummaryBootstrapRequest,
};

#[derive(Debug, Clone, InputObject)]
pub(crate) struct StartInitInput {
    #[graphql(name = "runSync")]
    pub run_sync: bool,
    #[graphql(name = "runIngest")]
    pub run_ingest: bool,
    #[graphql(name = "runCodeEmbeddings")]
    pub run_code_embeddings: bool,
    #[graphql(name = "runSummaries")]
    pub run_summaries: bool,
    #[graphql(name = "runSummaryEmbeddings")]
    pub run_summary_embeddings: bool,
    #[graphql(name = "ingestBackfill")]
    pub ingest_backfill: Option<i32>,
    #[graphql(name = "embeddingsBootstrap")]
    pub embeddings_bootstrap: Option<InitEmbeddingsBootstrapRequestInput>,
    #[graphql(name = "summariesBootstrap")]
    pub summaries_bootstrap: Option<SummaryBootstrapRequestInput>,
}

impl StartInitInput {
    pub(crate) fn into_selections(self) -> std::result::Result<StartInitSessionSelections, String> {
        if !self.run_ingest && self.ingest_backfill.is_some() {
            return Err("`ingestBackfill` requires `runIngest=true`".to_string());
        }
        if self.run_summary_embeddings && !self.run_summaries {
            return Err("`runSummaryEmbeddings` requires `runSummaries=true`".to_string());
        }
        if self.run_summary_embeddings && !self.run_code_embeddings {
            return Err("`runSummaryEmbeddings` requires `runCodeEmbeddings=true`".to_string());
        }
        Ok(StartInitSessionSelections {
            run_sync: self.run_sync,
            run_ingest: self.run_ingest,
            run_code_embeddings: self.run_code_embeddings,
            run_summaries: self.run_summaries,
            run_summary_embeddings: self.run_summary_embeddings,
            ingest_backfill: self
                .ingest_backfill
                .map(|value| usize::try_from(value.max(0)).unwrap_or(usize::MAX)),
            embeddings_bootstrap: self.embeddings_bootstrap.map(Into::into),
            summaries_bootstrap: self.summaries_bootstrap.map(Into::into),
        })
    }
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct InitEmbeddingsBootstrapRequestInput {
    #[graphql(name = "configPath")]
    pub config_path: String,
    #[graphql(name = "profileName")]
    pub profile_name: String,
    pub mode: Option<EmbeddingsBootstrapModeInput>,
    #[graphql(name = "gatewayUrlOverride")]
    pub gateway_url_override: Option<String>,
    #[graphql(name = "apiKeyEnv")]
    pub api_key_env: Option<String>,
}

impl From<InitEmbeddingsBootstrapRequestInput> for InitEmbeddingsBootstrapRequest {
    fn from(value: InitEmbeddingsBootstrapRequestInput) -> Self {
        Self {
            config_path: value.config_path.into(),
            profile_name: value.profile_name,
            mode: value.mode.map(Into::into).unwrap_or_default(),
            gateway_url_override: value.gateway_url_override,
            api_key_env: value.api_key_env,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub(crate) enum EmbeddingsBootstrapModeInput {
    Local,
    Platform,
}

impl From<EmbeddingsBootstrapModeInput> for EmbeddingsBootstrapMode {
    fn from(value: EmbeddingsBootstrapModeInput) -> Self {
        match value {
            EmbeddingsBootstrapModeInput::Local => Self::Local,
            EmbeddingsBootstrapModeInput::Platform => Self::Platform,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub(crate) enum SummaryBootstrapActionInput {
    InstallRuntimeOnly,
    InstallRuntimeOnlyPendingProbe,
    ConfigureLocal,
    ConfigureCloud,
}

impl From<SummaryBootstrapActionInput> for SummaryBootstrapAction {
    fn from(value: SummaryBootstrapActionInput) -> Self {
        match value {
            SummaryBootstrapActionInput::InstallRuntimeOnly => Self::InstallRuntimeOnly,
            SummaryBootstrapActionInput::InstallRuntimeOnlyPendingProbe => {
                Self::InstallRuntimeOnlyPendingProbe
            }
            SummaryBootstrapActionInput::ConfigureLocal => Self::ConfigureLocal,
            SummaryBootstrapActionInput::ConfigureCloud => Self::ConfigureCloud,
        }
    }
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct SummaryBootstrapRequestInput {
    pub action: SummaryBootstrapActionInput,
    pub message: Option<String>,
    #[graphql(name = "modelName")]
    pub model_name: Option<String>,
    #[graphql(name = "gatewayUrlOverride")]
    pub gateway_url_override: Option<String>,
    #[graphql(name = "apiKeyEnv")]
    pub api_key_env: Option<String>,
}

impl From<SummaryBootstrapRequestInput> for SummaryBootstrapRequest {
    fn from(value: SummaryBootstrapRequestInput) -> Self {
        Self {
            action: value.action.into(),
            message: value.message,
            model_name: value.model_name,
            gateway_url_override: value.gateway_url_override,
            api_key_env: value.api_key_env,
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct StartInitResult {
    #[graphql(name = "initSessionId")]
    pub init_session_id: ID,
}
