pub mod architecture_graph;
pub mod codecity;
pub mod knowledge;
pub mod navigation_context;
pub mod semantic_clones;
pub mod test_harness;

use std::path::Path;

use crate::host::capability_host::CapabilityPack;
use architecture_graph::ArchitectureGraphPack;
use codecity::CodeCityPack;
use knowledge::KnowledgePack;
use navigation_context::NavigationContextPack;
use semantic_clones::SemanticClonesPack;
use test_harness::TestHarnessPack;

pub fn builtin_packs(repo_root: &Path) -> anyhow::Result<Vec<Box<dyn CapabilityPack>>> {
    let _ = repo_root;
    Ok(vec![
        Box::new(ArchitectureGraphPack::new()),
        Box::new(CodeCityPack::new()),
        Box::new(KnowledgePack::new()?),
        Box::new(NavigationContextPack::new()),
        Box::new(TestHarnessPack::new()),
        Box::new(SemanticClonesPack::new()?),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_include_architecture_graph_codecity_knowledge_navigation_context_test_harness_and_semantic_clone_packs()
    -> anyhow::Result<()> {
        let packs = builtin_packs(Path::new("."))?;
        let ids = packs
            .iter()
            .map(|pack| pack.descriptor().id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"architecture_graph"));
        assert!(ids.contains(&"codecity"));
        assert!(ids.contains(&"knowledge"));
        assert!(ids.contains(&"navigation_context"));
        assert!(ids.contains(&"test_harness"));
        assert!(ids.contains(&"semantic_clones"));
        Ok(())
    }
}
