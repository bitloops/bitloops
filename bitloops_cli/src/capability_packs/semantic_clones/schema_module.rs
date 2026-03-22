use crate::host::capability_host::SchemaModule;

use super::types::SEMANTIC_CLONES_CAPABILITY_ID;

pub static SEMANTIC_CLONES_SCHEMA_MODULE: SchemaModule = SchemaModule {
    capability_id: SEMANTIC_CLONES_CAPABILITY_ID,
    name: "semantic_clones.schema",
    description: "symbol_clone_edges and related clone-detection relational artefacts",
};
