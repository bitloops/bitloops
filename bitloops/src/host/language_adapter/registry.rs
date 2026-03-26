use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use super::{
    CanonicalMapping, DependencyEdge, LanguageAdapterError, LanguageAdapterPack, LanguageArtefact,
};

pub(crate) struct LanguageAdapterRegistry {
    packs: HashMap<String, Arc<dyn LanguageAdapterPack>>,
}

impl LanguageAdapterRegistry {
    pub(crate) fn new() -> Self {
        Self {
            packs: HashMap::new(),
        }
    }

    pub(crate) fn register(
        &mut self,
        pack: Box<dyn LanguageAdapterPack>,
    ) -> Result<(), LanguageAdapterError> {
        let descriptor = pack.descriptor();
        let pack_id = descriptor.id.to_string();

        let supported = pack.supported_language_kinds();
        for mapping in pack.canonical_mappings() {
            if !supported.contains(&mapping.language_kind) {
                return Err(LanguageAdapterError::InvalidCanonicalMapping {
                    pack_id,
                    language_kind: mapping.language_kind.to_string(),
                    reason: "language_kind not in supported_language_kinds".to_string(),
                });
            }
        }

        self.packs.insert(pack_id, Arc::from(pack));
        Ok(())
    }

    pub(crate) fn with_builtins(
        packs: Vec<Box<dyn LanguageAdapterPack>>,
    ) -> Result<Self, LanguageAdapterError> {
        let mut registry = Self::new();
        for pack in packs {
            registry.register(pack)?;
        }
        Ok(registry)
    }

    pub(crate) fn get(&self, pack_id: &str) -> Option<Arc<dyn LanguageAdapterPack>> {
        self.packs.get(pack_id).cloned()
    }

    pub(crate) fn canonical_mappings_for(
        &self,
        pack_id: &str,
    ) -> Option<&'static [CanonicalMapping]> {
        self.packs.get(pack_id).map(|pack| pack.canonical_mappings())
    }

    pub(crate) fn extract_artefacts(
        &self,
        pack_id: &str,
        content: &str,
        path: &str,
    ) -> Result<Vec<LanguageArtefact>> {
        let pack = self
            .packs
            .get(pack_id)
            .ok_or_else(|| anyhow::anyhow!("language adapter pack `{pack_id}` not found"))?;
        pack.extract_artefacts(content, path)
    }

    pub(crate) fn extract_dependency_edges(
        &self,
        pack_id: &str,
        content: &str,
        path: &str,
        artefacts: &[LanguageArtefact],
    ) -> Result<Vec<DependencyEdge>> {
        let pack = self
            .packs
            .get(pack_id)
            .ok_or_else(|| anyhow::anyhow!("language adapter pack `{pack_id}` not found"))?;
        pack.extract_dependency_edges(content, path, artefacts)
    }

    pub(crate) fn extract_file_docstring(&self, pack_id: &str, content: &str) -> Option<String> {
        self.packs.get(pack_id)?.extract_file_docstring(content)
    }

    pub(crate) fn registered_pack_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.packs.keys().map(String::as_str).collect();
        ids.sort();
        ids
    }
}
