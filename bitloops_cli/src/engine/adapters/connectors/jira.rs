use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::{Value, json};

use crate::engine::devql::capabilities::knowledge::{
    KnowledgeLocator, KnowledgePayloadData, ParsedKnowledgeUrl,
};
use crate::store_config::AtlassianProviderConfig;

use super::types::{BoxFuture, ConnectorContext, ExternalKnowledgeRecord, KnowledgeConnectorAdapter};

pub struct JiraKnowledgeAdapter {
    client: Client,
    api_base_url: Option<String>,
}

impl JiraKnowledgeAdapter {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .build()
                .context("building Jira knowledge HTTP client")?,
            api_base_url: None,
        })
    }
}

impl KnowledgeConnectorAdapter for JiraKnowledgeAdapter {
    fn can_handle(&self, parsed: &ParsedKnowledgeUrl) -> bool {
        matches!(parsed.provider.as_str(), "jira")
    }

    fn fetch<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        ctx: &'a dyn ConnectorContext,
    ) -> BoxFuture<'a, Result<ExternalKnowledgeRecord>> {
        Box::pin(async move {
            let jira = jira_config(ctx.provider_config()).ok_or_else(|| {
                anyhow::anyhow!(
                    "missing Atlassian configuration: expected `knowledge.providers.jira` or `knowledge.providers.atlassian`"
                )
            })?;

            let (site, key): (&str, &str) = match &parsed.locator {
                KnowledgeLocator::JiraIssue { site, key } => (site.as_str(), key.as_str()),
                _ => bail!("Jira adapter received non-Jira locator"),
            };

            if site.trim_end_matches('/') != jira.site_url.trim_end_matches('/') {
                bail!(
                    "Jira URL site `{}` does not match configured Atlassian site_url `{}`",
                    site,
                    jira.site_url
                );
            }

            let base_url = self
                .api_base_url
                .as_deref()
                .unwrap_or(jira.site_url.as_str())
                .trim_end_matches('/');
            let endpoint = format!("{base_url}/rest/api/3/issue/{key}");
            let response = self
                .client
                .get(endpoint)
                .basic_auth(&jira.email, Some(&jira.token))
                .send()
                .await
                .context("sending Jira knowledge request")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                bail!("Jira knowledge fetch failed ({status}): {}", body.trim());
            }

            let payload: Value = response
                .json()
                .await
                .context("parsing Jira knowledge response JSON")?;
            build_jira_record(parsed, payload)
        })
    }
}

pub(crate) fn build_jira_record(
    parsed: &ParsedKnowledgeUrl,
    payload: Value,
) -> Result<ExternalKnowledgeRecord> {
    let key = match &parsed.locator {
        KnowledgeLocator::JiraIssue { key, .. } => key.as_str(),
        _ => bail!("Jira adapter received non-Jira locator"),
    };
    let fields = payload
        .get("fields")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("Jira response missing `fields` object"))?;

    let title = fields
        .get("summary")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("Jira response missing `fields.summary`"))?;
    let state = fields
        .get("status")
        .and_then(Value::as_object)
        .and_then(|status| status.get("name"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let author = fields
        .get("reporter")
        .and_then(Value::as_object)
        .and_then(|reporter| reporter.get("displayName"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let updated_at = fields
        .get("updated")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let description = fields.get("description").cloned().unwrap_or(Value::Null);
    let body_text = collect_text_preview(&description);

    Ok(ExternalKnowledgeRecord {
        provider: "jira".to_string(),
        source_kind: parsed.source_kind.as_str().to_string(),
        canonical_external_id: parsed.canonical_external_id.clone(),
        canonical_url: parsed.canonical_url.clone(),
        title: title.clone(),
        state: state.clone(),
        author: author.clone(),
        updated_at: updated_at.clone(),
        body_preview: body_text.clone(),
        normalized_fields: json!({
            "key": key,
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
            body_adf: if description.is_null() {
                None
            } else {
                Some(description)
            },
            discussion: None,
        },
    })
}

fn jira_config(provider_config: &crate::store_config::ProviderConfig) -> Option<&AtlassianProviderConfig> {
    provider_config.jira.as_ref().or(provider_config.atlassian.as_ref())
}

fn collect_text_preview(value: &Value) -> Option<String> {
    let mut collected = String::new();
    collect_text(value, &mut collected);
    let trimmed = collected.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn collect_text(value: &Value, output: &mut String) {
    match value {
        Value::String(text) => {
            if !output.is_empty() {
                output.push(' ');
            }
            output.push_str(text);
        }
        Value::Array(values) => {
            for value in values {
                collect_text(value, output);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                if !output.is_empty() {
                    output.push(' ');
                }
                output.push_str(text);
            }
            if let Some(content) = map.get("content") {
                collect_text(content, output);
            }
        }
        Value::Bool(_) | Value::Null | Value::Number(_) => {}
    }
}
