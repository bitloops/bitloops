pub mod knowledge;
pub mod semantic_clones;
pub mod test_harness;

use std::path::Path;

use crate::host::devql::capability_host::CapabilityPack;
use knowledge::KnowledgePack;
use semantic_clones::SemanticClonesPack;
use test_harness::TestHarnessPack;

pub fn builtin_packs(repo_root: &Path) -> anyhow::Result<Vec<Box<dyn CapabilityPack>>> {
    Ok(vec![
        Box::new(KnowledgePack::new()?),
        Box::new(TestHarnessPack::new(repo_root)),
        Box::new(SemanticClonesPack::new()?),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_include_knowledge_and_test_harness_packs() -> anyhow::Result<()> {
        let packs = builtin_packs(Path::new("."))?;
        let ids = packs
            .iter()
            .map(|pack| pack.descriptor().id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"knowledge"));
        assert!(ids.contains(&"test_harness"));
        assert!(ids.contains(&"semantic_clones"));
        Ok(())
    }
}
