use crate::host::capability_host::SchemaModule;

use super::types::CODECITY_CAPABILITY_ID;

pub static CODECITY_SCHEMA_MODULE: SchemaModule = SchemaModule {
    capability_id: CODECITY_CAPABILITY_ID,
    name: "codecity",
    description: "CodeCity visualisation metrics, building geometry, floors, and dependency arcs.",
};
