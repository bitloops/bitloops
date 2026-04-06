use std::sync::OnceLock;

use anyhow::Result;

use crate::adapters::languages::builtin_language_adapter_packs;
use crate::host::capability_host::gateways::LanguageServicesGateway;
use crate::host::extension_host::{CoreExtensionHost, LanguagePackResolutionInput};
use crate::host::language_adapter::LanguageAdapterRegistry;

pub struct BuiltinLanguageServicesGateway {
    extension_host: &'static CoreExtensionHost,
    registry: &'static LanguageAdapterRegistry,
}

impl LanguageServicesGateway for BuiltinLanguageServicesGateway {
    fn test_supports(
        &self,
    ) -> Vec<std::sync::Arc<dyn crate::host::language_adapter::LanguageTestSupport>> {
        self.registry.all_test_supports()
    }

    fn resolve_test_support_for_path(
        &self,
        relative_path: &str,
    ) -> Option<std::sync::Arc<dyn crate::host::language_adapter::LanguageTestSupport>> {
        let resolved = self
            .extension_host
            .language_packs()
            .resolve(LanguagePackResolutionInput::for_file_path(relative_path))
            .ok()?;
        self.registry.test_support_for_pack(resolved.pack.id)
    }
}

pub(super) fn builtin_language_services() -> Result<&'static BuiltinLanguageServicesGateway> {
    static SERVICES: OnceLock<Result<BuiltinLanguageServicesGateway, String>> = OnceLock::new();
    let service = SERVICES.get_or_init(|| {
        let extension_host = CoreExtensionHost::with_builtins().map_err(|err| err.to_string())?;
        let registry = LanguageAdapterRegistry::with_builtins(builtin_language_adapter_packs())
            .map_err(|err| err.to_string())?;
        Ok(BuiltinLanguageServicesGateway {
            extension_host: Box::leak(Box::new(extension_host)),
            registry: Box::leak(Box::new(registry)),
        })
    });

    match service {
        Ok(service) => Ok(service),
        Err(error) => anyhow::bail!("failed to initialise built-in language services: {error}"),
    }
}
