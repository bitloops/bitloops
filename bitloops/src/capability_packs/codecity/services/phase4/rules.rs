use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate_violations(
    source: &CodeCitySourceGraph,
    analysis: &CodeCityArchitectureAnalysis,
    file_arcs: &[CodeCityFileDependencyArc],
    evidence: &[CodeCityDependencyEvidence],
    buildings: &BTreeMap<String, &CodeCityBuilding>,
    boundaries: &BoundariesById<'_>,
    reports: &BoundaryReports<'_>,
    run_id: &str,
    world: &CodeCityWorldPayload,
    config: &CodeCityConfig,
    diagnostics: &mut Vec<CodeCityDiagnostic>,
) -> Vec<CodeCityArchitectureViolation> {
    let evidence_by_id = evidence
        .iter()
        .map(|row| (row.evidence_id.clone(), row))
        .collect::<BTreeMap<_, _>>();
    let rule_context = RuleEvaluationContext {
        evidence_by_id: &evidence_by_id,
        reports,
        boundaries,
        run_id,
        world,
        config,
    };
    let mut violations = Vec::new();
    violations.extend(evaluate_layered_rules(file_arcs, &rule_context));
    violations.extend(evaluate_hexagonal_rules(file_arcs, evidence, &rule_context));
    violations.extend(evaluate_modular_rules(
        source,
        analysis,
        file_arcs,
        &rule_context,
    ));
    violations.extend(evaluate_event_driven_rules(
        source,
        analysis,
        file_arcs,
        &rule_context,
        diagnostics,
    ));
    if config.violations.include_cross_boundary_rules {
        violations.extend(evaluate_cross_boundary_rules(
            analysis,
            file_arcs,
            &rule_context,
        ));
    }

    if violations.is_empty() && buildings.is_empty() {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.phase4.no_buildings".to_string(),
            severity: "info".to_string(),
            message: "CodeCity Phase 4 found no building geometry to evaluate.".to_string(),
            path: None,
            boundary_id: None,
        });
    }

    violations
}

fn evaluate_layered_rules(
    file_arcs: &[CodeCityFileDependencyArc],
    ctx: &RuleEvaluationContext<'_, '_>,
) -> Vec<CodeCityArchitectureViolation> {
    let mut violations = Vec::new();
    for arc in file_arcs.iter().filter(|arc| !arc.cross_boundary) {
        let Some(boundary_id) = arc.from_boundary_id.as_ref() else {
            continue;
        };
        if !boundary_has_pattern(
            boundary_id,
            CodeCityArchitecturePattern::Layered,
            ctx.reports,
            ctx.config,
        ) {
            continue;
        }
        let Some(from_rank) = layer_rank(arc.from_zone.as_deref()) else {
            continue;
        };
        let Some(to_rank) = layer_rank(arc.to_zone.as_deref()) else {
            continue;
        };
        let (rule, severity, message, explanation, recommendation) = if to_rank < from_rank {
            (
                CodeCityViolationRule::LayeredUpwardDependency,
                CodeCityViolationSeverity::High,
                format!(
                    "{} depends upward on {}, violating the detected layered architecture.",
                    arc.from_path, arc.to_path
                ),
                "The boundary is classified as layered. Dependencies should flow from edge/application code towards deeper core/periphery code, not back towards user-facing code.".to_string(),
                Some("Move shared contracts inward or invert the dependency through an application/core port.".to_string()),
            )
        } else if to_rank.saturating_sub(from_rank) > 1 {
            (
                CodeCityViolationRule::LayeredSkippedLayer,
                CodeCityViolationSeverity::Medium,
                format!(
                    "{} skips an intermediate layer and depends directly on {}.",
                    arc.from_path, arc.to_path
                ),
                "The dependency crosses more than one layer boundary in a detected layered architecture.".to_string(),
                Some("Route the dependency through the adjacent layer or introduce a narrower boundary contract.".to_string()),
            )
        } else {
            continue;
        };
        violations.push(violation_for_arc(
            arc,
            boundary_id,
            ArcViolationSpec {
                pattern: CodeCityViolationPattern::Layered,
                rule,
                severity,
                id_discriminator: None,
                message,
                explanation,
                recommendation,
            },
            ctx,
        ));
    }
    violations
}

