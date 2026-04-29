use super::*;

pub(super) fn build_render_arcs(
    file_arcs: &[CodeCityFileDependencyArc],
    violations: &[CodeCityArchitectureViolation],
    world: &CodeCityWorldPayload,
    config: &CodeCityConfig,
) -> Vec<CodeCityRenderArc> {
    if !config.arcs.enabled {
        return Vec::new();
    }
    let building_by_path = world
        .buildings
        .iter()
        .map(|building| (building.path.clone(), building))
        .collect::<BTreeMap<_, _>>();
    let mut arcs = Vec::new();

    for file_arc in file_arcs {
        if let Some(geometry) = file_arc_geometry(file_arc, &building_by_path, config) {
            arcs.push(CodeCityRenderArc {
                id: stable_id(
                    "codecity-render-arc",
                    &[
                        CodeCityArcKind::Dependency.as_str(),
                        file_arc.arc_id.as_str(),
                        file_arc.from_path.as_str(),
                        file_arc.to_path.as_str(),
                        CodeCityArcVisibility::VisibleOnSelection.as_str(),
                    ],
                ),
                kind: CodeCityArcKind::Dependency,
                visibility: CodeCityArcVisibility::VisibleOnSelection,
                severity: file_arc.highest_severity,
                from_path: Some(file_arc.from_path.clone()),
                to_path: Some(file_arc.to_path.clone()),
                from_boundary_id: file_arc.from_boundary_id.clone(),
                to_boundary_id: file_arc.to_boundary_id.clone(),
                source_arc_id: Some(file_arc.arc_id.clone()),
                violation_id: None,
                weight: file_arc.weight,
                label: None,
                tooltip: Some(format!(
                    "{} dependency evidence row(s), weight {:.2}",
                    file_arc.edge_count, file_arc.weight
                )),
                geometry,
                metadata_json: "{}".to_string(),
            });
        }
    }

    for violation in violations
        .iter()
        .filter(|violation| violation.arc_id.is_some())
    {
        let Some(file_arc) = file_arcs
            .iter()
            .find(|arc| Some(arc.arc_id.as_str()) == violation.arc_id.as_deref())
        else {
            continue;
        };
        let Some(geometry) = file_arc_geometry(file_arc, &building_by_path, config) else {
            continue;
        };
        let kind = if violation.rule == CodeCityViolationRule::CrossBoundaryCycle {
            CodeCityArcKind::Cycle
        } else {
            CodeCityArcKind::Violation
        };
        arcs.push(CodeCityRenderArc {
            id: stable_id(
                "codecity-render-arc",
                &[
                    kind.as_str(),
                    file_arc.arc_id.as_str(),
                    violation.id.as_str(),
                    CodeCityArcVisibility::VisibleAtMediumZoom.as_str(),
                ],
            ),
            kind,
            visibility: CodeCityArcVisibility::VisibleAtMediumZoom,
            severity: Some(violation.severity),
            from_path: Some(file_arc.from_path.clone()),
            to_path: Some(file_arc.to_path.clone()),
            from_boundary_id: file_arc.from_boundary_id.clone(),
            to_boundary_id: file_arc.to_boundary_id.clone(),
            source_arc_id: Some(file_arc.arc_id.clone()),
            violation_id: Some(violation.id.clone()),
            weight: file_arc.weight,
            label: Some(violation.rule.as_str().to_string()),
            tooltip: Some(violation.message.clone()),
            geometry,
            metadata_json: "{}".to_string(),
        });
    }

    arcs.extend(cross_boundary_render_arcs(
        file_arcs,
        world,
        &building_by_path,
        config,
    ));
    arcs.sort_by(compare_render_arcs);
    arcs
}

fn file_arc_geometry(
    arc: &CodeCityFileDependencyArc,
    buildings: &BTreeMap<String, &CodeCityBuilding>,
    config: &CodeCityConfig,
) -> Option<CodeCityArcGeometry> {
    let from = buildings.get(&arc.from_path)?;
    let to = buildings.get(&arc.to_path)?;
    Some(geometry_between_buildings(from, to, config))
}

pub(super) fn geometry_between_buildings(
    from: &CodeCityBuilding,
    to: &CodeCityBuilding,
    config: &CodeCityConfig,
) -> CodeCityArcGeometry {
    let from_x = from.geometry.x + from.geometry.width / 2.0;
    let from_z = from.geometry.z + from.geometry.depth / 2.0;
    let from_y = from.geometry.y + from.geometry.height + config.arcs.start_offset;
    let to_x = to.geometry.x + to.geometry.width / 2.0;
    let to_z = to.geometry.z + to.geometry.depth / 2.0;
    let to_y = to.geometry.y + to.geometry.height + config.arcs.end_offset;
    let horizontal_distance = ((to_x - from_x).powi(2) + (to_z - from_z).powi(2)).sqrt();
    let control_y = from_y.max(to_y)
        + config.arcs.base_arc_lift
        + horizontal_distance * config.arcs.arc_lift_scale;
    CodeCityArcGeometry {
        from_x,
        from_y,
        from_z,
        to_x,
        to_y,
        to_z,
        control_y,
    }
}

