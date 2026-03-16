use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::{Value, json};

use super::KnowledgeProviderClient;
use crate::engine::devql::capabilities::knowledge::types::{
    BoxFuture, FetchedKnowledgeDocument, KnowledgeHostContext, KnowledgeLocator,
    KnowledgePayloadData, KnowledgeSourceKind, ParsedKnowledgeUrl,
};

const GITHUB_ACCEPT_HEADER: &str = "application/vnd.github+json";

pub struct GitHubKnowledgeClient {
    client: Client,
    api_base_url: String,
}

impl GitHubKnowledgeClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .build()
                .context("building GitHub knowledge HTTP client")?,
            api_base_url: "https://api.github.com".to_string(),
        })
    }
}

impl KnowledgeProviderClient for GitHubKnowledgeClient {
    fn fetch<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        host: &'a KnowledgeHostContext,
    ) -> BoxFuture<'a, Result<FetchedKnowledgeDocument>> {
        Box::pin(async move {
            let github = host
                .provider_config
                .github
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing `providers.github` configuration"))?;

            let endpoint = match &parsed.locator {
                KnowledgeLocator::GithubIssue {
                    owner,
                    repo,
                    number,
                } => format!(
                    "{}/repos/{owner}/{repo}/issues/{number}",
                    self.api_base_url.trim_end_matches('/')
                ),
                KnowledgeLocator::GithubPullRequest {
                    owner,
                    repo,
                    number,
                } => format!(
                    "{}/repos/{owner}/{repo}/pulls/{number}",
                    self.api_base_url.trim_end_matches('/')
                ),
                _ => bail!("GitHub client received non-GitHub locator"),
            };

            let response = self
                .client
                .get(endpoint)
                .bearer_auth(&github.token)
                .header(reqwest::header::ACCEPT, GITHUB_ACCEPT_HEADER)
                .send()
                .await
                .context("sending GitHub knowledge request")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                bail!("GitHub knowledge fetch failed ({status}): {}", body.trim());
            }

            let payload: Value = response
                .json()
                .await
                .context("parsing GitHub knowledge response JSON")?;

            build_github_document(parsed, payload)
        })
    }
}

pub(crate) fn build_github_document(
    parsed: &ParsedKnowledgeUrl,
    payload: Value,
) -> Result<FetchedKnowledgeDocument> {
    if parsed.source_kind == KnowledgeSourceKind::GithubIssue
        && payload
            .get("pull_request")
            .and_then(Value::as_object)
            .is_some()
    {
        bail!("GitHub issue URL resolved to a pull request payload");
    }

    let title = required_string(&payload, "title")?;
    let state = optional_string(&payload, "state");
    let author = payload
        .get("user")
        .and_then(Value::as_object)
        .and_then(|user| user.get("login"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let updated_at = optional_string(&payload, "updated_at");
    let body_text = optional_string(&payload, "body");

    Ok(FetchedKnowledgeDocument {
        external_id: parsed.canonical_external_id.clone(),
        title: title.clone(),
        web_url: parsed.canonical_url.clone(),
        state: state.clone(),
        author: author.clone(),
        updated_at: updated_at.clone(),
        body_preview: preview_text(body_text.as_deref()),
        normalized_fields: json!({
            "title": title,
            "state": state,
            "author": author,
            "updated_at": updated_at,
            "web_url": parsed.canonical_url,
        }),
        payload: KnowledgePayloadData {
            raw_payload: payload,
            body_text,
            body_html: None,
            body_adf: None,
        },
    })
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn required_string(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("GitHub response missing `{key}`"))
}

fn preview_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}
