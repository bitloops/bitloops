use anyhow::{Context, Result, bail};
use regex::Regex;
use reqwest::Url;
use std::sync::OnceLock;

use super::types::{
    KnowledgeLocator, KnowledgeProvider, KnowledgeSourceKind, ParsedKnowledgeUrl,
};

pub fn parse_knowledge_url(raw: &str) -> Result<ParsedKnowledgeUrl> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("knowledge URL must not be empty");
    }

    let mut url = Url::parse(trimmed).with_context(|| format!("invalid knowledge URL `{trimmed}`"))?;
    url.set_query(None);
    url.set_fragment(None);

    if let Some(parsed) = parse_github_url(&url)? {
        return Ok(parsed);
    }
    if let Some(parsed) = parse_jira_url(&url)? {
        return Ok(parsed);
    }
    if let Some(parsed) = parse_confluence_url(&url)? {
        return Ok(parsed);
    }

    bail!("unsupported knowledge URL `{trimmed}`")
}

fn parse_github_url(url: &Url) -> Result<Option<ParsedKnowledgeUrl>> {
    let segments = path_segments(url);
    if segments.len() != 4 {
        return Ok(None);
    }

    let Some(number) = parse_u64(segments[3]) else {
        return Ok(None);
    };

    let owner = segments[0].to_string();
    let repo = segments[1].to_string();
    let canonical_url = normalized_url(url);

    match segments[2] {
        "issues" => Ok(Some(ParsedKnowledgeUrl {
            provider: KnowledgeProvider::Github,
            source_kind: KnowledgeSourceKind::GithubIssue,
            canonical_external_id: format!("github://{owner}/{repo}/issues/{number}"),
            canonical_url,
            provider_site: Some(origin_string(url)),
            locator: KnowledgeLocator::GithubIssue {
                owner,
                repo,
                number,
            },
        })),
        "pull" => Ok(Some(ParsedKnowledgeUrl {
            provider: KnowledgeProvider::Github,
            source_kind: KnowledgeSourceKind::GithubPullRequest,
            canonical_external_id: format!("github://{owner}/{repo}/pull/{number}"),
            canonical_url,
            provider_site: Some(origin_string(url)),
            locator: KnowledgeLocator::GithubPullRequest {
                owner,
                repo,
                number,
            },
        })),
        _ => Ok(None),
    }
}

fn parse_jira_url(url: &Url) -> Result<Option<ParsedKnowledgeUrl>> {
    let segments = path_segments(url);
    if segments.len() != 2 || segments[0] != "browse" {
        return Ok(None);
    }

    let key = segments[1].trim().to_ascii_uppercase();
    if !jira_key_regex().is_match(&key) {
        return Ok(None);
    }

    Ok(Some(ParsedKnowledgeUrl {
        provider: KnowledgeProvider::Jira,
        source_kind: KnowledgeSourceKind::JiraIssue,
        canonical_external_id: format!("jira://{}/{}", host_string(url), key),
        canonical_url: normalized_url(url),
        provider_site: Some(origin_string(url)),
        locator: KnowledgeLocator::JiraIssue {
            site: origin_string(url),
            key,
        },
    }))
}

fn parse_confluence_url(url: &Url) -> Result<Option<ParsedKnowledgeUrl>> {
    let segments = path_segments(url);
    let Some(pages_index) = segments.iter().position(|segment| *segment == "pages") else {
        return Ok(None);
    };
    let Some(page_id_raw) = segments.get(pages_index + 1) else {
        return Ok(None);
    };
    if !page_id_raw.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(None);
    }

    let page_id = (*page_id_raw).to_string();
    Ok(Some(ParsedKnowledgeUrl {
        provider: KnowledgeProvider::Confluence,
        source_kind: KnowledgeSourceKind::ConfluencePage,
        canonical_external_id: format!("confluence://{}/pages/{}", host_string(url), page_id),
        canonical_url: normalized_url(url),
        provider_site: Some(origin_string(url)),
        locator: KnowledgeLocator::ConfluencePage {
            site: origin_string(url),
            page_id,
        },
    }))
}

fn parse_u64(raw: &str) -> Option<u64> {
    raw.parse::<u64>().ok()
}

fn path_segments(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
        .unwrap_or_default()
}

fn normalized_url(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_query(None);
    normalized.set_fragment(None);
    let rendered = normalized.to_string();
    if normalized.path() != "/" {
        rendered.trim_end_matches('/').to_string()
    } else {
        rendered
    }
}

fn host_string(url: &Url) -> String {
    match url.port() {
        Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    }
}

fn origin_string(url: &Url) -> String {
    let mut origin = url.clone();
    origin.set_path("");
    origin.set_query(None);
    origin.set_fragment(None);
    origin.to_string().trim_end_matches('/').to_string()
}

fn jira_key_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^[A-Z][A-Z0-9_]*-\d+$").expect("valid jira key regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_issue_url() {
        let parsed = parse_knowledge_url("https://github.com/bitloops/bitloops/issues/42")
            .expect("github issue");
        assert_eq!(parsed.provider, KnowledgeProvider::Github);
        assert_eq!(parsed.source_kind, KnowledgeSourceKind::GithubIssue);
        assert_eq!(
            parsed.canonical_external_id,
            "github://bitloops/bitloops/issues/42"
        );
    }

    #[test]
    fn parses_github_pr_url_and_strips_query_fragment() {
        let parsed = parse_knowledge_url(
            "https://github.com/bitloops/bitloops/pull/137/?foo=bar#section",
        )
        .expect("github pull request");
        assert_eq!(parsed.source_kind, KnowledgeSourceKind::GithubPullRequest);
        assert_eq!(parsed.canonical_url, "https://github.com/bitloops/bitloops/pull/137");
    }

    #[test]
    fn parses_jira_issue_url() {
        let parsed = parse_knowledge_url("https://bitloops.atlassian.net/browse/CLI-1370")
            .expect("jira issue");
        assert_eq!(parsed.provider, KnowledgeProvider::Jira);
        assert_eq!(parsed.source_kind, KnowledgeSourceKind::JiraIssue);
        assert_eq!(parsed.canonical_external_id, "jira://bitloops.atlassian.net/CLI-1370");
    }

    #[test]
    fn parses_confluence_page_url() {
        let parsed = parse_knowledge_url(
            "https://bitloops.atlassian.net/wiki/spaces/ADCP/pages/438337548/Knowledge",
        )
        .expect("confluence page");
        assert_eq!(parsed.provider, KnowledgeProvider::Confluence);
        assert_eq!(parsed.source_kind, KnowledgeSourceKind::ConfluencePage);
        assert_eq!(
            parsed.canonical_external_id,
            "confluence://bitloops.atlassian.net/pages/438337548"
        );
    }

    #[test]
    fn rejects_unsupported_url() {
        let err = parse_knowledge_url("https://example.com/docs/page")
            .expect_err("unsupported url must fail");
        assert!(err.to_string().contains("unsupported knowledge URL"));
    }

    #[test]
    fn rejects_empty_url() {
        let err = parse_knowledge_url(" ").expect_err("empty url must fail");
        assert!(err.to_string().contains("must not be empty"));
    }
}
