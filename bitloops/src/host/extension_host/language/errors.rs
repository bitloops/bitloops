use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguagePackRegistryError {
    InvalidIdentifier {
        field: &'static str,
        value: String,
    },
    MissingSupportedLanguages {
        pack_id: String,
    },
    MissingLanguageProfiles {
        pack_id: String,
    },
    DuplicatePackId {
        pack_id: String,
    },
    AliasConflict {
        alias: String,
        existing_pack_id: String,
        attempted_pack_id: String,
    },
    LanguageAlreadyOwned {
        language: String,
        existing_pack_id: String,
        attempted_pack_id: String,
    },
    DuplicateProfileId {
        profile_id: String,
        pack_id: String,
    },
    ProfileAliasConflict {
        alias: String,
        existing_profile_key: String,
        attempted_profile_key: String,
    },
    ProfileLanguageNotSupported {
        profile_id: String,
        language: String,
        pack_id: String,
    },
    InvalidPackVersion {
        pack_id: String,
        version: String,
    },
    InvalidProfileSourceVersionRequirement {
        pack_id: String,
        profile_id: String,
        requirement: String,
    },
}

impl Display for LanguagePackRegistryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field, value } => {
                write!(f, "invalid {field}: `{value}`")
            }
            Self::MissingSupportedLanguages { pack_id } => {
                write!(
                    f,
                    "language pack `{pack_id}` must declare at least one supported language"
                )
            }
            Self::MissingLanguageProfiles { pack_id } => {
                write!(
                    f,
                    "language pack `{pack_id}` must declare at least one language profile"
                )
            }
            Self::DuplicatePackId { pack_id } => {
                write!(f, "duplicate language pack id: `{pack_id}`")
            }
            Self::AliasConflict {
                alias,
                existing_pack_id,
                attempted_pack_id,
            } => {
                write!(
                    f,
                    "language pack alias `{alias}` is already owned by `{existing_pack_id}` (attempted `{attempted_pack_id}`)"
                )
            }
            Self::LanguageAlreadyOwned {
                language,
                existing_pack_id,
                attempted_pack_id,
            } => {
                write!(
                    f,
                    "language `{language}` is already owned by `{existing_pack_id}` (attempted `{attempted_pack_id}`)"
                )
            }
            Self::DuplicateProfileId {
                profile_id,
                pack_id,
            } => {
                write!(
                    f,
                    "language profile `{profile_id}` is duplicated within language pack `{pack_id}`"
                )
            }
            Self::ProfileAliasConflict {
                alias,
                existing_profile_key,
                attempted_profile_key,
            } => {
                write!(
                    f,
                    "language profile alias `{alias}` is already owned by `{existing_profile_key}` (attempted `{attempted_profile_key}`)"
                )
            }
            Self::ProfileLanguageNotSupported {
                profile_id,
                language,
                pack_id,
            } => {
                write!(
                    f,
                    "language profile `{profile_id}` in pack `{pack_id}` declares unsupported language `{language}`"
                )
            }
            Self::InvalidPackVersion { pack_id, version } => {
                write!(
                    f,
                    "language pack `{pack_id}` has invalid version `{version}` (expected semver)"
                )
            }
            Self::InvalidProfileSourceVersionRequirement {
                pack_id,
                profile_id,
                requirement,
            } => {
                write!(
                    f,
                    "language profile `{profile_id}` in pack `{pack_id}` has invalid source version requirement `{requirement}`"
                )
            }
        }
    }
}

impl Error for LanguagePackRegistryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguagePackResolutionError {
    MissingResolutionInput,
    UnsupportedLanguage {
        language: String,
    },
    UnsupportedProfile {
        profile_key: String,
    },
    UnsupportedFileExtension {
        extension: String,
    },
    UnsupportedDialect {
        language: String,
        dialect: String,
    },
    InvalidSourceVersion {
        source_version: String,
    },
    UnsupportedSourceVersion {
        language: String,
        source_version: String,
    },
    ProfileLanguageMismatch {
        profile_key: String,
        language: String,
    },
    AmbiguousLanguageProfiles {
        language: String,
        profile_ids: Vec<String>,
    },
    AmbiguousFileExtension {
        extension: String,
        profile_ids: Vec<String>,
    },
}

impl Display for LanguagePackResolutionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingResolutionInput => {
                write!(
                    f,
                    "language pack resolution requires a language id, profile key, or file path"
                )
            }
            Self::UnsupportedLanguage { language } => {
                write!(f, "no language pack handles language `{language}`")
            }
            Self::UnsupportedProfile { profile_key } => {
                write!(f, "no language profile matches `{profile_key}`")
            }
            Self::UnsupportedFileExtension { extension } => {
                write!(
                    f,
                    "no language profile handles file extension `{extension}`"
                )
            }
            Self::UnsupportedDialect { language, dialect } => {
                write!(
                    f,
                    "no language profile handles language `{language}` with dialect `{dialect}`"
                )
            }
            Self::InvalidSourceVersion { source_version } => {
                write!(
                    f,
                    "source version `{source_version}` is not a valid semantic version"
                )
            }
            Self::UnsupportedSourceVersion {
                language,
                source_version,
            } => {
                write!(
                    f,
                    "no language profile handles language `{language}` at source version `{source_version}`"
                )
            }
            Self::ProfileLanguageMismatch {
                profile_key,
                language,
            } => {
                write!(
                    f,
                    "language profile `{profile_key}` does not support language `{language}`"
                )
            }
            Self::AmbiguousLanguageProfiles {
                language,
                profile_ids,
            } => {
                write!(
                    f,
                    "language `{language}` resolves to multiple profiles: {}",
                    profile_ids.join(", ")
                )
            }
            Self::AmbiguousFileExtension {
                extension,
                profile_ids,
            } => {
                write!(
                    f,
                    "file extension `{extension}` resolves to multiple profiles: {}",
                    profile_ids.join(", ")
                )
            }
        }
    }
}

impl Error for LanguagePackResolutionError {}
