impl Default for CoreExtensionHost {
    fn default() -> Self {
        Self {
            compatibility_context: HostCompatibilityContext::default(),
            language_packs: LanguagePackRegistry::new(),
            capability_packs: CapabilityPackRegistry::new(),
            diagnostics: Vec::new(),
            migrated_capability_packs: HashSet::new(),
            applied_migrations: Vec::new(),
        }
    }
}
