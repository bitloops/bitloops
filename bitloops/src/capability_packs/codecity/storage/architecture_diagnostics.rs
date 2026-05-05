use anyhow::{Context, Result};
use rusqlite::params;

use super::SqliteCodeCityRepository;
use crate::capability_packs::codecity::types::{
    CODECITY_DEFAULT_SNAPSHOT_KEY, CodeCityArcGeometry, CodeCityArcKind, CodeCityArcVisibility,
    CodeCityArchitectureDiagnosticsSnapshot, CodeCityArchitectureViolation,
    CodeCityDependencyEvidence, CodeCityFileDependencyArc, CodeCityRenderArc,
    CodeCityViolationEvidence, CodeCityViolationPattern, CodeCityViolationRule,
    CodeCityViolationSeverity,
};

impl SqliteCodeCityRepository {
    pub fn replace_architecture_diagnostics_snapshot(
        &self,
        snapshot: &CodeCityArchitectureDiagnosticsSnapshot,
    ) -> Result<()> {
        self.replace_architecture_diagnostics_snapshot_for_key(
            CODECITY_DEFAULT_SNAPSHOT_KEY,
            snapshot,
        )
    }

    pub(crate) fn replace_architecture_diagnostics_snapshot_for_key(
        &self,
        snapshot_key: &str,
        snapshot: &CodeCityArchitectureDiagnosticsSnapshot,
    ) -> Result<()> {
        let created_at = chrono::Utc::now().to_rfc3339();
        self.sqlite.with_connection(|conn| {
            let tx = conn
                .unchecked_transaction()
                .context("starting CodeCity architecture diagnostics snapshot transaction")?;

            tx.execute(
                "DELETE FROM codecity_render_arcs_current WHERE repo_id = ?1 AND snapshot_key = ?2",
                params![snapshot.repo_id, snapshot_key],
            )?;
            tx.execute(
                "DELETE FROM codecity_architecture_violations_current WHERE repo_id = ?1 AND snapshot_key = ?2",
                params![snapshot.repo_id, snapshot_key],
            )?;
            tx.execute(
                "DELETE FROM codecity_file_dependency_arcs_current WHERE repo_id = ?1 AND snapshot_key = ?2",
                params![snapshot.repo_id, snapshot_key],
            )?;
            tx.execute(
                "DELETE FROM codecity_dependency_evidence_current WHERE repo_id = ?1 AND snapshot_key = ?2",
                params![snapshot.repo_id, snapshot_key],
            )?;

            for row in &snapshot.evidence {
                tx.execute(
                    "INSERT INTO codecity_dependency_evidence_current (
                        repo_id, snapshot_key, evidence_id, run_id, commit_sha, from_path, to_path,
                        to_symbol_ref, from_boundary_id, to_boundary_id, from_zone, to_zone,
                        from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id,
                        edge_id, edge_kind, language, start_line, end_line, metadata_json,
                        resolved, cross_boundary, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
                    params![
                        snapshot.repo_id,
                        snapshot_key,
                        row.evidence_id,
                        row.run_id,
                        row.commit_sha,
                        row.from_path,
                        row.to_path,
                        row.to_symbol_ref,
                        row.from_boundary_id,
                        row.to_boundary_id,
                        row.from_zone,
                        row.to_zone,
                        row.from_symbol_id,
                        row.from_artefact_id,
                        row.to_symbol_id,
                        row.to_artefact_id,
                        row.edge_id,
                        row.edge_kind,
                        row.language,
                        row.start_line,
                        row.end_line,
                        row.metadata_json,
                        sqlite_int(row.resolved),
                        sqlite_int(row.cross_boundary),
                        created_at,
                    ],
                )?;
            }

            for row in &snapshot.file_arcs {
                tx.execute(
                    "INSERT INTO codecity_file_dependency_arcs_current (
                        repo_id, snapshot_key, arc_id, run_id, commit_sha, from_path, to_path,
                        from_boundary_id, to_boundary_id, from_zone, to_zone, edge_count,
                        import_count, call_count, reference_count, export_count,
                        inheritance_count, weight, cross_boundary, has_violation,
                        highest_severity, evidence_ids_json, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                    params![
                        snapshot.repo_id,
                        snapshot_key,
                        row.arc_id,
                        row.run_id,
                        row.commit_sha,
                        row.from_path,
                        row.to_path,
                        row.from_boundary_id,
                        row.to_boundary_id,
                        row.from_zone,
                        row.to_zone,
                        row.edge_count as i64,
                        row.import_count as i64,
                        row.call_count as i64,
                        row.reference_count as i64,
                        row.export_count as i64,
                        row.inheritance_count as i64,
                        row.weight,
                        sqlite_int(row.cross_boundary),
                        sqlite_int(row.has_violation),
                        row.highest_severity.map(CodeCityViolationSeverity::as_str),
                        serde_json::to_string(&row.evidence_ids)?,
                        created_at,
                    ],
                )?;
            }

            for row in &snapshot.violations {
                tx.execute(
                    "INSERT INTO codecity_architecture_violations_current (
                        repo_id, snapshot_key, violation_id, run_id, commit_sha, boundary_id,
                        boundary_root, pattern, rule, severity, from_path, to_path,
                        from_zone, to_zone, from_boundary_id, to_boundary_id, arc_id,
                        message, explanation, recommendation, evidence_ids_json,
                        evidence_json, confidence, suppressed, suppression_reason, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
                    params![
                        snapshot.repo_id,
                        snapshot_key,
                        row.id,
                        row.run_id,
                        row.commit_sha,
                        row.boundary_id,
                        row.boundary_root,
                        row.pattern.as_str(),
                        row.rule.as_str(),
                        row.severity.as_str(),
                        row.from_path,
                        row.to_path,
                        row.from_zone,
                        row.to_zone,
                        row.from_boundary_id,
                        row.to_boundary_id,
                        row.arc_id,
                        row.message,
                        row.explanation,
                        row.recommendation,
                        serde_json::to_string(&row.evidence_ids)?,
                        serde_json::to_string(&row.evidence)?,
                        row.confidence,
                        sqlite_int(row.suppressed),
                        Option::<String>::None,
                        created_at,
                    ],
                )?;
            }

            for row in &snapshot.render_arcs {
                tx.execute(
                    "INSERT INTO codecity_render_arcs_current (
                        repo_id, snapshot_key, render_arc_id, run_id, commit_sha, arc_kind, visibility,
                        severity, from_path, to_path, from_boundary_id, to_boundary_id,
                        source_arc_id, violation_id, weight, label, tooltip, from_x,
                        from_y, from_z, to_x, to_y, to_z, control_y, metadata_json,
                        created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
                    params![
                        snapshot.repo_id,
                        snapshot_key,
                        row.id,
                        snapshot.run_id,
                        snapshot.commit_sha,
                        row.kind.as_str(),
                        row.visibility.as_str(),
                        row.severity.map(CodeCityViolationSeverity::as_str),
                        row.from_path,
                        row.to_path,
                        row.from_boundary_id,
                        row.to_boundary_id,
                        row.source_arc_id,
                        row.violation_id,
                        row.weight,
                        row.label,
                        row.tooltip,
                        row.geometry.from_x,
                        row.geometry.from_y,
                        row.geometry.from_z,
                        row.geometry.to_x,
                        row.geometry.to_y,
                        row.geometry.to_z,
                        row.geometry.control_y,
                        row.metadata_json,
                        created_at,
                    ],
                )?;
            }

            tx.commit()
                .context("committing CodeCity architecture diagnostics snapshot transaction")?;
            Ok(())
        })
    }

