#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendHealthKind {
    Ok,
    Skip,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackendHealth {
    pub(crate) kind: BackendHealthKind,
    pub(crate) detail: String,
}

impl BackendHealth {
    pub(super) fn ok(detail: impl Into<String>) -> Self {
        Self {
            kind: BackendHealthKind::Ok,
            detail: detail.into(),
        }
    }

    pub(super) fn skip(detail: impl Into<String>) -> Self {
        Self {
            kind: BackendHealthKind::Skip,
            detail: detail.into(),
        }
    }

    pub(super) fn fail(detail: impl Into<String>) -> Self {
        Self {
            kind: BackendHealthKind::Fail,
            detail: detail.into(),
        }
    }

    pub(crate) fn status_label(&self) -> &'static str {
        match self.kind {
            BackendHealthKind::Ok => "OK",
            BackendHealthKind::Skip => "SKIP",
            BackendHealthKind::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DashboardDbHealth {
    pub(crate) relational: BackendHealth,
    pub(crate) events: BackendHealth,
    pub(crate) postgres: BackendHealth,
    pub(crate) clickhouse: BackendHealth,
}

impl DashboardDbHealth {
    pub(super) fn with_compat_fields(
        relational: BackendHealth,
        events: BackendHealth,
        has_postgres: bool,
        has_clickhouse: bool,
    ) -> Self {
        let postgres = if has_postgres {
            relational.clone()
        } else {
            BackendHealth::skip("inactive compatibility key (relational: sqlite)")
        };
        let clickhouse = if has_clickhouse {
            events.clone()
        } else {
            BackendHealth::skip("inactive compatibility key (events: duckdb)")
        };

        Self {
            relational,
            events,
            postgres,
            clickhouse,
        }
    }

    pub(crate) fn has_failures(&self) -> bool {
        self.relational.kind == BackendHealthKind::Fail
            || self.events.kind == BackendHealthKind::Fail
    }
}
