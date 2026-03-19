use anyhow::{Result, bail};

use super::descriptor::CapabilityDescriptor;
use super::health::{CapabilityHealthCheck, CapabilityHealthResult};
use super::migrations::CapabilityMigration;
use super::registrar::{CapabilityPack, CapabilityRegistrar};

pub fn validate_descriptor(descriptor: &CapabilityDescriptor) -> Result<()> {
    if descriptor.id.trim().is_empty() {
        bail!("capability descriptor id must not be empty");
    }
    if descriptor.display_name.trim().is_empty() {
        bail!("capability descriptor display name must not be empty");
    }
    if descriptor.version.trim().is_empty() {
        bail!("capability descriptor version must not be empty");
    }
    if descriptor.api_version == 0 {
        bail!("capability descriptor api_version must be greater than zero");
    }
    Ok(())
}

pub fn validate_pack(pack: &dyn CapabilityPack) -> Result<()> {
    validate_descriptor(pack.descriptor())
}

pub fn register_pack(
    registrar: &mut dyn CapabilityRegistrar,
    pack: &dyn CapabilityPack,
) -> Result<()> {
    validate_pack(pack)?;
    pack.register(registrar)
}

pub fn run_migrations(
    migrations: &[CapabilityMigration],
    ctx: &mut dyn super::contexts::CapabilityMigrationContext,
) -> Result<()> {
    for migration in migrations {
        (migration.run)(ctx)?;
    }
    Ok(())
}

pub fn run_health_checks(
    capability_id: &str,
    checks: &[CapabilityHealthCheck],
    ctx: &dyn super::contexts::CapabilityHealthContext,
) -> Vec<(String, CapabilityHealthResult)> {
    checks
        .iter()
        .map(|check| {
            (
                format!("{capability_id}.{}", check.name),
                (check.run)(ctx),
            )
        })
        .collect()
}
