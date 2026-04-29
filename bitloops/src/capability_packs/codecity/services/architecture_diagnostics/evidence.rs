use super::*;

pub(super) fn build_dependency_evidence(
    source: &CodeCitySourceGraph,
    analysis: &CodeCityArchitectureAnalysis,
    repo_id: &str,
    run_id: &str,
    commit_sha: Option<String>,
) -> Vec<CodeCityDependencyEvidence> {
    let mut evidence = Vec::new();
    for edge in &source.edges {
        let from_assignment = analysis.zone_assignments.get(&edge.from_path);
        let to_assignment = analysis.zone_assignments.get(&edge.to_path);
        let from_boundary_id = from_assignment.map(|assignment| assignment.boundary_id.clone());
        let to_boundary_id = to_assignment.map(|assignment| assignment.boundary_id.clone());
        let cross_boundary = from_boundary_id.is_some()
            && to_boundary_id.is_some()
            && from_boundary_id != to_boundary_id;

        evidence.push(CodeCityDependencyEvidence {
            evidence_id: stable_id(
                "codecity-evidence",
                &[
                    repo_id,
                    edge.edge_id.as_str(),
                    edge.from_path.as_str(),
                    edge.to_path.as_str(),
                    edge.from_symbol_id.as_str(),
                    edge.to_symbol_id.as_deref().unwrap_or(""),
                    edge.to_symbol_ref.as_deref().unwrap_or(""),
                ],
            ),
            run_id: run_id.to_string(),
            commit_sha: commit_sha.clone(),
            from_path: edge.from_path.clone(),
            to_path: Some(edge.to_path.clone()),
            to_symbol_ref: edge.to_symbol_ref.clone(),
            from_boundary_id,
            to_boundary_id,
            from_zone: from_assignment.map(|assignment| assignment.zone.as_str().to_string()),
            to_zone: to_assignment.map(|assignment| assignment.zone.as_str().to_string()),
            from_symbol_id: Some(edge.from_symbol_id.clone()),
            from_artefact_id: Some(edge.from_artefact_id.clone()),
            to_symbol_id: edge.to_symbol_id.clone(),
            to_artefact_id: edge.to_artefact_id.clone(),
            edge_id: Some(edge.edge_id.clone()),
            edge_kind: edge.edge_kind.clone(),
            language: Some(edge.language.clone()),
            start_line: edge.start_line,
            end_line: edge.end_line,
            metadata_json: edge.metadata.clone(),
            resolved: true,
            cross_boundary,
        });
    }

    for (index, hint) in source.external_dependency_hints.iter().enumerate() {
        let from_assignment = analysis.zone_assignments.get(&hint.from_path);
        let index_string = index.to_string();
        evidence.push(CodeCityDependencyEvidence {
            evidence_id: stable_id(
                "codecity-evidence",
                &[
                    repo_id,
                    "external",
                    index_string.as_str(),
                    hint.from_path.as_str(),
                    hint.to_symbol_ref.as_deref().unwrap_or(""),
                    hint.edge_kind.as_str(),
                ],
            ),
            run_id: run_id.to_string(),
            commit_sha: commit_sha.clone(),
            from_path: hint.from_path.clone(),
            to_path: None,
            to_symbol_ref: hint.to_symbol_ref.clone(),
            from_boundary_id: from_assignment.map(|assignment| assignment.boundary_id.clone()),
            to_boundary_id: None,
            from_zone: from_assignment.map(|assignment| assignment.zone.as_str().to_string()),
            to_zone: None,
            from_symbol_id: None,
            from_artefact_id: None,
            to_symbol_id: None,
            to_artefact_id: None,
            edge_id: None,
            edge_kind: hint.edge_kind.clone(),
            language: None,
            start_line: None,
            end_line: None,
            metadata_json: hint.metadata.clone(),
            resolved: false,
            cross_boundary: false,
        });
    }

    evidence.sort_by(|left, right| left.evidence_id.cmp(&right.evidence_id));
    evidence
}

pub(super) fn aggregate_file_arcs(
    evidence: &[CodeCityDependencyEvidence],
    _repo_id: &str,
    run_id: &str,
) -> Vec<CodeCityFileDependencyArc> {
    let mut grouped = BTreeMap::<(String, String), Vec<&CodeCityDependencyEvidence>>::new();
    for row in evidence.iter().filter(|row| row.resolved) {
        let Some(to_path) = row.to_path.as_ref() else {
            continue;
        };
        if row.from_path == *to_path {
            continue;
        }
        grouped
            .entry((row.from_path.clone(), to_path.clone()))
            .or_default()
            .push(row);
    }

    grouped
        .into_iter()
        .map(|((from_path, to_path), rows)| {
            let mut import_count = 0usize;
            let mut call_count = 0usize;
            let mut reference_count = 0usize;
            let mut export_count = 0usize;
            let mut inheritance_count = 0usize;
            for row in &rows {
                match row.edge_kind.as_str() {
                    "imports" => import_count += 1,
                    "calls" => call_count += 1,
                    "exports" => export_count += 1,
                    "extends" | "implements" | "inherits" => inheritance_count += 1,
                    _ => reference_count += 1,
                }
            }
            let edge_count = rows.len();
            let weight = import_count as f64
                + call_count as f64 * 0.75
                + reference_count as f64 * 0.5
                + export_count as f64 * 1.5
                + inheritance_count as f64 * 1.25;
            let first = rows[0];
            CodeCityFileDependencyArc {
                arc_id: stable_id("codecity-file-arc", &[from_path.as_str(), to_path.as_str()]),
                run_id: run_id.to_string(),
                commit_sha: first.commit_sha.clone(),
                from_path,
                to_path,
                from_boundary_id: first.from_boundary_id.clone(),
                to_boundary_id: first.to_boundary_id.clone(),
                from_zone: first.from_zone.clone(),
                to_zone: first.to_zone.clone(),
                edge_count,
                import_count,
                call_count,
                reference_count,
                export_count,
                inheritance_count,
                weight,
                cross_boundary: rows.iter().any(|row| row.cross_boundary),
                has_violation: false,
                highest_severity: None,
                evidence_ids: rows
                    .iter()
                    .take(MAX_EVIDENCE_IDS_PER_ARC)
                    .map(|row| row.evidence_id.clone())
                    .collect(),
            }
        })
        .collect()
}
