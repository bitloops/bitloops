use axum::http::{HeaderMap, HeaderValue};
use serde_json::{Map, Value, json};
use std::env;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task_local;

pub(crate) const DEVQL_TIMINGS_ENV: &str = "BITLOOPS_DEVQL_TIMINGS";
pub(crate) const DEVQL_TIMINGS_HEADER: &str = "x-bitloops-devql-timings";
pub(crate) const DEVQL_TIMINGS_EXTENSION: &str = "bitloopsTimings";

task_local! {
    static CURRENT_TRACE: TimingTrace;
}

#[derive(Clone, Debug)]
pub(crate) struct TimingTrace {
    inner: Arc<TimingTraceInner>,
}

#[derive(Debug)]
struct TimingTraceInner {
    started_at: Instant,
    stages: Mutex<Vec<Map<String, Value>>>,
}

impl TimingTrace {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(TimingTraceInner {
                started_at: Instant::now(),
                stages: Mutex::new(Vec::new()),
            }),
        }
    }

    pub(crate) fn record(&self, stage: &str, elapsed: Duration, detail: Value) {
        let mut entry = Map::new();
        entry.insert("stage".to_string(), Value::String(stage.to_string()));
        entry.insert(
            "durationMs".to_string(),
            json!(elapsed.as_secs_f64() * 1000.0),
        );
        if let Value::Object(detail) = detail {
            for (key, value) in detail {
                entry.insert(key, value);
            }
        } else if !detail.is_null() {
            entry.insert("detail".to_string(), detail);
        }

        self.inner
            .stages
            .lock()
            .expect("timing trace mutex should not be poisoned")
            .push(entry);
    }

    pub(crate) fn summary_value(&self) -> Value {
        let stages = self
            .inner
            .stages
            .lock()
            .expect("timing trace mutex should not be poisoned")
            .clone();
        json!({
            "totalMs": self.inner.started_at.elapsed().as_secs_f64() * 1000.0,
            "stages": stages,
        })
    }
}

pub(crate) async fn scope_trace<T>(trace: TimingTrace, future: impl Future<Output = T>) -> T {
    CURRENT_TRACE.scope(trace, future).await
}

pub(crate) fn record_current_stage(stage: &str, elapsed: Duration, detail: Value) {
    let _ = CURRENT_TRACE.try_with(|trace| trace.record(stage, elapsed, detail));
}

pub(crate) fn timings_enabled_from_env() -> bool {
    parse_flag_value(env::var(DEVQL_TIMINGS_ENV).ok().as_deref())
}

pub(crate) fn timings_requested(headers: &HeaderMap) -> bool {
    headers
        .get(DEVQL_TIMINGS_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| parse_flag_value(Some(value)))
}

pub(crate) fn timing_header_value() -> HeaderValue {
    HeaderValue::from_static("1")
}

pub(crate) fn print_summary(label: &str, summary: &Value) {
    let Some(summary_obj) = summary.as_object() else {
        eprintln!("[bitloops][timing] {label} {summary}");
        return;
    };

    if let Some(total_ms) = summary_obj.get("totalMs").and_then(Value::as_f64) {
        eprintln!("[bitloops][timing] {label}.total {:.2}ms", total_ms);
    }

    if let Some(stages) = summary_obj.get("stages").and_then(Value::as_array) {
        for stage in stages {
            let Some(entry) = stage.as_object() else {
                continue;
            };
            let stage_name = entry
                .get("stage")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let duration_ms = entry
                .get("durationMs")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let detail = format_stage_detail(entry);
            if detail.is_empty() {
                eprintln!(
                    "[bitloops][timing] {label}.{stage_name} {:.2}ms",
                    duration_ms
                );
            } else {
                eprintln!(
                    "[bitloops][timing] {label}.{stage_name} {:.2}ms {}",
                    duration_ms, detail
                );
            }
        }
    }
}

fn format_stage_detail(entry: &Map<String, Value>) -> String {
    let mut parts = entry
        .iter()
        .filter(|(key, _)| key.as_str() != "stage" && key.as_str() != "durationMs")
        .map(|(key, value)| format!("{key}={}", render_detail_value(value)))
        .collect::<Vec<_>>();
    parts.sort();
    parts.join(" ")
}

fn render_detail_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "<json>".to_string())
        }
    }
}

fn parse_flag_value(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flag_value_treats_falsey_values_as_disabled() {
        assert!(!parse_flag_value(None));
        assert!(!parse_flag_value(Some("")));
        assert!(!parse_flag_value(Some("0")));
        assert!(!parse_flag_value(Some("false")));
        assert!(!parse_flag_value(Some("off")));
    }

    #[test]
    fn parse_flag_value_treats_truthy_values_as_enabled() {
        assert!(parse_flag_value(Some("1")));
        assert!(parse_flag_value(Some("true")));
        assert!(parse_flag_value(Some("yes")));
    }

    #[test]
    fn timing_trace_summary_contains_recorded_stages() {
        let trace = TimingTrace::new();
        trace.record(
            "server.db.sqlite.query_rows",
            Duration::from_millis(12),
            json!({
                "rows": 1,
                "pooled": true,
            }),
        );

        let summary = trace.summary_value();
        let stages = summary["stages"].as_array().expect("stages array");

        assert_eq!(stages.len(), 1);
        assert_eq!(
            stages[0]["stage"].as_str(),
            Some("server.db.sqlite.query_rows")
        );
        assert_eq!(stages[0]["rows"].as_u64(), Some(1));
        assert_eq!(stages[0]["pooled"].as_bool(), Some(true));
        assert!(summary["totalMs"].as_f64().is_some());
    }
}