fn evaluate_hexagonal_rules(
    file_arcs: &[CodeCityFileDependencyArc],
    evidence: &[CodeCityDependencyEvidence],
    ctx: &RuleEvaluationContext<'_, '_>,
) -> Vec<CodeCityArchitectureViolation> {
    let mut violations = Vec::new();
    for arc in file_arcs.iter().filter(|arc| !arc.cross_boundary) {
        let Some(boundary_id) = arc.from_boundary_id.as_ref() else {
            continue;
        };
        if !boundary_has_pattern(
            boundary_id,
            CodeCityArchitecturePattern::Hexagonal,
            ctx.reports,
            ctx.config,
        ) {
            continue;
        }
        let from_zone = normalise_hex_zone(arc.from_zone.as_deref());
        let to_zone = normalise_hex_zone(arc.to_zone.as_deref());
        let Some((rule, severity, message, explanation, recommendation)) =
            hexagonal_file_rule(&arc.from_path, &arc.to_path, from_zone, to_zone)
        else {
            continue;
        };
        violations.push(violation_for_arc(
            arc,
            boundary_id,
            ArcViolationSpec {
                pattern: CodeCityViolationPattern::Hexagonal,
                rule,
                severity,
                id_discriminator: None,
                message,
                explanation,
                recommendation,
            },
            ctx,
        ));
    }

    for row in evidence.iter().filter(|row| !row.resolved) {
        let Some(boundary_id) = row.from_boundary_id.as_ref() else {
            continue;
        };
        if !boundary_has_pattern(
            boundary_id,
            CodeCityArchitecturePattern::Hexagonal,
            ctx.reports,
            ctx.config,
        ) || normalise_hex_zone(row.from_zone.as_deref()) != Some(CodeCityZone::Core)
            || !is_conservative_external_ref(row.to_symbol_ref.as_deref(), &row.metadata_json)
        {
            continue;
        }
        let evidence_ids = vec![row.evidence_id.clone()];
        let evidence_rows = evidence_ids
            .iter()
            .filter_map(|id| {
                ctx.evidence_by_id
                    .get(id)
                    .map(|row| violation_evidence(row))
            })
            .collect::<Vec<_>>();
        let to_ref = row
            .to_symbol_ref
            .clone()
            .unwrap_or_else(|| "external dependency".to_string());
        violations.push(CodeCityArchitectureViolation {
            id: stable_id(
                "codecity-violation",
                &[
                    ctx.world.repo_id.as_str(),
                    CodeCityViolationPattern::Hexagonal.as_str(),
                    CodeCityViolationRule::HexagonalCoreImportsExternal.as_str(),
                    row.from_path.as_str(),
                    to_ref.as_str(),
                ],
            ),
            run_id: ctx.run_id.to_string(),
            commit_sha: ctx.world.commit_sha.clone(),
            boundary_id: Some(boundary_id.clone()),
            boundary_root: ctx
                .boundaries
                .get(boundary_id)
                .map(|boundary| boundary.root_path.clone()),
            pattern: CodeCityViolationPattern::Hexagonal,
            rule: CodeCityViolationRule::HexagonalCoreImportsExternal,
            severity: CodeCityViolationSeverity::High,
            from_path: row.from_path.clone(),
            to_path: None,
            from_zone: row.from_zone.clone(),
            to_zone: None,
            from_boundary_id: row.from_boundary_id.clone(),
            to_boundary_id: None,
            arc_id: None,
            message: format!(
                "Core file {} imports external package {}.",
                row.from_path, to_ref
            ),
            explanation: "The boundary is classified as hexagonal. Core code should not depend directly on external packages; adapters or ports should isolate those dependencies.".to_string(),
            recommendation: Some(
                "Move the external integration behind a port or adapter outside the core zone."
                    .to_string(),
            ),
            evidence_ids,
            evidence: evidence_rows,
            confidence: 0.8,
            suppressed: false,
        });
    }

    violations
}

