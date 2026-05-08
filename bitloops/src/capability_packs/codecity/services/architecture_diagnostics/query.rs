use super::*;

pub fn violations_connection(
    violations: &[CodeCityArchitectureViolation],
    snapshot_status: CodeCitySnapshotStatus,
    filter: &CodeCityViolationFilter,
    first: usize,
    after: Option<&str>,
    last: Option<usize>,
    before: Option<&str>,
) -> CodeCityViolationConnectionPayload {
    let filtered = filter_violations(violations, filter);
    let (items, page_info) = page_items(filtered, first, after, last, before, violation_cursor);
    CodeCityViolationConnectionPayload {
        snapshot_status,
        total_count: violations
            .iter()
            .filter(|v| violation_matches(v, filter))
            .count(),
        edges: items
            .into_iter()
            .map(|node| CodeCityViolationConnectionEdgePayload {
                cursor: violation_cursor(&node),
                node,
            })
            .collect(),
        page_info,
    }
}

pub fn arcs_connection(
    arcs: &[CodeCityRenderArc],
    snapshot_status: CodeCitySnapshotStatus,
    filter: &CodeCityArcFilter,
    first: usize,
    after: Option<&str>,
    last: Option<usize>,
    before: Option<&str>,
) -> CodeCityArcConnectionPayload {
    let filtered = filter_arcs(arcs, filter);
    let (items, page_info) = page_items(filtered, first, after, last, before, arc_cursor);
    CodeCityArcConnectionPayload {
        snapshot_status,
        total_count: arcs.iter().filter(|arc| arc_matches(arc, filter)).count(),
        edges: items
            .into_iter()
            .map(|node| CodeCityArcConnectionEdgePayload {
                cursor: arc_cursor(&node),
                node,
            })
            .collect(),
        page_info,
    }
}

pub fn file_detail(
    path: &str,
    snapshot_status: CodeCitySnapshotStatus,
    world: &CodeCityWorldPayload,
    snapshot: &CodeCityArchitectureDiagnosticsSnapshot,
    incoming_limit: usize,
    outgoing_limit: usize,
) -> Option<CodeCityFileDetailPayload> {
    let building = world
        .buildings
        .iter()
        .find(|building| building.path == path)?
        .clone();
    let boundary = world
        .boundaries
        .iter()
        .find(|boundary| boundary.id == building.boundary_id);
    let incoming = dependency_connection(
        snapshot
            .file_arcs
            .iter()
            .filter(|arc| arc.to_path == path)
            .cloned()
            .collect::<Vec<_>>(),
        incoming_limit,
    );
    let outgoing = dependency_connection(
        snapshot
            .file_arcs
            .iter()
            .filter(|arc| arc.from_path == path)
            .cloned()
            .collect::<Vec<_>>(),
        outgoing_limit,
    );
    let violations = snapshot
        .violations
        .iter()
        .filter(|violation| {
            violation.from_path == path || violation.to_path.as_deref() == Some(path)
        })
        .take(100)
        .cloned()
        .collect::<Vec<_>>();
    let related_arcs = snapshot
        .render_arcs
        .iter()
        .filter(|arc| {
            arc.from_path.as_deref() == Some(path) || arc.to_path.as_deref() == Some(path)
        })
        .take(200)
        .cloned()
        .collect::<Vec<_>>();
    Some(CodeCityFileDetailPayload {
        status: "ok".to_string(),
        path: path.to_string(),
        snapshot_status,
        building: Some(building),
        architecture_context: CodeCityFileArchitectureContext {
            boundary_id: boundary.map(|boundary| boundary.id.clone()),
            boundary_name: boundary.map(|boundary| boundary.name.clone()),
            primary_pattern: boundary
                .and_then(|boundary| boundary.architecture.as_ref())
                .map(|architecture| architecture.primary_pattern),
        },
        incoming_dependencies: incoming,
        outgoing_dependencies: outgoing,
        violations,
        related_arcs,
    })
}

