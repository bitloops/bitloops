pub mod knowledge;

use crate::engine::devql::capability_host::CapabilityPack;
use knowledge::KnowledgePack;

pub fn builtin_packs() -> anyhow::Result<Vec<Box<dyn CapabilityPack>>> {
    Ok(vec![Box::new(KnowledgePack::new()?)])
}
