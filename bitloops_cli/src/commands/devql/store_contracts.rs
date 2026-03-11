#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::Value;

use crate::devql_config::{EventsProvider, RelationalProvider};

pub(crate) type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Shared relational contract for provider implementations.
/// The current runtime still uses Postgres directly; this trait is the stable
/// integration boundary for provider-specific tasks.
pub(crate) trait RelationalStore {
    fn provider(&self) -> RelationalProvider;
    fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()>;
    fn execute<'a>(&'a self, sql: &'a str) -> StoreFuture<'a, ()>;
    fn query_rows<'a>(&'a self, sql: &'a str) -> StoreFuture<'a, Vec<Value>>;
}

/// Shared events contract for provider implementations.
/// The current runtime still uses ClickHouse directly; this trait is the stable
/// integration boundary for provider-specific tasks.
pub(crate) trait EventsStore {
    fn provider(&self) -> EventsProvider;
    fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()>;
    fn execute<'a>(&'a self, sql: &'a str) -> StoreFuture<'a, String>;
    fn query_rows<'a>(&'a self, sql: &'a str) -> StoreFuture<'a, Vec<Value>>;
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

        fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn execute<'a>(&'a self, _sql: &'a str) -> StoreFuture<'a, String> {
            Box::pin(async { Ok(String::new()) })
        }

        fn query_rows<'a>(&'a self, _sql: &'a str) -> StoreFuture<'a, Vec<Value>> {
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
}
