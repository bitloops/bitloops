use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KnowledgeDiscussion {
    GitHubPullRequest {
        issue_comments: Vec<serde_json::Value>,
        reviews: Vec<serde_json::Value>,
        review_comments: Vec<serde_json::Value>,
    },
    GitHubIssue {
        issue_comments: Vec<serde_json::Value>,
        timeline: Vec<serde_json::Value>,
    },
    ConfluencePage {
        footer_comments: Vec<serde_json::Value>,
        inline_comments: Vec<serde_json::Value>,
    },
}
