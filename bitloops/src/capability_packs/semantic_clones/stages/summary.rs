use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

#[derive(Debug, Deserialize)]
struct CloneSummaryStagePayload {
    #[serde(default)]
    input_rows: Vec<Value>,
}

pub struct CloneSummaryStageHandler;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CloneSummaryGroup {
    relation_kind: String,
    count: i32,
}

fn summarize_clone_rows(rows: Vec<Value>) -> (i32, Vec<CloneSummaryGroup>) {
    let mut counts = BTreeMap::<String, usize>::new();

    for row in rows {
        let Some(relation_kind) = row
            .get("relation_kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        *counts.entry(relation_kind.to_string()).or_default() += 1;
    }

    let total_count = counts
        .values()
        .copied()
        .sum::<usize>()
        .try_into()
        .unwrap_or(i32::MAX);
    let mut groups = counts
        .into_iter()
        .map(|(relation_kind, count)| CloneSummaryGroup {
            relation_kind,
            count: count.try_into().unwrap_or(i32::MAX),
        })
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.relation_kind.cmp(&right.relation_kind))
    });

    (total_count, groups)
}

fn render_human_summary(total_count: i32, groups: &[CloneSummaryGroup]) -> String {
    if groups.is_empty() {
        return format!("clone summary: total_count={total_count}");
    }

    let rendered_groups = groups
        .iter()
        .map(|group| format!("{}={}", group.relation_kind, group.count))
        .collect::<Vec<_>>()
        .join(", ");
    format!("clone summary: total_count={total_count} ({rendered_groups})")
}

impl StageHandler for CloneSummaryStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        _ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, Result<StageResponse>> {
        Box::pin(async move {
            let payload: CloneSummaryStagePayload = request.parse_json()?;
            let (total_count, groups) = summarize_clone_rows(payload.input_rows);
            let human = render_human_summary(total_count, &groups);
            Ok(StageResponse::new(
                json!({
                    "total_count": total_count,
                    "groups": groups
                        .into_iter()
                        .map(|group| {
                            json!({
                                "relation_kind": group.relation_kind,
                                "count": group.count,
                            })
                        })
                        .collect::<Vec<_>>(),
                }),
                human,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::summarize_clone_rows;
    use serde_json::json;

    #[test]
    fn summarize_clone_rows_groups_and_sorts_relation_kinds() {
        let (total_count, groups) = summarize_clone_rows(vec![
            json!({ "relation_kind": "similar_implementation" }),
            json!({ "relation_kind": "contextual_neighbor" }),
            json!({ "relation_kind": "similar_implementation" }),
            json!({ "relation_kind": "" }),
        ]);

        assert_eq!(total_count, 3);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].relation_kind, "similar_implementation");
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[1].relation_kind, "contextual_neighbor");
        assert_eq!(groups[1].count, 1);
    }
}
