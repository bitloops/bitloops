#[path = "capability_events/coordinator.rs"]
mod coordinator;
#[path = "capability_events/plan.rs"]
mod plan;
#[path = "capability_events/queue.rs"]
mod queue;
#[cfg(test)]
#[path = "capability_events/tests.rs"]
mod tests;

pub(crate) use self::coordinator::SyncGenerationInput;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use self::coordinator::test_shared_instance_at;
pub use self::coordinator::{CapabilityEventCoordinator, CapabilityEventEnqueueResult};
