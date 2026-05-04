use super::config::{ColourConfig, HeightConfig};
use super::source_graph::{CodeCitySourceArtefact, CodeCitySourceFile};
use crate::capability_packs::codecity::types::{
    CodeCityFloor, CodeCityHealthEvidence, CodeCityHealthMetrics,
};

pub fn build_floors_for_file(
    file: &CodeCitySourceFile,
    artefacts: &[CodeCitySourceArtefact],
    height: &HeightConfig,
    colours: &ColourConfig,
) -> Vec<CodeCityFloor> {
    let file_artefact = artefacts.iter().find(|artefact| is_file_kind(artefact));
    let file_artefact_id = file_artefact.map(|artefact| artefact.artefact_id.as_str());

    let mut selected = artefacts
        .iter()
        .filter(|artefact| is_primary_floor_artefact(artefact, file_artefact_id))
        .cloned()
        .collect::<Vec<_>>();

    if selected.is_empty() {
        selected = artefacts
            .iter()
            .filter(|artefact| is_method_fallback_floor_artefact(artefact, file_artefact_id))
            .cloned()
            .collect::<Vec<_>>();
    }

    if selected.is_empty() {
        return vec![synthetic_file_floor(
            file,
            file_artefact,
            artefacts,
            height,
            colours,
        )];
    }

    selected.sort_by(|left, right| {
        left.start_line
            .cmp(&right.start_line)
            .then_with(|| left.end_line.cmp(&right.end_line))
            .then_with(|| left.symbol_id.cmp(&right.symbol_id))
    });

    selected
        .into_iter()
        .enumerate()
        .map(|(floor_index, artefact)| {
            let loc = line_span_loc(artefact.start_line, artefact.end_line);
            let name = artefact_name(&artefact);
            CodeCityFloor {
                artefact_id: Some(artefact.artefact_id),
                symbol_id: Some(artefact.symbol_id),
                name,
                canonical_kind: artefact.canonical_kind,
                language_kind: artefact.language_kind,
                start_line: artefact.start_line,
                end_line: artefact.end_line,
                loc,
                floor_index,
                floor_height: floor_height_for_loc(loc, height),
                health_risk: None,
                colour: colours.no_data.clone(),
                health_status: "insufficient_data".to_string(),
                health_confidence: 0.0,
                health_metrics: CodeCityHealthMetrics::default(),
                health_evidence: CodeCityHealthEvidence::default(),
            }
        })
        .collect()
}

pub fn total_height(floors: &[CodeCityFloor], height: &HeightConfig) -> f64 {
    soft_cap_height(
        floors.iter().map(|floor| floor.floor_height).sum::<f64>(),
        height.max_height,
    )
}

pub fn building_loc(floors: &[CodeCityFloor]) -> i64 {
    floors.iter().map(|floor| floor.loc.max(0)).sum()
}

fn synthetic_file_floor(
    file: &CodeCitySourceFile,
    file_artefact: Option<&CodeCitySourceArtefact>,
    artefacts: &[CodeCitySourceArtefact],
    height: &HeightConfig,
    colours: &ColourConfig,
) -> CodeCityFloor {
    let (start_line, end_line) = if let Some(file_artefact) = file_artefact {
        (file_artefact.start_line, file_artefact.end_line)
    } else {
        let end_line = artefacts
            .iter()
            .map(|artefact| artefact.end_line)
            .max()
            .unwrap_or(1)
            .max(1);
        (1, end_line)
    };
    let loc = line_span_loc(start_line, end_line);

    CodeCityFloor {
        artefact_id: None,
        symbol_id: None,
        name: file.path.clone(),
        canonical_kind: Some("file".to_string()),
        language_kind: None,
        start_line,
        end_line,
        loc,
        floor_index: 0,
        floor_height: floor_height_for_loc(loc, height),
        health_risk: None,
        colour: colours.no_data.clone(),
        health_status: "insufficient_data".to_string(),
        health_confidence: 0.0,
        health_metrics: CodeCityHealthMetrics::default(),
        health_evidence: CodeCityHealthEvidence::default(),
    }
}

fn is_primary_floor_artefact(
    artefact: &CodeCitySourceArtefact,
    file_artefact_id: Option<&str>,
) -> bool {
    let kind = normalised_kind(artefact);
    if kind.is_empty()
        || matches!(
            kind.as_str(),
            "file" | "import" | "import_statement" | "call" | "reference" | "field"
        )
    {
        return false;
    }
    if matches!(kind.as_str(), "method" | "constructor") {
        return false;
    }
    if !matches!(
        kind.as_str(),
        "class"
            | "function"
            | "struct"
            | "enum"
            | "trait"
            | "interface"
            | "type"
            | "module"
            | "const"
            | "static"
            | "variable"
            | "impl"
    ) {
        return false;
    }

    matches_top_level_parent(artefact, file_artefact_id)
}

fn is_method_fallback_floor_artefact(
    artefact: &CodeCitySourceArtefact,
    file_artefact_id: Option<&str>,
) -> bool {
    let kind = normalised_kind(artefact);
    matches!(kind.as_str(), "method" | "constructor")
        && matches_top_level_parent(artefact, file_artefact_id)
}

fn matches_top_level_parent(
    artefact: &CodeCitySourceArtefact,
    file_artefact_id: Option<&str>,
) -> bool {
    artefact.parent_artefact_id.is_none()
        || artefact.parent_artefact_id.as_deref() == file_artefact_id
}

