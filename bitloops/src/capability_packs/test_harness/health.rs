mod config;
mod dependencies;
mod storage;

use crate::host::capability_host::CapabilityHealthCheck;

pub use config::check_test_harness_config;
pub use dependencies::check_test_harness_dependencies;
pub use storage::check_test_harness_storage;

pub static TEST_HARNESS_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "test_harness.config",
        run: check_test_harness_config,
    },
    CapabilityHealthCheck {
        name: "test_harness.storage",
        run: check_test_harness_storage,
    },
    CapabilityHealthCheck {
        name: "test_harness.dependencies",
        run: check_test_harness_dependencies,
    },
];
