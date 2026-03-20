pub mod knowledge;

use crate::engine::devql::capability_host::CapabilityPack;
use knowledge::KnowledgePack;

pub fn builtin_packs() -> anyhow::Result<Vec<Box<dyn CapabilityPack>>> {
    Ok(vec![Box::new(KnowledgePack::new()?)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_include_knowledge_pack() -> anyhow::Result<()> {
        let packs = builtin_packs()?;
        let ids = packs
            .iter()
            .map(|pack| pack.descriptor().id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"knowledge"));
        Ok(())
    }
}
