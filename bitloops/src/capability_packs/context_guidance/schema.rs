use crate::host::capability_host::SchemaModule;

use super::descriptor::CONTEXT_GUIDANCE_CAPABILITY_ID;

pub static CONTEXT_GUIDANCE_SCHEMA_MODULE: SchemaModule = SchemaModule {
    capability_id: CONTEXT_GUIDANCE_CAPABILITY_ID,
    name: "context_guidance.schema",
    description: "Artefact-scoped context guidance facts, sources, targets, and distillation runs.",
};