fn dependency_connection(
    mut arcs: Vec<CodeCityFileDependencyArc>,
    limit: usize,
) -> CodeCityDependencyConnectionPayload {
    arcs.sort_by(|left, right| {
        right
            .has_violation
            .cmp(&left.has_violation)
            .then_with(|| {
                left.highest_severity
                    .map(CodeCityViolationSeverity::rank)
                    .cmp(&right.highest_severity.map(CodeCityViolationSeverity::rank))
            })
            .then_with(|| {
                right
                    .weight
                    .partial_cmp(&left.weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.from_path.cmp(&right.from_path))
            .then_with(|| left.to_path.cmp(&right.to_path))
    });
    let total_count = arcs.len();
    arcs.truncate(limit);
    CodeCityDependencyConnectionPayload {
        total_count,
        edges: arcs
            .into_iter()
            .map(|node| CodeCityDependencyConnectionEdgePayload {
                cursor: node.arc_id.clone(),
                node,
            })
            .collect(),
    }
}

pub fn filter_violations(
    violations: &[CodeCityArchitectureViolation],
    filter: &CodeCityViolationFilter,
) -> Vec<CodeCityArchitectureViolation> {
    let mut filtered = violations
        .iter()
        .filter(|violation| violation_matches(violation, filter))
        .cloned()
        .collect::<Vec<_>>();
    filtered.sort_by(compare_violations);
    filtered
}

pub fn filter_arcs(
    arcs: &[CodeCityRenderArc],
    filter: &CodeCityArcFilter,
) -> Vec<CodeCityRenderArc> {
    let mut filtered = arcs
        .iter()
        .filter(|arc| arc_matches(arc, filter))
        .cloned()
        .collect::<Vec<_>>();
    filtered.sort_by(compare_render_arcs);
    filtered
}

fn violation_matches(
    violation: &CodeCityArchitectureViolation,
    filter: &CodeCityViolationFilter,
) -> bool {
    if violation.suppressed && !filter.include_suppressed {
        return false;
    }
    if let Some(severity) = filter.severity
        && violation.severity != severity
    {
        return false;
    }
    if !filter.severities.is_empty() && !filter.severities.contains(&violation.severity) {
        return false;
    }
    if let Some(pattern) = filter.pattern
        && violation.pattern != pattern
    {
        return false;
    }
    if let Some(rule) = filter.rule
        && violation.rule != rule
    {
        return false;
    }
    if let Some(boundary_id) = filter.boundary_id.as_deref()
        && violation.boundary_id.as_deref() != Some(boundary_id)
        && violation.from_boundary_id.as_deref() != Some(boundary_id)
        && violation.to_boundary_id.as_deref() != Some(boundary_id)
    {
        return false;
    }
    if let Some(path) = filter.path.as_deref()
        && violation.from_path != path
        && violation.to_path.as_deref() != Some(path)
    {
        return false;
    }
    if let Some(from_path) = filter.from_path.as_deref()
        && violation.from_path != from_path
    {
        return false;
    }
    if let Some(to_path) = filter.to_path.as_deref()
        && violation.to_path.as_deref() != Some(to_path)
    {
        return false;
    }
    true
}

fn arc_matches(arc: &CodeCityRenderArc, filter: &CodeCityArcFilter) -> bool {
    if !filter.include_hidden && arc.visibility == CodeCityArcVisibility::HiddenByDefault {
        return false;
    }
    if let Some(kind) = filter.kind
        && arc.kind != kind
    {
        return false;
    }
    if let Some(visibility) = filter.visibility
        && arc.visibility != visibility
    {
        return false;
    }
    if let Some(severity) = filter.severity
        && arc.severity != Some(severity)
    {
        return false;
    }
    if let Some(boundary_id) = filter.boundary_id.as_deref()
        && arc.from_boundary_id.as_deref() != Some(boundary_id)
        && arc.to_boundary_id.as_deref() != Some(boundary_id)
    {
        return false;
    }
    if let Some(path) = filter.path.as_deref() {
        match filter
            .direction
            .unwrap_or(CodeCityDependencyDirection::Both)
        {
            CodeCityDependencyDirection::Incoming if arc.to_path.as_deref() != Some(path) => {
                return false;
            }
            CodeCityDependencyDirection::Outgoing if arc.from_path.as_deref() != Some(path) => {
                return false;
            }
            CodeCityDependencyDirection::Both
                if arc.from_path.as_deref() != Some(path)
                    && arc.to_path.as_deref() != Some(path) =>
            {
                return false;
            }
            _ => {}
        }
    }
    true
}

fn page_items<T: Clone>(
    items: Vec<T>,
    first: usize,
    after: Option<&str>,
    last: Option<usize>,
    before: Option<&str>,
    cursor: impl Fn(&T) -> String,
) -> (Vec<T>, CodeCityPageInfo) {
    let total = items.len();
    let (start, end) = if let Some(last) = last {
        let end = before
            .and_then(|needle| items.iter().position(|item| cursor(item) == needle))
            .unwrap_or(total);
        (end.saturating_sub(last), end)
    } else {
        let start = after
            .and_then(|needle| items.iter().position(|item| cursor(item) == needle))
            .map(|index| index + 1)
            .unwrap_or(0);
        (start, start.saturating_add(first).min(total))
    };
    let page = items[start..end].to_vec();
    let page_info = CodeCityPageInfo {
        has_next_page: end < total,
        has_previous_page: start > 0,
        start_cursor: page.first().map(&cursor),
        end_cursor: page.last().map(&cursor),
    };
    (page, page_info)
}
