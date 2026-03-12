#![allow(dead_code)]

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::Value;

use crate::devql_config::{EventsProvider, RelationalProvider};

pub(crate) type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Shared relational contract for provider implementations.
/// This trait is the stable integration boundary for provider-specific tasks.
pub(crate) trait RelationalStore {
    fn provider(&self) -> RelationalProvider;
    fn ping<'a>(&'a self) -> StoreFuture<'a, i32>;
    fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()>;
    fn execute<'a>(&'a self, sql: &'a str) -> StoreFuture<'a, ()>;
    fn query_rows<'a>(&'a self, sql: &'a str) -> StoreFuture<'a, Vec<Value>>;
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckpointEventWrite {
    pub event_id: String,
    pub repo_id: String,
    pub checkpoint_id: String,
    pub session_id: String,
    pub commit_sha: String,
    pub commit_unix: Option<i64>,
    pub branch: String,
    pub event_type: String,
    pub agent: String,
    pub strategy: String,
    pub files_touched: Vec<String>,
    pub created_at: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventsCheckpointQuery {
    pub repo_id: String,
    pub agent: Option<String>,
    pub since: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventsTelemetryQuery {
    pub repo_id: String,
    pub event_type: Option<String>,
    pub agent: Option<String>,
    pub since: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventsCommitShaQuery {
    pub repo_id: String,
    pub agent: Option<String>,
    pub since: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventsCheckpointHistoryQuery {
    pub repo_id: String,
    pub commit_shas: Vec<String>,
    pub path_candidates: Vec<String>,
    pub limit: usize,
}

/// Shared events contract for provider implementations.
/// This is the stable integration boundary for provider-specific tasks.
pub(crate) trait EventsStore {
    fn provider(&self) -> EventsProvider;
    fn ping<'a>(&'a self) -> StoreFuture<'a, i32>;
    fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()>;
    fn existing_event_ids<'a>(&'a self, repo_id: String) -> StoreFuture<'a, HashSet<String>>;
    fn insert_checkpoint_event<'a>(&'a self, event: CheckpointEventWrite) -> StoreFuture<'a, ()>;
    fn query_checkpoints<'a>(&'a self, query: EventsCheckpointQuery)
    -> StoreFuture<'a, Vec<Value>>;
    fn query_telemetry<'a>(&'a self, query: EventsTelemetryQuery) -> StoreFuture<'a, Vec<Value>>;
    fn query_commit_shas<'a>(&'a self, query: EventsCommitShaQuery)
    -> StoreFuture<'a, Vec<String>>;
    fn query_checkpoint_events<'a>(
        &'a self,
        query: EventsCheckpointHistoryQuery,
    ) -> StoreFuture<'a, Vec<Value>>;
}

/// Reusable provider-contract harness shape for backend implementations.
pub(crate) struct ProviderContractHarness<'a> {
    pub(crate) relational: &'a dyn RelationalStore,
    pub(crate) events: &'a dyn EventsStore,
}

impl<'a> ProviderContractHarness<'a> {
    pub(crate) fn provider_pair(&self) -> (RelationalProvider, EventsProvider) {
        (self.relational.provider(), self.events.provider())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRelational {
        provider: RelationalProvider,
    }

    impl RelationalStore for FakeRelational {
        fn provider(&self) -> RelationalProvider {
            self.provider
        }

        fn ping<'a>(&'a self) -> StoreFuture<'a, i32> {
            Box::pin(async { Ok(1) })
        }

        fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn execute<'a>(&'a self, _sql: &'a str) -> StoreFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn query_rows<'a>(&'a self, _sql: &'a str) -> StoreFuture<'a, Vec<Value>> {
            Box::pin(async { Ok(vec![]) })
        }
    }

    struct FakeEvents {
        provider: EventsProvider,
    }

    impl EventsStore for FakeEvents {
        fn provider(&self) -> EventsProvider {
            self.provider
        }

        fn ping<'a>(&'a self) -> StoreFuture<'a, i32> {
            Box::pin(async { Ok(1) })
        }

        fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn existing_event_ids<'a>(&'a self, _repo_id: String) -> StoreFuture<'a, HashSet<String>> {
            Box::pin(async { Ok(HashSet::new()) })
        }

        fn insert_checkpoint_event<'a>(
            &'a self,
            _event: CheckpointEventWrite,
        ) -> StoreFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn query_checkpoints<'a>(
            &'a self,
            _query: EventsCheckpointQuery,
        ) -> StoreFuture<'a, Vec<Value>> {
            Box::pin(async { Ok(vec![]) })
        }

        fn query_telemetry<'a>(
            &'a self,
            _query: EventsTelemetryQuery,
        ) -> StoreFuture<'a, Vec<Value>> {
            Box::pin(async { Ok(vec![]) })
        }

        fn query_commit_shas<'a>(
            &'a self,
            _query: EventsCommitShaQuery,
        ) -> StoreFuture<'a, Vec<String>> {
            Box::pin(async { Ok(vec![]) })
        }

        fn query_checkpoint_events<'a>(
            &'a self,
            _query: EventsCheckpointHistoryQuery,
        ) -> StoreFuture<'a, Vec<Value>> {
            Box::pin(async { Ok(vec![]) })
        }
    }

    #[test]
    fn provider_contract_harness_reports_active_providers() {
        let relational = FakeRelational {
            provider: RelationalProvider::Postgres,
        };
        let events = FakeEvents {
            provider: EventsProvider::ClickHouse,
        };
        let harness = ProviderContractHarness {
            relational: &relational,
            events: &events,
        };

        assert_eq!(
            harness.provider_pair(),
            (RelationalProvider::Postgres, EventsProvider::ClickHouse)
        );
    }

    #[tokio::test]
    async fn fake_relational_methods_are_callable() {
        let relational = FakeRelational {
            provider: RelationalProvider::Sqlite,
        };

        assert_eq!(relational.provider(), RelationalProvider::Sqlite);
        assert_eq!(relational.ping().await.expect("ping"), 1);
        relational
            .init_schema()
            .await
            .expect("init schema should succeed");
        relational
            .execute("SELECT 1")
            .await
            .expect("execute should succeed");
        let rows = relational
            .query_rows("SELECT 1")
            .await
            .expect("query rows should succeed");
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn fake_events_methods_are_callable() {
        let events = FakeEvents {
            provider: EventsProvider::DuckDb,
        };

        assert_eq!(events.provider(), EventsProvider::DuckDb);
        assert_eq!(events.ping().await.expect("ping"), 1);
        events
            .init_schema()
            .await
            .expect("init schema should succeed");

        let existing = events
            .existing_event_ids("repo-1".to_string())
            .await
            .expect("existing ids");
        assert!(existing.is_empty());

        let event = CheckpointEventWrite {
            event_id: "evt-1".to_string(),
            repo_id: "repo-1".to_string(),
            checkpoint_id: "cp-1".to_string(),
            session_id: "session-1".to_string(),
            commit_sha: "sha-1".to_string(),
            commit_unix: Some(1_741_211_200),
            branch: "main".to_string(),
            event_type: "checkpoint_committed".to_string(),
            agent: "claude-code".to_string(),
            strategy: "manual-commit".to_string(),
            files_touched: vec!["src/main.rs".to_string()],
            created_at: Some("2026-03-01T12:00:00Z".to_string()),
            payload: serde_json::json!({"ok": true}),
        };
        events
            .insert_checkpoint_event(event)
            .await
            .expect("insert event should succeed");

        let checkpoints = events
            .query_checkpoints(EventsCheckpointQuery {
                repo_id: "repo-1".to_string(),
                agent: Some("claude-code".to_string()),
                since: Some("2026-03-01".to_string()),
                limit: 10,
            })
            .await
            .expect("query checkpoints");
        assert!(checkpoints.is_empty());

        let telemetry = events
            .query_telemetry(EventsTelemetryQuery {
                repo_id: "repo-1".to_string(),
                event_type: Some("checkpoint_committed".to_string()),
                agent: Some("claude-code".to_string()),
                since: Some("2026-03-01".to_string()),
                limit: 10,
            })
            .await
            .expect("query telemetry");
        assert!(telemetry.is_empty());

        let commit_shas = events
            .query_commit_shas(EventsCommitShaQuery {
                repo_id: "repo-1".to_string(),
                agent: Some("claude-code".to_string()),
                since: Some("2026-03-01".to_string()),
                limit: 10,
            })
            .await
            .expect("query commit shas");
        assert!(commit_shas.is_empty());

        let history = events
            .query_checkpoint_events(EventsCheckpointHistoryQuery {
                repo_id: "repo-1".to_string(),
                commit_shas: vec!["sha-1".to_string()],
                path_candidates: vec!["src/main.rs".to_string()],
                limit: 10,
            })
            .await
            .expect("query checkpoint events");
        assert!(history.is_empty());
    }

    #[test]
    fn query_payload_structs_are_cloneable_and_equatable() {
        let checkpoints = EventsCheckpointQuery {
            repo_id: "repo-1".to_string(),
            agent: Some("claude-code".to_string()),
            since: Some("2026-03-01".to_string()),
            limit: 25,
        };
        assert_eq!(checkpoints, checkpoints.clone());

        let telemetry = EventsTelemetryQuery {
            repo_id: "repo-1".to_string(),
            event_type: Some("checkpoint_committed".to_string()),
            agent: None,
            since: None,
            limit: 50,
        };
        assert_eq!(telemetry, telemetry.clone());

        let commit_shas = EventsCommitShaQuery {
            repo_id: "repo-1".to_string(),
            agent: None,
            since: Some("2026-01-01".to_string()),
            limit: 100,
        };
        assert_eq!(commit_shas, commit_shas.clone());

        let history = EventsCheckpointHistoryQuery {
            repo_id: "repo-1".to_string(),
            commit_shas: vec!["sha-1".to_string(), "sha-2".to_string()],
            path_candidates: vec!["src/main.rs".to_string()],
            limit: 200,
        };
        assert_eq!(history, history.clone());
    }
}
