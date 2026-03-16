use anyhow::{Context, Result, bail};
use regex::Regex;
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::OnceLock;

use super::KnowledgeProviderClient;
use crate::engine::devql::capabilities::knowledge::types::{
    BoxFuture, FetchedKnowledgeDocument, KnowledgeHostContext, KnowledgeLocator,
    KnowledgePayloadData, ParsedKnowledgeUrl,
};

pub struct ConfluenceKnowledgeClient {
    client: Client,
    api_base_url: Option<String>,
}

impl ConfluenceKnowledgeClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .build()
                .context("building Confluence knowledge HTTP client")?,
            api_base_url: None,
        })
    }
}

impl KnowledgeProviderClient for ConfluenceKnowledgeClient {
    fn fetch<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        host: &'a KnowledgeHostContext,
    ) -> BoxFuture<'a, Result<FetchedKnowledgeDocument>> {
        Box::pin(async move {
            let confluence = host.provider_config.confluence.as_ref().ok_or_else(|| {
                anyhow::anyhow!("missing `knowledge.providers.confluence` configuration")
            })?;

            let KnowledgeLocator::ConfluencePage { site, page_id } = &parsed.locator else {
                bail!("Confluence client received non-Confluence locator");
            };

            if site.trim_end_matches('/') != confluence.site_url.trim_end_matches('/') {
                bail!(
                    "Confluence URL site `{}` does not match configured knowledge.providers.confluence.site_url `{}`",
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
            build_confluence_document(parsed, payload)
        })
    }
}

pub(crate) fn build_confluence_document(
    parsed: &ParsedKnowledgeUrl,
    payload: Value,
) -> Result<FetchedKnowledgeDocument> {
    let KnowledgeLocator::ConfluencePage { page_id, .. } = &parsed.locator else {
        bail!("Confluence client received non-Confluence locator");
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

    Ok(FetchedKnowledgeDocument {
        external_id: parsed.canonical_external_id.clone(),
        title: title.clone(),
        web_url: parsed.canonical_url.clone(),
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
        },
    })
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