fn evaluate_modular_rules(
    source: &CodeCitySourceGraph,
    analysis: &CodeCityArchitectureAnalysis,
    file_arcs: &[CodeCityFileDependencyArc],
    ctx: &RuleEvaluationContext<'_, '_>,
) -> Vec<CodeCityArchitectureViolation> {
    let communities = communities_by_boundary(source, analysis, ctx.config);
    let mut violations = Vec::new();
    let mut touched = BTreeMap::<String, BTreeSet<String>>::new();

    for arc in file_arcs.iter().filter(|arc| !arc.cross_boundary) {
        let Some(boundary_id) = arc.from_boundary_id.as_ref() else {
            continue;
        };
        if !boundary_has_pattern(
            boundary_id,
            CodeCityArchitecturePattern::Modular,
            ctx.reports,
            ctx.config,
        ) {
            continue;
        }
        let Some(community_by_path) = communities.get(boundary_id) else {
            continue;
        };
        let Some(from_community) = community_by_path.get(&arc.from_path) else {
            continue;
        };
        let Some(to_community) = community_by_path.get(&arc.to_path) else {
            continue;
        };
        if from_community == to_community {
            continue;
        }
        touched
            .entry(arc.from_path.clone())
            .or_default()
            .insert(to_community.clone());
        touched
            .entry(arc.to_path.clone())
            .or_default()
            .insert(from_community.clone());
        if is_facade_file(&arc.to_path) {
            continue;
        }
        violations.push(violation_for_arc(
            arc,
            boundary_id,
            ArcViolationSpec {
                pattern: CodeCityViolationPattern::Modular,
                rule: CodeCityViolationRule::ModularInternalCrossModuleDependency,
                severity: CodeCityViolationSeverity::High,
                id_discriminator: None,
                message: format!(
                    "{} depends on internal file {} across module boundaries.",
                    arc.from_path, arc.to_path
                ),
                explanation: "The boundary is classified as modular, and this dependency crosses detected communities without targeting a public facade.".to_string(),
                recommendation: Some("Depend on a public module facade or move the shared contract into an explicit shared boundary.".to_string()),
            },
            ctx,
        ));
    }

    for (path, modules) in touched {
        if modules.len() < ctx.config.violations.modular_bridge_module_threshold
            || is_facade_file(&path)
        {
            continue;
        }
        let Some(arc) = file_arcs
            .iter()
            .find(|arc| arc.from_path == path || arc.to_path == path)
        else {
            continue;
        };
        let boundary_id = arc
            .from_boundary_id
            .as_ref()
            .or(arc.to_boundary_id.as_ref())
            .cloned()
            .unwrap_or_default();
        violations.push(violation_for_arc(
            arc,
            boundary_id.as_str(),
            ArcViolationSpec {
                pattern: CodeCityViolationPattern::Modular,
                rule: CodeCityViolationRule::ModularBroadBridgeFile,
                severity: CodeCityViolationSeverity::Medium,
                id_discriminator: Some(path.clone()),
                message: format!(
                    "{} bridges {} detected modules and is becoming an implicit shared dependency hub.",
                    path,
                    modules.len()
                ),
                explanation: "The boundary is classified as modular. Broad bridge files blur module ownership and make dependency direction harder to reason about.".to_string(),
                recommendation: Some("Promote the file to an explicit facade or split shared behaviour behind narrower module contracts.".to_string()),
            },
            ctx,
        ));
    }

    violations
}