    pub fn load_architecture_diagnostics_snapshot(
        &self,
        repo_id: &str,
    ) -> Result<CodeCityArchitectureDiagnosticsSnapshot> {
        self.load_architecture_diagnostics_snapshot_for_key(repo_id, CODECITY_DEFAULT_SNAPSHOT_KEY)
    }

    pub(crate) fn load_architecture_diagnostics_snapshot_for_key(
        &self,
        repo_id: &str,
        snapshot_key: &str,
    ) -> Result<CodeCityArchitectureDiagnosticsSnapshot> {
        let evidence = self.load_dependency_evidence(repo_id, snapshot_key)?;
        let file_arcs = self.load_file_dependency_arcs(repo_id, snapshot_key)?;
        let violations = self.load_architecture_violations(repo_id, snapshot_key)?;
        let render_arcs = self.load_render_arcs(repo_id, snapshot_key)?;
        let run_id = evidence
            .first()
            .map(|row| row.run_id.clone())
            .or_else(|| file_arcs.first().map(|row| row.run_id.clone()))
            .or_else(|| violations.first().map(|row| row.run_id.clone()))
            .unwrap_or_default();
        let commit_sha = evidence
            .first()
            .and_then(|row| row.commit_sha.clone())
            .or_else(|| file_arcs.first().and_then(|row| row.commit_sha.clone()))
            .or_else(|| violations.first().and_then(|row| row.commit_sha.clone()));
        Ok(CodeCityArchitectureDiagnosticsSnapshot {
            repo_id: repo_id.to_string(),
            run_id,
            commit_sha,
            evidence,
            file_arcs,
            violations,
            render_arcs,
            diagnostics: Vec::new(),
        })
    }

