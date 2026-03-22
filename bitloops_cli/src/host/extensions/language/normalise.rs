use std::path::Path;

use semver::Version;

use super::errors::LanguagePackRegistryError;

pub(super) fn normalise_identifier(
    value: &str,
    field: &'static str,
) -> Result<String, LanguagePackRegistryError> {
    let normalised = value.trim().to_ascii_lowercase();
    if normalised.is_empty() {
        return Err(LanguagePackRegistryError::InvalidIdentifier {
            field,
            value: value.to_string(),
        });
    }
    Ok(normalised)
}

pub(super) fn normalise_resolution_identifier(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .fold(String::new(), |mut acc, character| {
            if !character.is_whitespace() {
                acc.push(character);
            }
            acc
        })
}

pub(super) fn normalise_extension(
    value: &str,
    field: &'static str,
) -> Result<Option<String>, LanguagePackRegistryError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let without_dot = trimmed.trim_start_matches('.');
    if without_dot.is_empty() {
        return Err(LanguagePackRegistryError::InvalidIdentifier {
            field,
            value: value.to_string(),
        });
    }
    Ok(Some(without_dot.to_ascii_lowercase()))
}

pub(super) fn parse_source_version(value: &str) -> Option<Version> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(parsed) = Version::parse(trimmed) {
        return Some(parsed);
    }

    let mut segments = trimmed.split('.');
    let major = segments.next()?;
    let minor = segments.next();
    let patch = segments.next();

    if segments.next().is_some() {
        return None;
    }

    match (minor, patch) {
        (None, None) => Version::parse(&format!("{major}.0.0")).ok(),
        (Some(minor), None) => Version::parse(&format!("{major}.{minor}.0")).ok(),
        (Some(minor), Some(patch)) => Version::parse(&format!("{major}.{minor}.{patch}")).ok(),
        (None, Some(_)) => None,
    }
}

pub(super) fn extract_normalised_extension(path: &str) -> Option<String> {
    let extension = Path::new(path).extension()?.to_str()?;
    let normalised = extension.trim().to_ascii_lowercase();
    if normalised.is_empty() {
        None
    } else {
        Some(normalised)
    }
}
