//! Core extension host: built-in language/capability packs, registration, migrations, readiness.

use std::collections::{HashMap, HashSet};

use super::capability::{CapabilityPackDescriptor, CapabilityPackRegistry};
use super::contexts::{CapabilityHealthContext, CapabilityMigrationContext};
use super::language::{LanguagePackDescriptor, LanguagePackRegistry};
use super::lifecycle::{
    CapabilityMigrationExecution, CapabilityMigrationRunReport, CapabilityMigrationStep,
    ExtensionDiagnostic, ExtensionDiagnosticKind, ExtensionDiagnosticSeverity,
    ExtensionLifecycleState, ExtensionReadinessFailure, ExtensionReadinessReport,
    ExtensionReadinessStatus, HostCompatibilityContext, orchestrate_capability_migrations,
};

pub(crate) mod builtins;
mod error;
mod readiness_snapshot;

pub use error::CoreExtensionHostError;
pub use readiness_snapshot::CoreExtensionHostReadinessSnapshot;

use builtins::{
    KNOWLEDGE_CAPABILITY_PACK, RUST_LANGUAGE_PACK, SEMANTIC_CLONES_CAPABILITY_PACK,
    TEST_HARNESS_CAPABILITY_PACK, TS_JS_LANGUAGE_PACK,
};

#[derive(Debug, Clone)]
pub struct CoreExtensionHost {
    compatibility_context: HostCompatibilityContext,
    language_packs: LanguagePackRegistry,
    capability_packs: CapabilityPackRegistry,
    diagnostics: Vec<ExtensionDiagnostic>,
    migrated_capability_packs: HashSet<String>,
    applied_migrations: Vec<CapabilityMigrationExecution>,
}

include!("host/impl_default.rs");
include!("host/core_extension_host_impl.rs");

#[cfg(test)]
mod tests;