fn cross_boundary_render_arcs(
    file_arcs: &[CodeCityFileDependencyArc],
    world: &CodeCityWorldPayload,
    buildings: &BTreeMap<String, &CodeCityBuilding>,
    config: &CodeCityConfig,
) -> Vec<CodeCityRenderArc> {
    let mut grouped = BTreeMap::<(String, String), (f64, usize)>::new();
    for arc in file_arcs.iter().filter(|arc| arc.cross_boundary) {
        let Some(from_boundary) = arc.from_boundary_id.as_ref() else {
            continue;
        };
        let Some(to_boundary) = arc.to_boundary_id.as_ref() else {
            continue;
        };
        let entry = grouped
            .entry((from_boundary.clone(), to_boundary.clone()))
            .or_insert((0.0, 0));
        entry.0 += arc.weight;
        entry.1 += arc.edge_count;
    }

    let mut centroids = BTreeMap::<String, CodeCityBuilding>::new();
    for boundary in &world.boundaries {
        let members = buildings
            .values()
            .filter(|building| building.boundary_id == boundary.id)
            .copied()
            .collect::<Vec<_>>();
        if let Some(centroid) = centroid_building(boundary.id.as_str(), &members) {
            centroids.insert(boundary.id.clone(), centroid);
        }
    }

    let mut arcs = grouped
        .into_iter()
        .filter_map(|((from_boundary, to_boundary), (weight, edge_count))| {
            let from = centroids.get(&from_boundary)?;
            let to = centroids.get(&to_boundary)?;
            let geometry = geometry_between_buildings(from, to, config);
            Some(CodeCityRenderArc {
                id: stable_id(
                    "codecity-render-arc",
                    &[
                        CodeCityArcKind::CrossBoundary.as_str(),
                        from_boundary.as_str(),
                        to_boundary.as_str(),
                        CodeCityArcVisibility::VisibleAtWorldZoom.as_str(),
                    ],
                ),
                kind: CodeCityArcKind::CrossBoundary,
                visibility: CodeCityArcVisibility::VisibleAtWorldZoom,
                severity: None,
                from_path: None,
                to_path: None,
                from_boundary_id: Some(from_boundary.clone()),
                to_boundary_id: Some(to_boundary.clone()),
                source_arc_id: None,
                violation_id: None,
                weight,
                label: Some(format!("{from_boundary} -> {to_boundary}")),
                tooltip: Some(format!(
                    "{edge_count} cross-boundary dependency evidence row(s), weight {:.2}",
                    weight
                )),
                geometry,
                metadata_json: "{}".to_string(),
            })
        })
        .collect::<Vec<_>>();
    arcs.sort_by(|left, right| {
        right
            .weight
            .partial_cmp(&left.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });
    arcs.truncate(config.arcs.max_world_arcs);
    arcs
}

fn centroid_building(boundary_id: &str, members: &[&CodeCityBuilding]) -> Option<CodeCityBuilding> {
    let count = members.len() as f64;
    if count == 0.0 {
        return None;
    }
    let x = members
        .iter()
        .map(|building| building.geometry.x + building.geometry.width / 2.0)
        .sum::<f64>()
        / count;
    let z = members
        .iter()
        .map(|building| building.geometry.z + building.geometry.depth / 2.0)
        .sum::<f64>()
        / count;
    let height = members
        .iter()
        .map(|building| building.geometry.height)
        .fold(0.0_f64, f64::max);
    let mut building = members[0].clone();
    building.path = boundary_id.to_string();
    building.geometry.x = x;
    building.geometry.z = z;
    building.geometry.width = 0.0;
    building.geometry.depth = 0.0;
    building.geometry.height = height;
    Some(building)
}

pub(super) fn world_arcs(
    arcs: &[CodeCityRenderArc],
    config: &CodeCityConfig,
) -> Vec<CodeCityRenderArc> {
    let mut visible = arcs
        .iter()
        .filter(|arc| {
            matches!(
                arc.visibility,
                CodeCityArcVisibility::VisibleAtMediumZoom
                    | CodeCityArcVisibility::VisibleAtWorldZoom
                    | CodeCityArcVisibility::AlwaysVisible
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    visible.sort_by(compare_render_arcs);
    visible.truncate(config.arcs.max_world_arcs + config.arcs.max_violation_arcs);
    visible
}

pub(super) fn apply_violation_state_to_file_arcs(
    file_arcs: &mut [CodeCityFileDependencyArc],
    violations: &[CodeCityArchitectureViolation],
) {
    let mut by_arc = BTreeMap::<String, CodeCityViolationSeverity>::new();
    for violation in violations {
        let Some(arc_id) = violation.arc_id.as_ref() else {
            continue;
        };
        by_arc
            .entry(arc_id.clone())
            .and_modify(|current| {
                if violation.severity.rank() < current.rank() {
                    *current = violation.severity;
                }
            })
            .or_insert(violation.severity);
    }
    for arc in file_arcs {
        arc.highest_severity = by_arc.get(&arc.arc_id).copied();
        arc.has_violation = arc.highest_severity.is_some();
    }
}

pub(super) fn apply_diagnostic_badges(
    buildings: &mut [CodeCityBuilding],
    violations: &[CodeCityArchitectureViolation],
    file_arcs: &[CodeCityFileDependencyArc],
) {
    let mut violation_counts = BTreeMap::<String, (usize, CodeCityViolationSeverity)>::new();
    for violation in violations {
        for path in [Some(&violation.from_path), violation.to_path.as_ref()]
            .into_iter()
            .flatten()
        {
            violation_counts
                .entry(path.clone())
                .and_modify(|(count, severity)| {
                    *count += 1;
                    if violation.severity.rank() < severity.rank() {
                        *severity = violation.severity;
                    }
                })
                .or_insert((1, violation.severity));
        }
    }

    let mut cross_boundary_counts = BTreeMap::<String, usize>::new();
    for arc in file_arcs
        .iter()
        .filter(|arc| arc.cross_boundary && arc.weight >= 10.0)
    {
        *cross_boundary_counts
            .entry(arc.from_path.clone())
            .or_insert(0) += 1;
        *cross_boundary_counts
            .entry(arc.to_path.clone())
            .or_insert(0) += 1;
    }

    for building in buildings {
        building.diagnostic_badges.clear();
        if let Some((count, severity)) = violation_counts.get(&building.path) {
            building.diagnostic_badges.push(CodeCityDiagnosticBadge {
                kind: CodeCityDiagnosticBadgeKind::ArchitectureViolation,
                severity: *severity,
                count: *count,
                tooltip: format!(
                    "{count} architecture violation(s) involve this file; highest severity: {}.",
                    severity.as_str()
                ),
            });
        }
        if let Some(count) = cross_boundary_counts.get(&building.path) {
            building.diagnostic_badges.push(CodeCityDiagnosticBadge {
                kind: CodeCityDiagnosticBadgeKind::CrossBoundaryCoupling,
                severity: CodeCityViolationSeverity::Medium,
                count: *count,
                tooltip: format!(
                    "{count} high-coupling cross-boundary dependency arc(s) involve this file."
                ),
            });
        }
    }
}

pub(super) fn apply_boundary_violation_summaries(
    boundaries: &mut [CodeCityBoundary],
    violations: &[CodeCityArchitectureViolation],
) {
    let mut by_boundary = BTreeMap::<String, Vec<&CodeCityArchitectureViolation>>::new();
    for violation in violations {
        if let Some(boundary_id) = violation
            .boundary_id
            .as_ref()
            .or(violation.from_boundary_id.as_ref())
        {
            by_boundary
                .entry(boundary_id.clone())
                .or_default()
                .push(violation);
        }
    }
    for boundary in boundaries {
        boundary.violation_summary =
            summarise_violations(by_boundary.remove(&boundary.id).unwrap_or_default());
    }
}

fn summarise_violations(
    violations: Vec<&CodeCityArchitectureViolation>,
) -> CodeCityViolationSummary {
    let mut summary = CodeCityViolationSummary::default();
    let mut by_rule = BTreeMap::<CodeCityViolationRule, (usize, CodeCityViolationSeverity)>::new();
    for violation in violations {
        summary.total += 1;
        match violation.severity {
            CodeCityViolationSeverity::High => summary.high += 1,
            CodeCityViolationSeverity::Medium => summary.medium += 1,
            CodeCityViolationSeverity::Low => summary.low += 1,
            CodeCityViolationSeverity::Info => summary.info += 1,
        }
        by_rule
            .entry(violation.rule)
            .and_modify(|(count, severity)| {
                *count += 1;
                if violation.severity.rank() < severity.rank() {
                    *severity = violation.severity;
                }
            })
            .or_insert((1, violation.severity));
    }
    summary.by_rule = by_rule
        .into_iter()
        .map(|(rule, (count, severity))| CodeCityViolationRuleCount {
            rule,
            count,
            severity,
        })
        .collect();
    summary
}
