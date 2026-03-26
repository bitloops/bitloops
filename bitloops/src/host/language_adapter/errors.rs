#[derive(Debug)]
pub(crate) enum LanguageAdapterError {
    InvalidCanonicalMapping {
        pack_id: String,
        language_kind: String,
        reason: String,
    },
}

impl std::fmt::Display for LanguageAdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCanonicalMapping {
                pack_id,
                language_kind,
                reason,
            } => {
                write!(
                    f,
                    "invalid canonical mapping for pack `{pack_id}`, kind `{language_kind}`: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for LanguageAdapterError {}
