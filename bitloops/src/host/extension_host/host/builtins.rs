use crate::host::extension_host::capability::{
    CapabilityDescriptor, CapabilityIngesterContribution, CapabilityPackDescriptor,
    CapabilityQueryExampleContribution, CapabilitySchemaModuleContribution,
    CapabilityStageContribution,
};
use crate::host::extension_host::language::{LanguagePackDescriptor, LanguageProfileDescriptor};
use crate::host::extension_host::lifecycle::ExtensionCompatibility;

pub(super) const LANGUAGE_PACK_FEATURES: &[&str] = &["language-packs", "readiness", "diagnostics"];
pub(super) const CAPABILITY_PACK_FEATURES: &[&str] = &[
    "capability-packs",
    "readiness",
    "diagnostics",
    "capability-migrations",
];

pub(crate) const RUST_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
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

pub(crate) const TS_JS_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
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

pub(crate) const PYTHON_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "python-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "Python Language Pack",
    aliases: &["python-pack", "py-pack"],
    supported_languages: &["python"],
    language_profiles: &[LanguageProfileDescriptor {
        id: "python-standard",
        display_name: "Python Standard",
        language_id: "python",
        dialect: Some("py"),
        aliases: &["py"],
        file_extensions: &["py"],
        supported_source_versions: &["^3.10", "^3.11", "^3.12"],
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(LANGUAGE_PACK_FEATURES),
};

pub(crate) const GO_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "go-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "Go Language Pack",
    aliases: &["golang-pack", "go-pack"],
    supported_languages: &["go", "golang"],
    language_profiles: &[LanguageProfileDescriptor {
        id: "go-standard",
        display_name: "Go Standard",
        language_id: "go",
        dialect: Some("go"),
        aliases: &["golang"],
        file_extensions: &["go"],
        supported_source_versions: &["^1.22", "^1.23", "^1.24"],
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(LANGUAGE_PACK_FEATURES),
};

pub(crate) const JAVA_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "java-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "Java Language Pack",
    aliases: &["java-pack", "jdk-pack"],
    supported_languages: &["java"],
    language_profiles: &[LanguageProfileDescriptor {
        id: "java-standard",
        display_name: "Java Standard",
        language_id: "java",
        dialect: Some("java"),
        aliases: &["jdk", "jvm-java"],
        file_extensions: &["java"],
        supported_source_versions: &["^17", "^21"],
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(LANGUAGE_PACK_FEATURES),
};

pub(crate) const CSHARP_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "csharp-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "C# Language Pack",
    aliases: &["cs-pack", "dotnet-pack"],
    supported_languages: &["csharp", "c#"],
    language_profiles: &[LanguageProfileDescriptor {
        id: "csharp-standard",
        display_name: "C# Standard",
        language_id: "csharp",
        dialect: Some("cs"),
        aliases: &["cs", "dotnet"],
        file_extensions: &["cs"],
        supported_source_versions: &["^8.0", "^9.0", "^10.0", "^11.0", "^12.0", "^13.0"],
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(LANGUAGE_PACK_FEATURES),
};

pub(crate) const PHP_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "php-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "PHP Language Pack",
    aliases: &["php-pack"],
    supported_languages: &["php"],
    language_profiles: &[LanguageProfileDescriptor {
        id: "php-standard",
        display_name: "PHP Standard",
        language_id: "php",
        dialect: Some("php"),
        aliases: &["php-default"],
        file_extensions: &["php", "phtml", "php5", "php7", "php8"],
        supported_source_versions: &["^8.1", "^8.2", "^8.3", "^8.4"],
    }],
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