fn evaluate_event_driven_rules(
    source: &CodeCitySourceGraph,
    analysis: &CodeCityArchitectureAnalysis,
    file_arcs: &[CodeCityFileDependencyArc],
    ctx: &RuleEvaluationContext<'_, '_>,
    diagnostics: &mut Vec<CodeCityDiagnostic>,
) -> Vec<CodeCityArchitectureViolation> {
    let communities = communities_by_boundary(source, analysis, ctx.config);
    let hubs = event_hubs(source, ctx.config);
    let mut violations = Vec::new();

    for boundary_id in ctx.reports.keys() {
        if boundary_has_pattern(
            boundary_id,
            CodeCityArchitecturePattern::EventDriven,
            ctx.reports,
            ctx.config,
        ) && hubs.is_disjoint(
            &analysis
                .file_to_boundary
                .iter()
                .filter_map(|(path, id)| (id == boundary_id).then_some(path.clone()))
                .collect::<BTreeSet<_>>(),
        ) {
            diagnostics.push(CodeCityDiagnostic {
                code: "codecity.violations.event_hub_missing".to_string(),
                severity: "info".to_string(),
                message: format!(
                    "Boundary `{boundary_id}` was classified as event-driven but no message hub signal was detected for peer-dependency diagnostics."
                ),
                path: None,
                boundary_id: Some(boundary_id.clone()),
            });
        }
    }

    for arc in file_arcs.iter().filter(|arc| !arc.cross_boundary) {
        let Some(boundary_id) = arc.from_boundary_id.as_ref() else {
            continue;
        };
        if !boundary_has_pattern(
            boundary_id,
            CodeCityArchitecturePattern::EventDriven,
            ctx.reports,
            ctx.config,
        ) || hubs.is_empty()
            || hubs.contains(&arc.from_path)
            || hubs.contains(&arc.to_path)
        {
            continue;
        }
        let Some(community_by_path) = communities.get(boundary_id) else {
            continue;
        };
        let Some(from_community) = community_by_path.get(&arc.from_path) else {
            continue;
        };
        let Some(to_community) = community_by_path.get(&arc.to_path) else {
            continue;
        };
        if from_community == to_community {
            continue;
        }
        violations.push(violation_for_arc(
            arc,
            boundary_id,
            ArcViolationSpec {
                pattern: CodeCityViolationPattern::EventDriven,
                rule: CodeCityViolationRule::EventDrivenDirectPeerDependency,
                severity: CodeCityViolationSeverity::Medium,
                id_discriminator: None,
                message: format!(
                    "{} directly depends on peer {} in an event-driven boundary.",
                    arc.from_path, arc.to_path
                ),
                explanation: "The boundary is classified as event-driven. Peer modules should communicate through message contracts or hubs rather than direct file dependencies.".to_string(),
                recommendation: Some("Route this interaction through an event/message contract or a detected hub.".to_string()),
            },
            ctx,
        ));
    }

    violations
}

