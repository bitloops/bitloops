use std::path::Path;

use anyhow::Result as AnyhowResult;

use super::models::{
    CapabilityPackDependencyObject, CapabilityPackInferenceSlotObject, PackMetadata, PackUiSpec,
};
use crate::capability_packs::builtin_packs;

const PACK_UI_SPECS: &[PackUiSpec] = &[
    PackUiSpec {
        pack_id: "architecture_graph",
        dashboard_configurable: true,
        display_order: 10,
    },
    PackUiSpec {
        pack_id: "context_guidance",
        dashboard_configurable: true,
        display_order: 20,
    },
    PackUiSpec {
        pack_id: "semantic_clones",
        dashboard_configurable: true,
        display_order: 30,
    },
    PackUiSpec {
        pack_id: "knowledge",
        dashboard_configurable: true,
        display_order: 40,
    },
    PackUiSpec {
        pack_id: "test_harness",
        dashboard_configurable: true,
        display_order: 50,
    },
    PackUiSpec {
        pack_id: "codecity",
        dashboard_configurable: false,
        display_order: 90,
    },
    PackUiSpec {
        pack_id: "http",
        dashboard_configurable: false,
        display_order: 100,
    },
    PackUiSpec {
        pack_id: "navigation_context",
        dashboard_configurable: false,
        display_order: 110,
    },
];

pub(super) fn pack_metadata(repo_root: &Path) -> AnyhowResult<Vec<PackMetadata>> {
    builtin_packs(repo_root).map(|packs| {
        packs
            .into_iter()
            .map(|pack| {
                let descriptor = pack.descriptor();
                PackMetadata {
                    id: descriptor.id.to_string(),
                    display_name: descriptor.display_name.to_string(),
                    description: descriptor.description.to_string(),
                    version: descriptor.version.to_string(),
                    default_enabled: descriptor.default_enabled,
                    experimental: descriptor.experimental,
                    dependencies: descriptor
                        .dependencies
                        .iter()
                        .map(|dependency| CapabilityPackDependencyObject {
                            pack_id: dependency.capability_id.to_string(),
                            min_version: dependency.min_version.to_string(),
                        })
                        .collect(),
                    inference_slots: descriptor
                        .inference_slots
                        .iter()
                        .map(|slot| CapabilityPackInferenceSlotObject {
                            name: slot.name.to_string(),
                            task: slot.task.to_string(),
                        })
                        .collect(),
                }
            })
            .collect()
    })
}

pub(super) fn ui_spec_for_pack(pack_id: &str) -> Option<PackUiSpec> {
    PACK_UI_SPECS
        .iter()
        .find(|spec| spec.pack_id == pack_id)
        .copied()
}
