use super::descriptors::LanguagePackDescriptor;
use super::descriptors::LanguageProfileDescriptor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageResolutionSource {
    ProfileKey,
    LanguageId,
    FilePath,
    LanguageIdAndFilePath,
    LanguageProfile,
    LanguageProfileAndFilePath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageProfile<'a> {
    pub language_id: &'a str,
    pub dialect: Option<&'a str>,
    pub source_version: Option<&'a str>,
    pub project_root: Option<&'a str>,
    pub config_fingerprint: Option<&'a str>,
    pub runtime_profile: Option<&'a str>,
}

impl<'a> LanguageProfile<'a> {
    pub fn new(language_id: &'a str) -> Self {
        Self {
            language_id,
            dialect: None,
            source_version: None,
            project_root: None,
            config_fingerprint: None,
            runtime_profile: None,
        }
    }

    pub fn with_dialect(mut self, dialect: &'a str) -> Self {
        self.dialect = Some(dialect);
        self
    }

    pub fn with_source_version(mut self, source_version: &'a str) -> Self {
        self.source_version = Some(source_version);
        self
    }

    pub fn with_project_root(mut self, project_root: &'a str) -> Self {
        self.project_root = Some(project_root);
        self
    }

    pub fn with_config_fingerprint(mut self, config_fingerprint: &'a str) -> Self {
        self.config_fingerprint = Some(config_fingerprint);
        self
    }

    pub fn with_runtime_profile(mut self, runtime_profile: &'a str) -> Self {
        self.runtime_profile = Some(runtime_profile);
        self
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LanguagePackResolutionInput<'a> {
    pub language_id: Option<&'a str>,
    pub profile_key: Option<&'a str>,
    pub file_path: Option<&'a str>,
    pub language_profile: Option<LanguageProfile<'a>>,
}

impl<'a> LanguagePackResolutionInput<'a> {
    pub fn for_profile(profile_key: &'a str) -> Self {
        Self {
            language_id: None,
            profile_key: Some(profile_key),
            file_path: None,
            language_profile: None,
        }
    }

    pub fn for_language(language_id: &'a str) -> Self {
        Self {
            language_id: Some(language_id),
            profile_key: None,
            file_path: None,
            language_profile: None,
        }
    }

    pub fn for_file_path(file_path: &'a str) -> Self {
        Self {
            language_id: None,
            profile_key: None,
            file_path: Some(file_path),
            language_profile: None,
        }
    }

    pub fn for_language_profile(language_profile: LanguageProfile<'a>) -> Self {
        Self {
            language_id: None,
            profile_key: None,
            file_path: None,
            language_profile: Some(language_profile),
        }
    }

    pub fn with_file_path(mut self, file_path: &'a str) -> Self {
        self.file_path = Some(file_path);
        self
    }

    pub fn with_language_profile(mut self, language_profile: LanguageProfile<'a>) -> Self {
        self.language_profile = Some(language_profile);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLanguagePack<'a> {
    pub pack: &'a LanguagePackDescriptor,
    pub profile: &'a LanguageProfileDescriptor,
    pub source: LanguageResolutionSource,
}
