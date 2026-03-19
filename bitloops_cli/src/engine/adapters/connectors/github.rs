use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::{Value, json};

use crate::engine::devql::capabilities::knowledge::{
    KnowledgeLocator, KnowledgePayloadData, KnowledgeSourceKind, ParsedKnowledgeUrl,
};

use super::types::{BoxFuture, ConnectorContext, ExternalKnowledgeRecord, KnowledgeConnectorAdapter};

const GITHUB_ACCEPT_HEADER: &str = "application/vnd.github+json";
const GITHUB_USER_AGENT: &str = "bitloops-cli";

pub struct GitHubKnowledgeAdapter {
    client: Client,
    api_base_url: String,
}

impl GitHubKnowledgeAdapter {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .build()
                .context("building GitHub knowledge HTTP client")?,
            api_base_url: "https://api.github.com".to_string(),
        })
    }

    pub fn build_request(&self, endpoint: &str, token: &str) -> Result<reqwest::Request> {
        self.client
            .get(endpoint)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, GITHUB_ACCEPT_HEADER)
            .header(reqwest::header::USER_AGENT, GITHUB_USER_AGENT)
            .build()
            .context("building GitHub knowledge request")
    }
}

impl KnowledgeConnectorAdapter for GitHubKnowledgeAdapter {
    fn can_handle(&self, parsed: &ParsedKnowledgeUrl) -> bool {
        matches!(parsed.provider.as_str(), "github")
    }

    fn fetch<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        ctx: &'a dyn ConnectorContext,
    ) -> BoxFuture<'a, Result<ExternalKnowledgeRecord>> {
        Box::pin(async move {
            let github = ctx
                .provider_config()
                .github
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing `knowledge.providers.github` configuration"))?;

            let (owner, repo, number) = match &parsed.locator {
                KnowledgeLocator::GithubIssue { owner, repo, number }
                | KnowledgeLocator::GithubPullRequest { owner, repo, number } => {
                    (owner.as_str(), repo.as_str(), *number)
                }
                _ => bail!("GitHub adapter received non-GitHub locator"),
            };

            let endpoint = match parsed.source_kind.as_str() {
                "github_issue" => format!(
                    "{}/repos/{owner}/{repo}/issues/{number}",
                    self.api_base_url.trim_end_matches('/')
                ),
                "github_pull_request" => format!(
                    "{}/repos/{owner}/{repo}/pulls/{number}",
                    self.api_base_url.trim_end_matches('/')
                ),
                other => bail!("GitHub adapter received unsupported source kind `{other}`"),
            };

            let request = self.build_request(&endpoint, &github.token)?;
            let response = self
                .client
                .execute(request)
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

            build_github_record(parsed, payload)
        })
    }
}

pub(crate) fn build_github_record(
    parsed: &ParsedKnowledgeUrl,
    payload: Value,
) -> Result<ExternalKnowledgeRecord> {
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

    Ok(ExternalKnowledgeRecord {
        provider: "github".to_string(),
        source_kind: parsed.source_kind.as_str().to_string(),
        canonical_external_id: parsed.canonical_external_id.clone(),
        canonical_url: parsed.canonical_url.clone(),
        title: title.clone(),
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
            discussion: None,
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
