use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::capability_packs::codecity::types::CodeCityZone;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityConfig {
    pub importance: ImportanceConfig,
    pub height: HeightConfig,
    pub layout: LayoutConfig,
    pub colours: ColourConfig,
    pub exclusions: Vec<String>,
    pub include_dependency_arcs: bool,
    pub include_boundaries: bool,
    pub include_architecture: bool,
    pub include_macro_edges: bool,
    pub include_zone_diagnostics: bool,
    pub boundaries: CodeCityBoundaryConfig,
    pub architecture: CodeCityArchitectureConfig,
    pub zones: CodeCityZoneConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportanceConfig {
    pub blast_radius_weight: f64,
    pub weighted_fan_in_weight: f64,
    pub articulation_score_weight: f64,
    pub pagerank_damping: f64,
    pub pagerank_threshold: f64,
    pub min_footprint: f64,
    pub max_footprint: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeightConfig {
    pub base_floor_height: f64,
    pub loc_scale: f64,
    pub max_height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutConfig {
    pub building_gap: f64,
    pub building_padding: f64,
    pub target_aspect_ratio: f64,
    pub world_gap: f64,
    pub zone_gap: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColourConfig {
    pub no_data: String,
    pub healthy: String,
    pub moderate: String,
    pub high_risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityBoundaryConfig {
    pub manifest_files: Vec<String>,
    pub entry_point_patterns: Vec<String>,
    pub overlap_split_threshold: f64,
    pub overlap_merge_threshold: f64,
    pub community_modularity_threshold: f64,
    pub shared_library_fan_in_percentile: f64,
    pub shared_library_fan_out_percentile: f64,
    pub small_cluster_collapse_file_limit: usize,
    pub min_runtime_boundary_files: usize,
    pub min_implicit_boundary_files: usize,
    pub community_max_iterations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityArchitectureConfig {
    pub enabled: bool,
    pub mud_warning_threshold: f64,
    pub secondary_pattern_threshold: f64,
    pub message_infra_libraries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityZoneConfig {
    pub zone_overrides: Vec<CodeCityZoneOverride>,
    pub conventions: CodeCityZoneConventions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeCityZoneOverride {
    pub pattern: String,
    pub zone: CodeCityZone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityZoneConventions {
    pub core: Vec<String>,
    pub application: Vec<String>,
    pub periphery: Vec<String>,
    pub edge: Vec<String>,
    pub ports: Vec<String>,
}

impl Default for CodeCityConfig {
    fn default() -> Self {
        Self {
            importance: ImportanceConfig {
                blast_radius_weight: 0.50,
                weighted_fan_in_weight: 0.35,
                articulation_score_weight: 0.15,
                pagerank_damping: 0.85,
                pagerank_threshold: 1e-6,
                min_footprint: 1.0,
                max_footprint: 12.0,
            },
            height: HeightConfig {
                base_floor_height: 0.3,
                loc_scale: 0.02,
                max_height: 80.0,
            },
            layout: LayoutConfig {
                building_gap: 0.5,
                building_padding: 0.25,
                target_aspect_ratio: 1.35,
                world_gap: 8.0,
                zone_gap: 2.0,
            },
            colours: ColourConfig {
                no_data: "#888888".to_string(),
                healthy: "#6B8FA3".to_string(),
                moderate: "#D4A04A".to_string(),
                high_risk: "#C23B22".to_string(),
            },
            exclusions: vec![
                "vendor/**".to_string(),
                "node_modules/**".to_string(),
                "**/*.generated.*".to_string(),
                "**/*_test.*".to_string(),
                "**/*.spec.*".to_string(),
            ],
            include_dependency_arcs: false,
            include_boundaries: true,
            include_architecture: true,
            include_macro_edges: true,
            include_zone_diagnostics: true,
            boundaries: CodeCityBoundaryConfig {
                manifest_files: vec![
                    "package.json".to_string(),
                    "go.mod".to_string(),
                    "Cargo.toml".to_string(),
                    "pom.xml".to_string(),
                    "build.gradle".to_string(),
                    "build.gradle.kts".to_string(),
                    "*.csproj".to_string(),
                    "*.fsproj".to_string(),
                    "*.sln".to_string(),
                    "pyproject.toml".to_string(),
                    "setup.py".to_string(),
                    "setup.cfg".to_string(),
                    "BUILD".to_string(),
                    "BUILD.bazel".to_string(),
                    "project.json".to_string(),
                    "lerna.json".to_string(),
                    "nx.json".to_string(),
                    "pnpm-workspace.yaml".to_string(),
                ],
                entry_point_patterns: vec![
                    "main.go".to_string(),
                    "main.py".to_string(),
                    "index.ts".to_string(),
                    "index.js".to_string(),
                    "app.py".to_string(),
                    "App.java".to_string(),
                    "Program.cs".to_string(),
                    "src/main.rs".to_string(),
                    "bin/*".to_string(),
                    "cmd/*/main.go".to_string(),
                ],
                overlap_split_threshold: 0.3,
                overlap_merge_threshold: 0.7,
                community_modularity_threshold: 0.4,
                shared_library_fan_in_percentile: 75.0,
                shared_library_fan_out_percentile: 25.0,
                small_cluster_collapse_file_limit: 50,
                min_runtime_boundary_files: 3,
                min_implicit_boundary_files: 5,
                community_max_iterations: 24,
            },
            architecture: CodeCityArchitectureConfig {
                enabled: true,
                mud_warning_threshold: 0.4,
                secondary_pattern_threshold: 0.3,
                message_infra_libraries: vec![
                    "kafka".to_string(),
                    "rabbitmq".to_string(),
                    "amqp".to_string(),
                    "nats".to_string(),
                    "eventemitter".to_string(),
                    "mediatr".to_string(),
                    "redux".to_string(),
                    "rxjs".to_string(),
                ],
            },
            zones: CodeCityZoneConfig {
                zone_overrides: Vec::new(),
                conventions: CodeCityZoneConventions {
                    core: vec![
                        "domain".to_string(),
                        "core".to_string(),
                        "model".to_string(),
                        "entities".to_string(),
                    ],
                    application: vec![
                        "application".to_string(),
                        "services".to_string(),
                        "usecases".to_string(),
                        "use-cases".to_string(),
                        "use_cases".to_string(),
                        "commands".to_string(),
                        "queries".to_string(),
                    ],
                    periphery: vec![
                        "adapters".to_string(),
                        "infrastructure".to_string(),
                        "infra".to_string(),
                        "repositories".to_string(),
                        "persistence".to_string(),
                        "gateways".to_string(),
                        "providers".to_string(),
                    ],
                    edge: vec![
                        "controllers".to_string(),
                        "handlers".to_string(),
                        "routes".to_string(),
                        "api".to_string(),
                        "cli".to_string(),
                        "ui".to_string(),
                        "views".to_string(),
                        "pages".to_string(),
                    ],
                    ports: vec![
                        "ports".to_string(),
                        "interfaces".to_string(),
                        "contracts".to_string(),
                    ],
                },
            },
        }
    }
}

impl CodeCityConfig {
    pub fn from_stage_args(args: &Value) -> Result<Self> {
        let entries = match args {
            Value::Null => return Ok(Self::default()),
            Value::Object(entries) => entries,
            _ => bail!("codecity args must be a JSON object"),
        };

        let mut config = Self::default();
        for (key, value) in entries {
            match key.as_str() {
                "include_dependency_arcs" => {
                    config.include_dependency_arcs = bool_arg("include_dependency_arcs", value)?;
                }
                "include_boundaries" => {
                    config.include_boundaries = bool_arg("include_boundaries", value)?;
                }
                "include_architecture" => {
                    config.include_architecture = bool_arg("include_architecture", value)?;
                }
                "include_macro_edges" => {
                    config.include_macro_edges = bool_arg("include_macro_edges", value)?;
                }
                "include_zone_diagnostics" => {
                    config.include_zone_diagnostics = bool_arg("include_zone_diagnostics", value)?;
                }
                "architecture_enabled" => {
                    config.architecture.enabled = bool_arg("architecture_enabled", value)?;
                }
                "min_footprint" => {
                    config.importance.min_footprint = numeric_arg("min_footprint", value)?;
                }
                "max_footprint" => {
                    config.importance.max_footprint = numeric_arg("max_footprint", value)?;
                }
                "base_floor_height" => {
                    config.height.base_floor_height = numeric_arg("base_floor_height", value)?;
                }
                "loc_scale" => {
                    config.height.loc_scale = numeric_arg("loc_scale", value)?;
                }
                "max_height" => {
                    config.height.max_height = numeric_arg("max_height", value)?;
                }
                other => bail!("unknown codecity arg `{other}`"),
            }
        }

        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        for (name, value) in [
            ("blast_radius_weight", self.importance.blast_radius_weight),
            (
                "weighted_fan_in_weight",
                self.importance.weighted_fan_in_weight,
            ),
            (
                "articulation_score_weight",
                self.importance.articulation_score_weight,
            ),
            ("pagerank_damping", self.importance.pagerank_damping),
            ("pagerank_threshold", self.importance.pagerank_threshold),
            ("min_footprint", self.importance.min_footprint),
            ("max_footprint", self.importance.max_footprint),
            ("base_floor_height", self.height.base_floor_height),
            ("loc_scale", self.height.loc_scale),
            ("max_height", self.height.max_height),
            ("building_gap", self.layout.building_gap),
            ("building_padding", self.layout.building_padding),
            ("target_aspect_ratio", self.layout.target_aspect_ratio),
            ("world_gap", self.layout.world_gap),
            ("zone_gap", self.layout.zone_gap),
            (
                "overlap_split_threshold",
                self.boundaries.overlap_split_threshold,
            ),
            (
                "overlap_merge_threshold",
                self.boundaries.overlap_merge_threshold,
            ),
            (
                "community_modularity_threshold",
                self.boundaries.community_modularity_threshold,
            ),
            (
                "shared_library_fan_in_percentile",
                self.boundaries.shared_library_fan_in_percentile,
            ),
            (
                "shared_library_fan_out_percentile",
                self.boundaries.shared_library_fan_out_percentile,
            ),
            (
                "mud_warning_threshold",
                self.architecture.mud_warning_threshold,
            ),
            (
                "secondary_pattern_threshold",
                self.architecture.secondary_pattern_threshold,
            ),
        ] {
            if !value.is_finite() {
                bail!("`{name}` must be finite");
            }
        }

        for (name, value) in [
            ("blast_radius_weight", self.importance.blast_radius_weight),
            (
                "weighted_fan_in_weight",
                self.importance.weighted_fan_in_weight,
            ),
            (
                "articulation_score_weight",
                self.importance.articulation_score_weight,
            ),
            ("pagerank_threshold", self.importance.pagerank_threshold),
            ("base_floor_height", self.height.base_floor_height),
            ("loc_scale", self.height.loc_scale),
            ("building_gap", self.layout.building_gap),
            ("building_padding", self.layout.building_padding),
            ("world_gap", self.layout.world_gap),
            ("zone_gap", self.layout.zone_gap),
        ] {
            if value < 0.0 {
                bail!("`{name}` must be non-negative");
            }
        }

        if self.importance.min_footprint <= 0.0 {
            bail!("`min_footprint` must be greater than 0");
        }
        if self.importance.max_footprint < self.importance.min_footprint {
            bail!("`max_footprint` must be greater than or equal to `min_footprint`");
        }
        if self.height.max_height <= 0.0 {
            bail!("`max_height` must be greater than 0");
        }
        if self.layout.target_aspect_ratio <= 0.0 {
            bail!("`target_aspect_ratio` must be greater than 0");
        }
        if !(0.0..=1.0).contains(&self.boundaries.overlap_split_threshold) {
            bail!("`overlap_split_threshold` must be between 0 and 1");
        }
        if !(0.0..=1.0).contains(&self.boundaries.overlap_merge_threshold) {
            bail!("`overlap_merge_threshold` must be between 0 and 1");
        }
        if !(0.0..=1.0).contains(&self.boundaries.community_modularity_threshold) {
            bail!("`community_modularity_threshold` must be between 0 and 1");
        }
        if !(0.0..=100.0).contains(&self.boundaries.shared_library_fan_in_percentile) {
            bail!("`shared_library_fan_in_percentile` must be between 0 and 100");
        }
        if !(0.0..=100.0).contains(&self.boundaries.shared_library_fan_out_percentile) {
            bail!("`shared_library_fan_out_percentile` must be between 0 and 100");
        }
        if !(0.0..=1.0).contains(&self.architecture.mud_warning_threshold) {
            bail!("`mud_warning_threshold` must be between 0 and 1");
        }
        if !(0.0..=1.0).contains(&self.architecture.secondary_pattern_threshold) {
            bail!("`secondary_pattern_threshold` must be between 0 and 1");
        }
        if self.boundaries.min_runtime_boundary_files == 0 {
            bail!("`min_runtime_boundary_files` must be greater than 0");
        }
        if self.boundaries.min_implicit_boundary_files == 0 {
            bail!("`min_implicit_boundary_files` must be greater than 0");
        }
        if self.boundaries.community_max_iterations == 0 {
            bail!("`community_max_iterations` must be greater than 0");
        }

        Ok(())
    }

    pub fn fingerprint(&self) -> Result<String> {
        let encoded = serde_json::to_vec(self)?;
        let mut hasher = Sha256::new();
        hasher.update(encoded);
        Ok(hex::encode(hasher.finalize()))
    }
}

fn numeric_arg(name: &str, value: &Value) -> Result<f64> {
    value
        .as_f64()
        .ok_or_else(|| anyhow!("`{name}` must be a number"))
}

fn bool_arg(name: &str, value: &Value) -> Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| anyhow!("`{name}` must be a boolean"))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::json;

    use super::CodeCityConfig;

    #[test]
    fn default_config_validates() -> Result<()> {
        CodeCityConfig::default().validate()
    }

    #[test]
    fn invalid_footprint_range_is_rejected() {
        let err = CodeCityConfig::from_stage_args(&json!({
            "min_footprint": 5.0,
            "max_footprint": 4.0,
        }))
        .expect_err("invalid footprint range must fail");

        assert!(err.to_string().contains("`max_footprint`"));
    }

    #[test]
    fn scalar_stage_args_override_defaults() -> Result<()> {
        let config = CodeCityConfig::from_stage_args(&json!({
            "include_dependency_arcs": true,
            "include_boundaries": false,
            "include_architecture": false,
            "include_macro_edges": false,
            "include_zone_diagnostics": false,
            "architecture_enabled": false,
            "min_footprint": 2.0,
            "max_footprint": 20.0,
            "base_floor_height": 0.5,
            "loc_scale": 0.1,
            "max_height": 12.0,
        }))?;

        assert!(config.include_dependency_arcs);
        assert!(!config.include_boundaries);
        assert!(!config.include_architecture);
        assert!(!config.include_macro_edges);
        assert!(!config.include_zone_diagnostics);
        assert!(!config.architecture.enabled);
        assert_eq!(config.importance.min_footprint, 2.0);
        assert_eq!(config.importance.max_footprint, 20.0);
        assert_eq!(config.height.base_floor_height, 0.5);
        assert_eq!(config.height.loc_scale, 0.1);
        assert_eq!(config.height.max_height, 12.0);
        Ok(())
    }

    #[test]
    fn unknown_stage_args_are_rejected() {
        let err = CodeCityConfig::from_stage_args(&json!({
            "unsupported": true
        }))
        .expect_err("unknown args must fail");

        assert!(err.to_string().contains("unknown codecity arg"));
    }

    #[test]
    fn defaults_include_valid_phase_two_thresholds() -> Result<()> {
        let config = CodeCityConfig::default();
        assert_eq!(config.boundaries.overlap_split_threshold, 0.3);
        assert_eq!(config.boundaries.overlap_merge_threshold, 0.7);
        assert_eq!(config.architecture.mud_warning_threshold, 0.4);
        config.validate()
    }
}