    fn load_dependency_evidence(
        &self,
        repo_id: &str,
        snapshot_key: &str,
    ) -> Result<Vec<CodeCityDependencyEvidence>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT evidence_id, run_id, commit_sha, from_path, to_path, to_symbol_ref,
                        from_boundary_id, to_boundary_id, from_zone, to_zone, from_symbol_id,
                        from_artefact_id, to_symbol_id, to_artefact_id, edge_id, edge_kind,
                        language, start_line, end_line, metadata_json, resolved, cross_boundary
                 FROM codecity_dependency_evidence_current
                 WHERE repo_id = ?1 AND snapshot_key = ?2
                 ORDER BY evidence_id ASC",
            )?;
            let rows = stmt.query_map(params![repo_id, snapshot_key], |row| {
                Ok(CodeCityDependencyEvidence {
                    evidence_id: row.get(0)?,
                    run_id: row.get(1)?,
                    commit_sha: row.get(2)?,
                    from_path: row.get(3)?,
                    to_path: row.get(4)?,
                    to_symbol_ref: row.get(5)?,
                    from_boundary_id: row.get(6)?,
                    to_boundary_id: row.get(7)?,
                    from_zone: row.get(8)?,
                    to_zone: row.get(9)?,
                    from_symbol_id: row.get(10)?,
                    from_artefact_id: row.get(11)?,
                    to_symbol_id: row.get(12)?,
                    to_artefact_id: row.get(13)?,
                    edge_id: row.get(14)?,
                    edge_kind: row.get(15)?,
                    language: row.get(16)?,
                    start_line: row.get(17)?,
                    end_line: row.get(18)?,
                    metadata_json: row.get(19)?,
                    resolved: sqlite_bool(row.get::<_, i64>(20)?),
                    cross_boundary: sqlite_bool(row.get::<_, i64>(21)?),
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })
    }

    fn load_file_dependency_arcs(
        &self,
        repo_id: &str,
        snapshot_key: &str,
    ) -> Result<Vec<CodeCityFileDependencyArc>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT arc_id, run_id, commit_sha, from_path, to_path, from_boundary_id,
                        to_boundary_id, from_zone, to_zone, edge_count, import_count,
                        call_count, reference_count, export_count, inheritance_count, weight,
                        cross_boundary, has_violation, highest_severity, evidence_ids_json
                 FROM codecity_file_dependency_arcs_current
                 WHERE repo_id = ?1 AND snapshot_key = ?2
                 ORDER BY from_path ASC, to_path ASC",
            )?;
            let rows = stmt.query_map(params![repo_id, snapshot_key], |row| {
                let evidence_ids: String = row.get(19)?;
                let severity: Option<String> = row.get(18)?;
                Ok(CodeCityFileDependencyArc {
                    arc_id: row.get(0)?,
                    run_id: row.get(1)?,
                    commit_sha: row.get(2)?,
                    from_path: row.get(3)?,
                    to_path: row.get(4)?,
                    from_boundary_id: row.get(5)?,
                    to_boundary_id: row.get(6)?,
                    from_zone: row.get(7)?,
                    to_zone: row.get(8)?,
                    edge_count: row.get::<_, i64>(9)? as usize,
                    import_count: row.get::<_, i64>(10)? as usize,
                    call_count: row.get::<_, i64>(11)? as usize,
                    reference_count: row.get::<_, i64>(12)? as usize,
                    export_count: row.get::<_, i64>(13)? as usize,
                    inheritance_count: row.get::<_, i64>(14)? as usize,
                    weight: row.get(15)?,
                    cross_boundary: sqlite_bool(row.get::<_, i64>(16)?),
                    has_violation: sqlite_bool(row.get::<_, i64>(17)?),
                    highest_severity: severity.and_then(|value| parse_severity(&value)),
                    evidence_ids: serde_json::from_str(&evidence_ids).unwrap_or_default(),
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })
    }

    fn load_architecture_violations(
        &self,
        repo_id: &str,
        snapshot_key: &str,
    ) -> Result<Vec<CodeCityArchitectureViolation>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT violation_id, run_id, commit_sha, boundary_id, boundary_root, pattern,
                        rule, severity, from_path, to_path, from_zone, to_zone,
                        from_boundary_id, to_boundary_id, arc_id, message, explanation,
                        recommendation, evidence_ids_json, evidence_json, confidence, suppressed
                 FROM codecity_architecture_violations_current
                 WHERE repo_id = ?1 AND snapshot_key = ?2
                 ORDER BY severity ASC, rule ASC, from_path ASC, to_path ASC",
            )?;
            let rows = stmt.query_map(params![repo_id, snapshot_key], |row| {
                let evidence_ids: String = row.get(18)?;
                let evidence_json: String = row.get(19)?;
                let pattern: String = row.get(5)?;
                let rule: String = row.get(6)?;
                let severity: String = row.get(7)?;
                Ok(CodeCityArchitectureViolation {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    commit_sha: row.get(2)?,
                    boundary_id: row.get(3)?,
                    boundary_root: row.get(4)?,
                    pattern: parse_pattern(&pattern).unwrap_or(CodeCityViolationPattern::Mud),
                    rule: parse_rule(&rule)
                        .unwrap_or(CodeCityViolationRule::CrossBoundaryHighCoupling),
                    severity: parse_severity(&severity).unwrap_or(CodeCityViolationSeverity::Info),
                    from_path: row.get(8)?,
                    to_path: row.get(9)?,
                    from_zone: row.get(10)?,
                    to_zone: row.get(11)?,
                    from_boundary_id: row.get(12)?,
                    to_boundary_id: row.get(13)?,
                    arc_id: row.get(14)?,
                    message: row.get(15)?,
                    explanation: row.get(16)?,
                    recommendation: row.get(17)?,
                    evidence_ids: serde_json::from_str(&evidence_ids).unwrap_or_default(),
                    evidence: serde_json::from_str::<Vec<CodeCityViolationEvidence>>(
                        &evidence_json,
                    )
                    .unwrap_or_default(),
                    confidence: row.get(20)?,
                    suppressed: sqlite_bool(row.get::<_, i64>(21)?),
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })
    }

    fn load_render_arcs(
        &self,
        repo_id: &str,
        snapshot_key: &str,
    ) -> Result<Vec<CodeCityRenderArc>> {
        self.sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT render_arc_id, arc_kind, visibility, severity, from_path, to_path,
                        from_boundary_id, to_boundary_id, source_arc_id, violation_id,
                        weight, label, tooltip, from_x, from_y, from_z, to_x, to_y, to_z,
                        control_y, metadata_json
                 FROM codecity_render_arcs_current
                 WHERE repo_id = ?1 AND snapshot_key = ?2
                 ORDER BY arc_kind ASC, render_arc_id ASC",
            )?;
            let rows = stmt.query_map(params![repo_id, snapshot_key], |row| {
                let kind: String = row.get(1)?;
                let visibility: String = row.get(2)?;
                let severity: Option<String> = row.get(3)?;
                Ok(CodeCityRenderArc {
                    id: row.get(0)?,
                    kind: parse_arc_kind(&kind).unwrap_or(CodeCityArcKind::Dependency),
                    visibility: parse_visibility(&visibility)
                        .unwrap_or(CodeCityArcVisibility::HiddenByDefault),
                    severity: severity.and_then(|value| parse_severity(&value)),
                    from_path: row.get(4)?,
                    to_path: row.get(5)?,
                    from_boundary_id: row.get(6)?,
                    to_boundary_id: row.get(7)?,
                    source_arc_id: row.get(8)?,
                    violation_id: row.get(9)?,
                    weight: row.get(10)?,
                    label: row.get(11)?,
                    tooltip: row.get(12)?,
                    geometry: CodeCityArcGeometry {
                        from_x: row.get(13)?,
                        from_y: row.get(14)?,
                        from_z: row.get(15)?,
                        to_x: row.get(16)?,
                        to_y: row.get(17)?,
                        to_z: row.get(18)?,
                        control_y: row.get(19)?,
                    },
                    metadata_json: row.get(20)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })
    }
}

fn sqlite_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn sqlite_bool(value: i64) -> bool {
    value != 0
}

fn parse_severity(value: &str) -> Option<CodeCityViolationSeverity> {
    match value {
        "high" => Some(CodeCityViolationSeverity::High),
        "medium" => Some(CodeCityViolationSeverity::Medium),
        "low" => Some(CodeCityViolationSeverity::Low),
        "info" => Some(CodeCityViolationSeverity::Info),
        _ => None,
    }
}

fn parse_pattern(value: &str) -> Option<CodeCityViolationPattern> {
    match value {
        "layered" => Some(CodeCityViolationPattern::Layered),
        "hexagonal" => Some(CodeCityViolationPattern::Hexagonal),
        "modular" => Some(CodeCityViolationPattern::Modular),
        "event_driven" => Some(CodeCityViolationPattern::EventDriven),
        "cross_boundary" => Some(CodeCityViolationPattern::CrossBoundary),
        "cycle" => Some(CodeCityViolationPattern::Cycle),
        "mud" => Some(CodeCityViolationPattern::Mud),
        _ => None,
    }
}

fn parse_rule(value: &str) -> Option<CodeCityViolationRule> {
    match value {
        "layered_upward_dependency" => Some(CodeCityViolationRule::LayeredUpwardDependency),
        "layered_skipped_layer" => Some(CodeCityViolationRule::LayeredSkippedLayer),
        "hexagonal_core_imports_periphery" => {
            Some(CodeCityViolationRule::HexagonalCoreImportsPeriphery)
        }
        "hexagonal_core_imports_external" => {
            Some(CodeCityViolationRule::HexagonalCoreImportsExternal)
        }
        "hexagonal_application_imports_edge" => {
            Some(CodeCityViolationRule::HexagonalApplicationImportsEdge)
        }
        "modular_internal_cross_module_dependency" => {
            Some(CodeCityViolationRule::ModularInternalCrossModuleDependency)
        }
        "modular_broad_bridge_file" => Some(CodeCityViolationRule::ModularBroadBridgeFile),
        "event_driven_direct_peer_dependency" => {
            Some(CodeCityViolationRule::EventDrivenDirectPeerDependency)
        }
        "cross_boundary_cycle" => Some(CodeCityViolationRule::CrossBoundaryCycle),
        "cross_boundary_high_coupling" => Some(CodeCityViolationRule::CrossBoundaryHighCoupling),
        _ => None,
    }
}

fn parse_arc_kind(value: &str) -> Option<CodeCityArcKind> {
    match value {
        "dependency" => Some(CodeCityArcKind::Dependency),
        "violation" => Some(CodeCityArcKind::Violation),
        "cross_boundary" => Some(CodeCityArcKind::CrossBoundary),
        "cycle" => Some(CodeCityArcKind::Cycle),
        "bridge" => Some(CodeCityArcKind::Bridge),
        _ => None,
    }
}

fn parse_visibility(value: &str) -> Option<CodeCityArcVisibility> {
    match value {
        "hidden_by_default" => Some(CodeCityArcVisibility::HiddenByDefault),
        "visible_on_selection" => Some(CodeCityArcVisibility::VisibleOnSelection),
        "visible_at_medium_zoom" => Some(CodeCityArcVisibility::VisibleAtMediumZoom),
        "visible_at_world_zoom" => Some(CodeCityArcVisibility::VisibleAtWorldZoom),
        "always_visible" => Some(CodeCityArcVisibility::AlwaysVisible),
        _ => None,
    }
}