fn normalised_kind(artefact: &CodeCitySourceArtefact) -> String {
    artefact
        .canonical_kind
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn is_file_kind(artefact: &CodeCitySourceArtefact) -> bool {
    normalised_kind(artefact) == "file"
}

fn artefact_name(artefact: &CodeCitySourceArtefact) -> String {
    if let Some(symbol_fqn) = artefact.symbol_fqn.as_deref() {
        let tail = symbol_fqn.rsplit("::").next().unwrap_or(symbol_fqn);
        return tail.rsplit('.').next().unwrap_or(tail).to_string();
    }

    artefact
        .canonical_kind
        .clone()
        .unwrap_or_else(|| artefact.artefact_id.clone())
}

fn floor_height_for_loc(loc: i64, height: &HeightConfig) -> f64 {
    if loc <= 0 {
        return 0.0;
    }
    height.base_floor_height + height.loc_scale * loc as f64
}

fn line_span_loc(start_line: i64, end_line: i64) -> i64 {
    (end_line - start_line + 1).max(1)
}

fn soft_cap_height(total_height: f64, max_height: f64) -> f64 {
    if total_height <= max_height {
        total_height
    } else {
        max_height + (total_height - max_height + 1.0).log2()
    }
}

#[cfg(test)]
mod tests {
    use super::{build_floors_for_file, total_height};
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::capability_packs::codecity::services::source_graph::{
        CodeCitySourceArtefact, CodeCitySourceFile,
    };

    fn file(path: &str) -> CodeCitySourceFile {
        CodeCitySourceFile {
            path: path.to_string(),
            language: "typescript".to_string(),
            effective_content_id: format!("content::{path}"),
            included: true,
            exclusion_reason: None,
        }
    }

    fn artefact(
        path: &str,
        artefact_id: &str,
        symbol_id: &str,
        symbol_fqn: &str,
        canonical_kind: &str,
        parent_artefact_id: Option<&str>,
        line_span: (i64, i64),
    ) -> CodeCitySourceArtefact {
        let (start_line, end_line) = line_span;
        CodeCitySourceArtefact {
            artefact_id: artefact_id.to_string(),
            symbol_id: symbol_id.to_string(),
            path: path.to_string(),
            symbol_fqn: Some(symbol_fqn.to_string()),
            canonical_kind: Some(canonical_kind.to_string()),
            language_kind: Some("fixture".to_string()),
            parent_artefact_id: parent_artefact_id.map(str::to_string),
            parent_symbol_id: None,
            signature: None,
            start_line,
            end_line,
        }
    }

    #[test]
    fn one_artefact_uses_line_span_formula() {
        let config = CodeCityConfig::default();
        let floors = build_floors_for_file(
            &file("src/core.ts"),
            &[artefact(
                "src/core.ts",
                "artefact::core",
                "sym::core",
                "src/core.ts::Core",
                "class",
                None,
                (1, 10),
            )],
            &config.height,
            &config.colours,
        );

        assert_eq!(floors.len(), 1);
        assert_eq!(floors[0].loc, 10);
        assert!((floors[0].floor_height - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn total_height_sums_multiple_floors() {
        let config = CodeCityConfig::default();
        let floors = build_floors_for_file(
            &file("src/core.ts"),
            &[
                artefact(
                    "src/core.ts",
                    "artefact::first",
                    "sym::first",
                    "src/core.ts::First",
                    "function",
                    None,
                    (1, 10),
                ),
                artefact(
                    "src/core.ts",
                    "artefact::second",
                    "sym::second",
                    "src/core.ts::Second",
                    "function",
                    None,
                    (11, 15),
                ),
            ],
            &config.height,
            &config.colours,
        );

        let total = total_height(&floors, &config.height);
        assert!((total - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn total_height_soft_caps_outliers() {
        let mut config = CodeCityConfig::default();
        config.height.max_height = 1.0;
        let floors = build_floors_for_file(
            &file("src/core.ts"),
            &[artefact(
                "src/core.ts",
                "artefact::giant",
                "sym::giant",
                "src/core.ts::Giant",
                "function",
                None,
                (1, 400),
            )],
            &config.height,
            &config.colours,
        );

        let total = total_height(&floors, &config.height);
        assert!(total > 1.0);
        assert!(total < floors[0].floor_height);
    }

    #[test]
    fn methods_are_excluded_when_a_higher_level_parent_exists() {
        let config = CodeCityConfig::default();
        let floors = build_floors_for_file(
            &file("src/core.ts"),
            &[
                artefact(
                    "src/core.ts",
                    "artefact::file",
                    "sym::file",
                    "src/core.ts",
                    "file",
                    None,
                    (1, 20),
                ),
                artefact(
                    "src/core.ts",
                    "artefact::class",
                    "sym::class",
                    "src/core.ts::Core",
                    "class",
                    Some("artefact::file"),
                    (1, 20),
                ),
                artefact(
                    "src/core.ts",
                    "artefact::method",
                    "sym::method",
                    "src/core.ts::Core.method",
                    "method",
                    Some("artefact::class"),
                    (4, 8),
                ),
            ],
            &config.height,
            &config.colours,
        );

        assert_eq!(floors.len(), 1);
        assert_eq!(floors[0].name, "Core");
    }

    #[test]
    fn creates_a_synthetic_floor_when_no_addressable_artefacts_remain() {
        let config = CodeCityConfig::default();
        let floors = build_floors_for_file(
            &file("src/generated.ts"),
            &[artefact(
                "src/generated.ts",
                "artefact::import",
                "sym::import",
                "src/generated.ts::Import",
                "import",
                None,
                (1, 1),
            )],
            &config.height,
            &config.colours,
        );

        assert_eq!(floors.len(), 1);
        assert_eq!(floors[0].canonical_kind.as_deref(), Some("file"));
        assert_eq!(floors[0].health_status, "insufficient_data");
    }
}
