use super::super::lifecycle::ExtensionCompatibility;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageProfileDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub language_id: &'static str,
    pub dialect: Option<&'static str>,
    pub aliases: &'static [&'static str],
    pub file_extensions: &'static [&'static str],
    pub supported_source_versions: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagePackDescriptor {
    pub id: &'static str,
    pub version: &'static str,
    pub api_version: u32,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub supported_languages: &'static [&'static str],
    pub language_profiles: &'static [LanguageProfileDescriptor],
    pub compatibility: ExtensionCompatibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguagePackRegistrationStatus {
    Registered,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagePackRegistrationObservation {
    pub pack_id: String,
    pub status: LanguagePackRegistrationStatus,
    pub reason: Option<String>,
}
