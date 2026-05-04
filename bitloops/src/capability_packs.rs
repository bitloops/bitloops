pub mod architecture_graph;
pub mod codecity;
pub mod context_guidance;
pub mod knowledge;
pub mod navigation_context;
pub mod semantic_clones;
pub mod test_harness;

use std::path::Path;

use crate::host::capability_host::CapabilityPack;
use architecture_graph::ArchitectureGraphPack;
use codecity::CodeCityPack;
use context_guidance::ContextGuidancePack;
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
        Box::new(ContextGuidancePack::new()?),
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

    #[test]
    fn builtin_packs_include_context_guidance_next_to_knowledge() -> anyhow::Result<()> {
        let packs = builtin_packs(Path::new("."))?;
        let ids = packs
            .iter()
            .map(|pack| pack.descriptor().id)
            .collect::<Vec<_>>();

        assert!(ids.contains(&"context_guidance"));
        let knowledge_index = ids.iter().position(|id| *id == "knowledge").unwrap();
        let context_guidance_index = ids.iter().position(|id| *id == "context_guidance").unwrap();
        let test_harness_index = ids.iter().position(|id| *id == "test_harness").unwrap();
        assert!(knowledge_index < context_guidance_index);
        assert!(context_guidance_index < test_harness_index);
        Ok(())
    }
}
