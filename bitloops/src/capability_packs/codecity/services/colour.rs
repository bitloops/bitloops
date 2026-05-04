use anyhow::{Result, bail};

use crate::capability_packs::codecity::services::config::ColourConfig;
use crate::capability_packs::codecity::types::HealthStatus;

pub fn colour_for_health(
    risk: Option<f64>,
    status: HealthStatus,
    palette: &ColourConfig,
) -> String {
    if matches!(
        status,
        HealthStatus::InsufficientData | HealthStatus::NotRequested
    ) {
        return palette.no_data.clone();
    }

    let Some(risk) = risk else {
        return palette.no_data.clone();
    };
    let risk = risk.clamp(0.0, 1.0);
    if risk <= 0.5 {
        let ratio = risk / 0.5;
        interpolate_hex(&palette.healthy, &palette.moderate, ratio)
            .unwrap_or_else(|_| palette.no_data.clone())
    } else {
        let ratio = (risk - 0.5) / 0.5;
        interpolate_hex(&palette.moderate, &palette.high_risk, ratio)
            .unwrap_or_else(|_| palette.no_data.clone())
    }
}

fn interpolate_hex(left: &str, right: &str, ratio: f64) -> Result<String> {
    let left = parse_hex_colour(left)?;
    let right = parse_hex_colour(right)?;
    let ratio = ratio.clamp(0.0, 1.0);
    let channel = |idx: usize| {
        (left[idx] as f64 + (right[idx] as f64 - left[idx] as f64) * ratio).round() as u8
    };
    Ok(format!(
        "#{:02X}{:02X}{:02X}",
        channel(0),
        channel(1),
        channel(2)
    ))
}

fn parse_hex_colour(value: &str) -> Result<[u8; 3]> {
    let Some(hex) = value.strip_prefix('#') else {
        bail!("colour must start with #");
    };
    if hex.len() != 6 {
        bail!("colour must be six hex digits");
    }
    let r = u8::from_str_radix(&hex[0..2], 16)?;
    let g = u8::from_str_radix(&hex[2..4], 16)?;
    let b = u8::from_str_radix(&hex[4..6], 16)?;
    Ok([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::colour_for_health;
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::capability_packs::codecity::types::HealthStatus;

    #[test]
    fn colour_for_health_uses_expected_stops() {
        let palette = &CodeCityConfig::default().colours;
        assert_eq!(
            colour_for_health(None, HealthStatus::InsufficientData, palette),
            "#888888"
        );
        assert_eq!(
            colour_for_health(Some(0.0), HealthStatus::Ok, palette),
            "#6B8FA3"
        );
        assert_eq!(
            colour_for_health(Some(0.5), HealthStatus::Ok, palette),
            "#D4A04A"
        );
        assert_eq!(
            colour_for_health(Some(1.0), HealthStatus::Ok, palette),
            "#C23B22"
        );
    }

    #[test]
    fn colour_for_health_interpolates_and_clamps() {
        let palette = &CodeCityConfig::default().colours;
        assert_eq!(
            colour_for_health(Some(-1.0), HealthStatus::Ok, palette),
            "#6B8FA3"
        );
        assert_eq!(
            colour_for_health(Some(2.0), HealthStatus::Ok, palette),
            "#C23B22"
        );
        assert_eq!(
            colour_for_health(Some(0.25), HealthStatus::Ok, palette),
            "#A09877"
        );
    }
}
