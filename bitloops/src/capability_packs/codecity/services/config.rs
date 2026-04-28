use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeCityConfig {
    pub importance: ImportanceConfig,
    pub height: HeightConfig,
    pub layout: LayoutConfig,
    pub colours: ColourConfig,
    pub exclusions: Vec<String>,
    pub include_dependency_arcs: bool,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColourConfig {
    pub no_data: String,
    pub healthy: String,
    pub moderate: String,
    pub high_risk: String,
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
                    config.include_dependency_arcs = value
                        .as_bool()
                        .ok_or_else(|| anyhow!("`include_dependency_arcs` must be a boolean"))?;
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
            "min_footprint": 2.0,
            "max_footprint": 20.0,
            "base_floor_height": 0.5,
            "loc_scale": 0.1,
            "max_height": 12.0,
        }))?;

        assert!(config.include_dependency_arcs);
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
}