fn evaluate_cross_boundary_rules(
    analysis: &CodeCityArchitectureAnalysis,
    file_arcs: &[CodeCityFileDependencyArc],
    ctx: &RuleEvaluationContext<'_, '_>,
) -> Vec<CodeCityArchitectureViolation> {
    let mut violations = Vec::new();
    let threshold = high_coupling_threshold(file_arcs, ctx.config);
    for arc in file_arcs
        .iter()
        .filter(|arc| arc.cross_boundary && arc.weight >= threshold)
    {
        let boundary_id = arc.from_boundary_id.as_deref().unwrap_or("");
        violations.push(violation_for_arc(
            arc,
            boundary_id,
            ArcViolationSpec {
                pattern: CodeCityViolationPattern::CrossBoundary,
                rule: CodeCityViolationRule::CrossBoundaryHighCoupling,
                severity: CodeCityViolationSeverity::Medium,
                id_discriminator: None,
                message: format!(
                    "{} depends heavily on {} with weighted coupling {:.2}.",
                    arc.from_path, arc.to_path, arc.weight
                ),
                explanation: "This dependency crosses CodeCity boundaries with unusually high coupling for the current repository.".to_string(),
                recommendation: Some("Review whether the dependency should move behind a public boundary contract or shared package.".to_string()),
            },
            ctx,
        ));
    }

    if !ctx.config.violations.include_cycle_diagnostics {
        return violations;
    }

    let paths = analysis
        .macro_graph
        .edges
        .iter()
        .flat_map(|edge| [edge.from_boundary_id.clone(), edge.to_boundary_id.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let index_by_path = paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let graph_edges = analysis
        .macro_graph
        .edges
        .iter()
        .filter_map(|edge| {
            Some((
                *index_by_path.get(&edge.from_boundary_id)?,
                *index_by_path.get(&edge.to_boundary_id)?,
            ))
        })
        .collect::<Vec<_>>();
    let graph = FileGraph {
        paths: paths.clone(),
        index_by_path,
        edges: graph_edges,
    };
    let cycle_boundaries = strongly_connected_components(&graph)
        .into_iter()
        .filter(|component| component.len() > 1)
        .flat_map(|component| {
            component
                .into_iter()
                .map(|index| paths[index].clone())
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    if cycle_boundaries.is_empty() {
        return violations;
    }

    for arc in file_arcs.iter().filter(|arc| {
        arc.cross_boundary
            && arc
                .from_boundary_id
                .as_ref()
                .is_some_and(|id| cycle_boundaries.contains(id))
            && arc
                .to_boundary_id
                .as_ref()
                .is_some_and(|id| cycle_boundaries.contains(id))
    }) {
        let boundary_id = arc.from_boundary_id.as_deref().unwrap_or("");
        violations.push(violation_for_arc(
            arc,
            boundary_id,
            ArcViolationSpec {
                pattern: CodeCityViolationPattern::Cycle,
                rule: CodeCityViolationRule::CrossBoundaryCycle,
                severity: CodeCityViolationSeverity::High,
                id_discriminator: None,
                message: format!(
                    "Boundaries {} and {} participate in a macro-level dependency cycle.",
                    arc.from_boundary_id.as_deref().unwrap_or("unknown"),
                    arc.to_boundary_id.as_deref().unwrap_or("unknown")
                ),
                explanation: "The macro graph contains a strongly connected component across CodeCity boundaries, so these boundaries cannot be reasoned about in a one-way dependency order.".to_string(),
                recommendation: Some("Break the cycle by extracting shared contracts or reversing one dependency behind a stable interface.".to_string()),
            },
            ctx,
        ));
    }

    violations
}

fn violation_for_arc(
    arc: &CodeCityFileDependencyArc,
    boundary_id: &str,
    spec: ArcViolationSpec,
    ctx: &RuleEvaluationContext<'_, '_>,
) -> CodeCityArchitectureViolation {
    let evidence = arc
        .evidence_ids
        .iter()
        .filter_map(|id| {
            ctx.evidence_by_id
                .get(id)
                .map(|row| violation_evidence(row))
        })
        .collect::<Vec<_>>();
    let mut id_parts = vec![
        ctx.world.repo_id.as_str(),
        spec.pattern.as_str(),
        spec.rule.as_str(),
        arc.from_path.as_str(),
        arc.to_path.as_str(),
        arc.arc_id.as_str(),
    ];
    if let Some(discriminator) = spec.id_discriminator.as_deref() {
        id_parts.push(discriminator);
    }
    CodeCityArchitectureViolation {
        id: stable_id("codecity-violation", &id_parts),
        run_id: ctx.run_id.to_string(),
        commit_sha: ctx.world.commit_sha.clone(),
        boundary_id: (!boundary_id.is_empty()).then(|| boundary_id.to_string()),
        boundary_root: ctx
            .boundaries
            .get(boundary_id)
            .map(|boundary| boundary.root_path.clone()),
        pattern: spec.pattern,
        rule: spec.rule,
        severity: spec.severity,
        from_path: arc.from_path.clone(),
        to_path: Some(arc.to_path.clone()),
        from_zone: arc.from_zone.clone(),
        to_zone: arc.to_zone.clone(),
        from_boundary_id: arc.from_boundary_id.clone(),
        to_boundary_id: arc.to_boundary_id.clone(),
        arc_id: Some(arc.arc_id.clone()),
        message: spec.message,
        explanation: spec.explanation,
        recommendation: spec.recommendation,
        evidence_ids: arc.evidence_ids.clone(),
        evidence,
        confidence: 1.0,
        suppressed: false,
    }
}

fn violation_evidence(row: &CodeCityDependencyEvidence) -> CodeCityViolationEvidence {
    CodeCityViolationEvidence {
        evidence_id: row.evidence_id.clone(),
        edge_id: row.edge_id.clone(),
        edge_kind: row.edge_kind.clone(),
        from_symbol_id: row.from_symbol_id.clone(),
        to_symbol_id: row.to_symbol_id.clone(),
        from_artefact_id: row.from_artefact_id.clone(),
        to_artefact_id: row.to_artefact_id.clone(),
        start_line: row.start_line,
        end_line: row.end_line,
        to_symbol_ref: row.to_symbol_ref.clone(),
    }
}

fn boundary_has_pattern(
    boundary_id: &str,
    pattern: CodeCityArchitecturePattern,
    reports: &BoundaryReports<'_>,
    config: &CodeCityConfig,
) -> bool {
    let Some(report) = reports.get(boundary_id) else {
        return false;
    };
    report.primary_pattern == pattern
        || (config.violations.include_secondary_architecture_patterns
            && report.secondary_pattern == Some(pattern)
            && report
                .secondary_score
                .is_some_and(|score| score >= config.architecture.secondary_pattern_threshold))
}

fn layer_rank(zone: Option<&str>) -> Option<usize> {
    match zone {
        Some("edge") => Some(0),
        Some("application") | Some("ports") => Some(1),
        Some("core") => Some(2),
        Some("periphery") | Some("shared") => Some(3),
        Some(CODECITY_UNCLASSIFIED_ZONE) | None => None,
        _ => None,
    }
}

fn normalise_hex_zone(zone: Option<&str>) -> Option<CodeCityZone> {
    match zone {
        Some("core") => Some(CodeCityZone::Core),
        Some("application") => Some(CodeCityZone::Application),
        Some("ports") => Some(CodeCityZone::Ports),
        Some("periphery") | Some("infrastructure") | Some("adapters") | Some("shared") => {
            Some(CodeCityZone::Periphery)
        }
        Some("edge") => Some(CodeCityZone::Edge),
        _ => None,
    }
}

fn hexagonal_file_rule(
    from_path: &str,
    to_path: &str,
    from_zone: Option<CodeCityZone>,
    to_zone: Option<CodeCityZone>,
) -> Option<(
    CodeCityViolationRule,
    CodeCityViolationSeverity,
    String,
    String,
    Option<String>,
)> {
    match (from_zone, to_zone) {
        (Some(CodeCityZone::Core), Some(CodeCityZone::Periphery | CodeCityZone::Edge)) => Some((
            CodeCityViolationRule::HexagonalCoreImportsPeriphery,
            CodeCityViolationSeverity::High,
            format!("Core file {from_path} imports adapter/periphery file {to_path}."),
            "The boundary is classified as hexagonal. Core code should not depend on adapters or edge files.".to_string(),
            Some("Move this dependency behind a port/interface owned by the core or application layer.".to_string()),
        )),
        (Some(CodeCityZone::Application), Some(CodeCityZone::Edge)) => Some((
            CodeCityViolationRule::HexagonalApplicationImportsEdge,
            CodeCityViolationSeverity::Medium,
            format!("Application file {from_path} imports edge file {to_path}."),
            "Application code should not depend on delivery mechanisms such as routes, controllers, CLI, or UI files.".to_string(),
            Some("Move reusable behaviour into application/core code and have the edge depend inward.".to_string()),
        )),
        _ => None,
    }
}

fn is_conservative_external_ref(to_symbol_ref: Option<&str>, metadata: &str) -> bool {
    let Some(reference) = to_symbol_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return metadata.to_ascii_lowercase().contains("\"external\"");
    };
    if reference.starts_with("./")
        || reference.starts_with("../")
        || reference.starts_with("crate::")
        || reference.starts_with("self::")
        || reference.starts_with("super::")
    {
        return false;
    }
    reference.starts_with('@')
        || reference.contains('.')
        || (!reference.contains('/') && !reference.contains("::"))
}

fn communities_by_boundary(
    source: &CodeCitySourceGraph,
    analysis: &CodeCityArchitectureAnalysis,
    config: &CodeCityConfig,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut files_by_boundary = BTreeMap::<String, Vec<String>>::new();
    for (path, boundary_id) in &analysis.file_to_boundary {
        files_by_boundary
            .entry(boundary_id.clone())
            .or_default()
            .push(path.clone());
    }
    files_by_boundary
        .into_iter()
        .map(|(boundary_id, files)| {
            let graph = build_graph_from_paths(&files, &source.edges);
            let communities =
                detect_communities(&graph, config.boundaries.community_max_iterations);
            (boundary_id, communities.community_by_path)
        })
        .collect()
}

fn build_graph_from_paths(paths: &[String], edges: &[CodeCitySourceEdge]) -> FileGraph {
    let mut sorted_paths = paths.to_vec();
    sorted_paths.sort();
    sorted_paths.dedup();
    let index_by_path = sorted_paths
        .iter()
        .enumerate()
        .map(|(index, path)| (path.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let graph_edges = edges
        .iter()
        .filter_map(|edge| {
            Some((
                *index_by_path.get(&edge.from_path)?,
                *index_by_path.get(&edge.to_path)?,
            ))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    FileGraph {
        paths: sorted_paths,
        index_by_path,
        edges: graph_edges,
    }
}

fn is_facade_file(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    matches!(
        filename,
        "mod.rs" | "lib.rs" | "index.ts" | "index.js" | "__init__.py"
    ) || path.split('/').any(|segment| {
        matches!(
            segment,
            "ports" | "interfaces" | "contracts" | "api" | "public"
        )
    })
}

fn event_hubs(source: &CodeCitySourceGraph, config: &CodeCityConfig) -> BTreeSet<String> {
    let mut hubs = BTreeSet::new();
    for artefact in &source.artefacts {
        let lower = format!(
            "{} {} {}",
            artefact.path,
            artefact.symbol_fqn.as_deref().unwrap_or(""),
            artefact.signature.as_deref().unwrap_or("")
        )
        .to_ascii_lowercase();
        if lower.contains("event")
            || lower.contains("message")
            || lower.contains("bus")
            || lower.contains("broker")
        {
            hubs.insert(artefact.path.clone());
        }
    }
    for hint in &source.external_dependency_hints {
        let lower = format!(
            "{} {}",
            hint.to_symbol_ref.as_deref().unwrap_or(""),
            hint.metadata
        )
        .to_ascii_lowercase();
        if config
            .architecture
            .message_infra_libraries
            .iter()
            .any(|needle| lower.contains(needle))
        {
            hubs.insert(hint.from_path.clone());
        }
    }
    hubs
}

fn high_coupling_threshold(
    file_arcs: &[CodeCityFileDependencyArc],
    config: &CodeCityConfig,
) -> f64 {
    let mut weights = file_arcs
        .iter()
        .filter(|arc| arc.cross_boundary)
        .map(|arc| arc.weight)
        .filter(|weight| weight.is_finite())
        .collect::<Vec<_>>();
    if weights.is_empty() {
        return f64::INFINITY;
    }
    weights.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let percentile_index = ((weights.len().saturating_sub(1)) as f64
        * config.violations.cross_boundary_high_coupling_percentile)
        .round() as usize;
    weights[percentile_index].max(
        config
            .violations
            .cross_boundary_high_coupling_absolute_threshold,
    )
}
