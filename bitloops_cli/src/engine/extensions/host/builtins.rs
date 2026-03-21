use crate::engine::extensions::capability::{
    CapabilityDescriptor, CapabilityIngesterContribution, CapabilityPackDescriptor,
    CapabilityQueryExampleContribution, CapabilitySchemaModuleContribution,
    CapabilityStageContribution,
};
use crate::engine::extensions::language::{LanguagePackDescriptor, LanguageProfileDescriptor};
use crate::engine::extensions::lifecycle::ExtensionCompatibility;

pub(super) const LANGUAGE_PACK_FEATURES: &[&str] = &["language-packs", "readiness", "diagnostics"];
pub(super) const CAPABILITY_PACK_FEATURES: &[&str] = &[
    "capability-packs",
    "readiness",
    "diagnostics",
    "capability-migrations",
];

pub(super) const RUST_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "rust-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "Rust Language Pack",
    aliases: &["rust-pack"],
    supported_languages: &["rust"],
    language_profiles: &[LanguageProfileDescriptor {
        id: "rust-default",
        display_name: "Rust Default",
        language_id: "rust",
        dialect: None,
        aliases: &["rust-profile"],
        file_extensions: &["rs"],
        supported_source_versions: &["^1.70"],
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(LANGUAGE_PACK_FEATURES),
};

pub(super) const TS_JS_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "ts-js-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "TypeScript/JavaScript Language Pack",
    aliases: &["typescript-pack", "javascript-pack"],
    supported_languages: &["typescript", "javascript", "tsx", "jsx"],
    language_profiles: &[
        LanguageProfileDescriptor {
            id: "typescript-standard",
            display_name: "TypeScript Standard",
            language_id: "typescript",
            dialect: Some("ts"),
            aliases: &["ts"],
            file_extensions: &["ts", "tsx", "mts", "cts"],
            supported_source_versions: &["^5.0"],
        },
        LanguageProfileDescriptor {
            id: "javascript-standard",
            display_name: "JavaScript Standard",
            language_id: "javascript",
            dialect: Some("js"),
            aliases: &["js"],
            file_extensions: &["js", "jsx", "mjs", "cjs"],
            supported_source_versions: &[],
        },
    ],
    compatibility: ExtensionCompatibility::phase1_local_cli(LANGUAGE_PACK_FEATURES),
};

const KNOWLEDGE_CAPABILITY_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "knowledge-capability-pack",
    display_name: "Knowledge Capability Pack",
    version: "1.0.0",
    api_version: 1,
    description: "Knowledge retrieval and enrichment capability",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: CAPABILITY_PACK_FEATURES,
};

const TEST_HARNESS_CAPABILITY_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "test-harness-capability-pack",
    display_name: "Test Harness Capability Pack",
    version: "1.0.0",
    api_version: 1,
    description: "Test harness ingestion and verification capability",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: CAPABILITY_PACK_FEATURES,
};

pub(super) const KNOWLEDGE_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
    capability: KNOWLEDGE_CAPABILITY_DESCRIPTOR,
    aliases: &["knowledge-pack"],
    stage_contributions: &[CapabilityStageContribution { id: "knowledge" }],
    ingester_contributions: &[CapabilityIngesterContribution {
        id: "knowledge-ingester",
    }],
    schema_module_contributions: &[CapabilitySchemaModuleContribution {
        id: "knowledge-schema",
    }],
    query_example_contributions: &[CapabilityQueryExampleContribution {
        id: "knowledge-basic",
        query: "repo(\"bitloops\")->knowledge()->limit(10)",
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(CAPABILITY_PACK_FEATURES),
    migrations: &[],
};

pub(super) const TEST_HARNESS_CAPABILITY_PACK: CapabilityPackDescriptor =
    CapabilityPackDescriptor {
        capability: TEST_HARNESS_CAPABILITY_DESCRIPTOR,
        aliases: &["test-harness-pack"],
        stage_contributions: &[CapabilityStageContribution { id: "test-harness" }],
        ingester_contributions: &[CapabilityIngesterContribution {
            id: "test-harness-ingester",
        }],
        schema_module_contributions: &[CapabilitySchemaModuleContribution {
            id: "test-harness-schema",
        }],
        query_example_contributions: &[CapabilityQueryExampleContribution {
            id: "test-harness-basic",
            query: "repo(\"bitloops\")->testHarness()->limit(10)",
        }],
        compatibility: ExtensionCompatibility::phase1_local_cli(CAPABILITY_PACK_FEATURES),
        migrations: &[],
    };
