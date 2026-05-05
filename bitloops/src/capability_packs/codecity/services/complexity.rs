use std::collections::BTreeMap;

use super::source_graph::{CodeCitySourceArtefact, CodeCitySourceEdge};
use crate::capability_packs::codecity::types::MetricSource;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComplexityMetric {
    pub value: f64,
    pub source: MetricSource,
}

pub fn structural_complexity_by_symbol(
    artefacts: &[CodeCitySourceArtefact],
    edges: &[CodeCitySourceEdge],
) -> BTreeMap<String, ComplexityMetric> {
    let mut child_counts = BTreeMap::<String, usize>::new();
    for artefact in artefacts {
        if let Some(parent_symbol_id) = artefact.parent_symbol_id.as_deref() {
            *child_counts
                .entry(parent_symbol_id.to_string())
                .or_default() += 1;
        }
    }

    let mut call_counts = BTreeMap::<String, usize>::new();
    let mut reference_counts = BTreeMap::<String, usize>::new();
    for edge in edges {
        match edge.edge_kind.as_str() {
            "calls" => *call_counts.entry(edge.from_symbol_id.clone()).or_default() += 1,
            "references" => {
                *reference_counts
                    .entry(edge.from_symbol_id.clone())
                    .or_default() += 1
            }
            _ => {}
        }
    }

    artefacts
        .iter()
        .map(|artefact| {
            let calls = call_counts
                .get(&artefact.symbol_id)
                .copied()
                .unwrap_or_default() as f64;
            let references = reference_counts
                .get(&artefact.symbol_id)
                .copied()
                .unwrap_or_default() as f64;
            let children = child_counts
                .get(&artefact.symbol_id)
                .copied()
                .unwrap_or_default() as f64;
            (
                artefact.symbol_id.clone(),
                ComplexityMetric {
                    value: 1.0 + calls + references * 0.25 + children * 0.5,
                    source: MetricSource::StructuralProxy,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::structural_complexity_by_symbol;
    use crate::capability_packs::codecity::services::source_graph::{
        CodeCitySourceArtefact, CodeCitySourceEdge,
    };

    fn artefact(symbol_id: &str, parent_symbol_id: Option<&str>) -> CodeCitySourceArtefact {
        CodeCitySourceArtefact {
            artefact_id: format!("artefact:{symbol_id}"),
            symbol_id: symbol_id.to_string(),
            path: "src/lib.rs".to_string(),
            symbol_fqn: None,
            canonical_kind: Some("function".to_string()),
            language_kind: None,
            parent_artefact_id: None,
            parent_symbol_id: parent_symbol_id.map(str::to_string),
            signature: None,
            start_line: 1,
            end_line: 2,
        }
    }

    fn edge(from_symbol_id: &str, edge_kind: &str) -> CodeCitySourceEdge {
        CodeCitySourceEdge {
            edge_id: format!("edge:{from_symbol_id}:{edge_kind}"),
            from_path: "src/lib.rs".to_string(),
            to_path: "src/other.rs".to_string(),
            from_symbol_id: from_symbol_id.to_string(),
            from_artefact_id: format!("artefact:{from_symbol_id}"),
            to_symbol_id: None,
            to_artefact_id: None,
            to_symbol_ref: None,
            edge_kind: edge_kind.to_string(),
            language: "rust".to_string(),
            start_line: Some(1),
            end_line: Some(1),
            metadata: "{}".to_string(),
        }
    }

    #[test]
    fn structural_proxy_counts_edges_and_children() {
        let metrics = structural_complexity_by_symbol(
            &[artefact("parent", None), artefact("child", Some("parent"))],
            &[edge("parent", "calls"), edge("parent", "references")],
        );

        assert_eq!(metrics["parent"].value, 2.75);
        assert_eq!(metrics["child"].value, 1.0);
    }
}
