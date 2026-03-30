use std::sync::Arc;

use anyhow::Result;

use crate::capability_packs::test_harness::mapping::languages::rust::{
    RustLanguageProvider,
    enumeration::{parse_enumerated_doctests, parse_enumerated_host_tests},
};
use crate::capability_packs::test_harness::mapping::registry::LanguageProvider;
use crate::host::language_adapter::{
    DiscoveredTestFile, EnumerationMode, EnumerationResult, LanguageAdapterContext,
    LanguageTestSupport, ReconciledDiscovery,
};

#[derive(Default)]
pub(crate) struct RustLanguageTestSupport;

impl LanguageTestSupport for RustLanguageTestSupport {
    fn language_id(&self) -> &'static str {
        "rust"
    }

    fn priority(&self) -> u8 {
        0
    }

    fn supports_path(&self, absolute_path: &std::path::Path, relative_path: &str) -> bool {
        RustLanguageProvider::new()
            .map(|provider| provider.supports_path(absolute_path, relative_path))
            .unwrap_or(false)
    }

    fn discover_tests(
        &self,
        absolute_path: &std::path::Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        RustLanguageProvider::new()?.discover_tests(absolute_path, relative_path)
    }

    fn enumerate_tests(&self, ctx: &LanguageAdapterContext) -> EnumerationResult {
        if !ctx.repo_root.join("Cargo.toml").exists() {
            return EnumerationResult::default();
        }

        let host_output =
            ctx.run_command_capture("cargo", &["test", "--workspace", "--", "--list"]);
        let doc_output =
            ctx.run_command_capture("cargo", &["test", "--workspace", "--doc", "--", "--list"]);

        let mut result = EnumerationResult::default();
        let mut full_success = true;

        match host_output {
            Ok(output) if output.success => {
                result
                    .scenarios
                    .extend(parse_enumerated_host_tests(&output.combined_output));
            }
            Ok(output) => {
                full_success = false;
                result.notes.push(format!(
                    "host enumeration unavailable: {}",
                    output.combined_output.replace('\n', " ")
                ));
            }
            Err(error) => {
                full_success = false;
                result.notes.push(format!(
                    "host enumeration unavailable: {}",
                    error.to_string().replace('\n', " ")
                ));
            }
        }

        match doc_output {
            Ok(output) if output.success => {
                result
                    .scenarios
                    .extend(parse_enumerated_doctests(&output.combined_output));
            }
            Ok(output) => {
                full_success = false;
                result.notes.push(format!(
                    "doctest enumeration unavailable: {}",
                    output.combined_output.replace('\n', " ")
                ));
            }
            Err(error) => {
                full_success = false;
                result.notes.push(format!(
                    "doctest enumeration unavailable: {}",
                    error.to_string().replace('\n', " ")
                ));
            }
        }

        result.mode = if result.notes.is_empty() && full_success {
            EnumerationMode::Full
        } else if !result.scenarios.is_empty() {
            EnumerationMode::Partial
        } else {
            EnumerationMode::Skipped
        };
        result
    }

    fn reconcile(
        &self,
        source_files: &[DiscoveredTestFile],
        enumeration: EnumerationResult,
    ) -> ReconciledDiscovery {
        RustLanguageProvider::new()
            .map(|provider| provider.reconcile(source_files, enumeration))
            .unwrap_or_else(|_| ReconciledDiscovery::default())
    }
}

pub(crate) fn rust_test_support() -> Arc<dyn LanguageTestSupport> {
    Arc::new(RustLanguageTestSupport)
}
