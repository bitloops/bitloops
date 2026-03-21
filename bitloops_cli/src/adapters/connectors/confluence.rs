use anyhow::{Context, Result, bail};
use regex::Regex;
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::OnceLock;

use crate::config::AtlassianProviderConfig;
use crate::engine::devql::capabilities::knowledge::{
    KnowledgeLocator, KnowledgePayloadData, ParsedKnowledgeUrl,
};

use super::types::{
    BoxFuture, ConnectorContext, ExternalKnowledgeRecord, KnowledgeConnectorAdapter,
};

pub struct ConfluenceKnowledgeAdapter {
    client: Client,
    api_base_url: Option<String>,
}

impl ConfluenceKnowledgeAdapter {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .build()
                .context("building Confluence knowledge HTTP client")?,
            api_base_url: None,
        })
    }
}

impl KnowledgeConnectorAdapter for ConfluenceKnowledgeAdapter {
    fn can_handle(&self, parsed: &ParsedKnowledgeUrl) -> bool {
        matches!(parsed.provider.as_str(), "confluence")
    }

    fn fetch<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        ctx: &'a dyn ConnectorContext,
    ) -> BoxFuture<'a, Result<ExternalKnowledgeRecord>> {
        Box::pin(async move {
            let confluence = confluence_config(ctx.provider_config()).ok_or_else(|| {
                anyhow::anyhow!(
                    "missing Atlassian configuration: expected `knowledge.providers.confluence` or `knowledge.providers.atlassian`"
                )
            })?;

            let (site, page_id): (&str, &str) = match &parsed.locator {
                KnowledgeLocator::ConfluencePage { site, page_id } => {
                    (site.as_str(), page_id.as_str())
                }
                _ => bail!("Confluence adapter received non-Confluence locator"),
            };

            if site.trim_end_matches('/') != confluence.site_url.trim_end_matches('/') {
                bail!(
                    "Confluence URL site `{}` does not match configured Atlassian site_url `{}`",
                    site,
                    confluence.site_url
                );
            }

            let base_url = self
                .api_base_url
                .as_deref()
                .unwrap_or(confluence.site_url.as_str())
                .trim_end_matches('/');
            let endpoint =
                format!("{base_url}/wiki/rest/api/content/{page_id}?expand=body.storage,version");
            let response = self
                .client
                .get(endpoint)
                .basic_auth(&confluence.email, Some(&confluence.token))
                .send()
                .await
                .context("sending Confluence knowledge request")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                bail!(
                    "Confluence knowledge fetch failed ({status}): {}",
                    body.trim()
                );
            }

            let payload: Value = response
                .json()
                .await
                .context("parsing Confluence knowledge response JSON")?;
            build_confluence_record(parsed, payload)
        })
    }
}

pub(crate) fn build_confluence_record(
    parsed: &ParsedKnowledgeUrl,
    payload: Value,
) -> Result<ExternalKnowledgeRecord> {
    let page_id = match &parsed.locator {
        KnowledgeLocator::ConfluencePage { page_id, .. } => page_id.as_str(),
        _ => bail!("Confluence adapter received non-Confluence locator"),
    };
    let title = payload
        .get("title")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("Confluence response missing `title`"))?;
    let body_html = payload
        .get("body")
        .and_then(Value::as_object)
        .and_then(|body| body.get("storage"))
        .and_then(Value::as_object)
        .and_then(|storage| storage.get("value"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let body_text = body_html.as_deref().map(strip_html_tags);
    let updated_at = payload
        .get("version")
        .and_then(Value::as_object)
        .and_then(|version| version.get("when"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let author = payload
        .get("version")
        .and_then(Value::as_object)
        .and_then(|version| version.get("by"))
        .and_then(Value::as_object)
        .and_then(|by| by.get("displayName"))
        .and_then(Value::as_str)
        .map(ToString::to_string);

    Ok(ExternalKnowledgeRecord {
        provider: "confluence".to_string(),
        source_kind: parsed.source_kind.as_str().to_string(),
        canonical_external_id: parsed.canonical_external_id.clone(),
        canonical_url: parsed.canonical_url.clone(),
        title: title.clone(),
        state: None,
        author: author.clone(),
        updated_at: updated_at.clone(),
        body_preview: body_text.clone(),
        normalized_fields: json!({
            "title": title,
            "author": author,
            "updated_at": updated_at,
            "web_url": parsed.canonical_url,
            "page_id": page_id,
        }),
        payload: KnowledgePayloadData {
            raw_payload: payload,
            body_text,
            body_html,
            body_adf: None,
            discussion: None,
        },
    })
}

#[cfg(test)]
pub(crate) fn build_confluence_document(
    parsed: &ParsedKnowledgeUrl,
    payload: Value,
) -> Result<crate::engine::devql::capabilities::knowledge::FetchedKnowledgeDocument> {
    Ok(build_confluence_record(parsed, payload)?.into())
}

fn confluence_config(
    provider_config: &crate::config::ProviderConfig,
) -> Option<&AtlassianProviderConfig> {
    provider_config
        .confluence
        .as_ref()
        .or(provider_config.atlassian.as_ref())
}

fn strip_html_tags(raw: &str) -> String {
    let collapsed = html_tag_regex().replace_all(raw, " ");
    collapsed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn html_tag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"<[^>]+>").expect("valid html strip regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_page() -> ParsedKnowledgeUrl {
        ParsedKnowledgeUrl {
            provider: crate::engine::devql::capabilities::knowledge::KnowledgeProvider::Confluence,
            source_kind:
                crate::engine::devql::capabilities::knowledge::KnowledgeSourceKind::ConfluencePage,
            canonical_external_id:
                "confluence://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548".to_string(),
            canonical_url:
                "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge"
                    .to_string(),
            provider_site: Some("https://bitloops.atlassian.net".to_string()),
            locator: KnowledgeLocator::ConfluencePage {
                site: "https://bitloops.atlassian.net".to_string(),
                page_id: "438337548".to_string(),
            },
        }
    }

    #[test]
    fn can_handle_only_confluence_urls() {
        let adapter = ConfluenceKnowledgeAdapter::new().expect("adapter");
        assert!(adapter.can_handle(&parsed_page()));
    }

    #[test]
    fn build_document_maps_page_payload() {
        let document = build_confluence_document(
            &parsed_page(),
            serde_json::json!({
                "title": " Knowledge page ",
                "version": {
                    "when": "2026-03-16T12:00:00Z",
                    "by": { "displayName": "Docs User" }
                },
                "body": {
                    "storage": {
                        "value": "<p>Hello <strong>world</strong></p>"
                    }
                }
            }),
        )
        .expect("document");

        assert_eq!(
            document.external_id,
            "confluence://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548"
        );
        assert_eq!(document.title, " Knowledge page ");
        assert_eq!(document.author.as_deref(), Some("Docs User"));
        assert_eq!(document.body_preview.as_deref(), Some("Hello world"));
        assert_eq!(document.payload.body_text.as_deref(), Some("Hello world"));
        assert_eq!(
            document.payload.body_html.as_deref(),
            Some("<p>Hello <strong>world</strong></p>")
        );
    }

    #[test]
    fn build_document_rejects_missing_title() {
        let err = build_confluence_document(
            &parsed_page(),
            serde_json::json!({
                "version": { "when": "2026-03-16T12:00:00Z" }
            }),
        )
        .expect_err("missing title must fail");

        assert!(err.to_string().contains("missing `title`"));
    }
}
