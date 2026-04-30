mod initial;
mod lifecycle;

use crate::host::capability_host::CapabilityMigration;

pub static CONTEXT_GUIDANCE_MIGRATIONS: &[CapabilityMigration] = &[
    initial::CONTEXT_GUIDANCE_INITIAL_MIGRATION,
    lifecycle::CONTEXT_GUIDANCE_LIFECYCLE_MIGRATION,
];
