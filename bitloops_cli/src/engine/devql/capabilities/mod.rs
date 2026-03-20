pub mod knowledge;
pub mod test_harness;

use crate::engine::devql::capability_host::CapabilityPack;
use knowledge::KnowledgePack;
use test_harness::TestHarnessPack;

pub fn builtin_packs() -> anyhow::Result<Vec<Box<dyn CapabilityPack>>> {
    Ok(vec![
        Box::new(KnowledgePack::new()?),
        Box::new(TestHarnessPack::new()?),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_include_knowledge_and_test_harness_packs() -> anyhow::Result<()> {
        let packs = builtin_packs()?;
        let ids = packs
            .iter()
            .map(|pack| pack.descriptor().id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"knowledge"));
        assert!(ids.contains(&"test_harness"));
        Ok(())
    }
}
