//! Host-owned canonical agent contract types.
//!
//! Adapter-specific structs under `engine/agent/<adapter>/...` are allowed to
//! keep target quirks. These types are the small, stable surface that host code
//! should use when it needs to reason about agents without binding to a
//! particular remote runtime.

mod compatibility;
mod identity;
mod invocation;
mod lifecycle;
mod progress;
mod result;
mod stream;

pub use compatibility::{
    CanonicalContractCompatibility, CanonicalContractVersion, HostCapabilityFlags,
};
pub use identity::{CanonicalAgentIdentity, CanonicalSessionDescriptor};
pub use invocation::{
    CanonicalCorrelationMetadata, CanonicalInvocationFailure, CanonicalInvocationFailureKind,
    CanonicalInvocationRequest, CanonicalInvocationResponse,
};
pub use lifecycle::{CanonicalLifecycleEvent, CanonicalLifecycleEventKind};
pub use progress::CanonicalProgressUpdate;
pub use result::{
    CanonicalResultFragment, CanonicalResultState, CanonicalResumableSession,
    CanonicalResumableSessionState,
};
pub use stream::{CanonicalStreamEvent, CanonicalStreamEventKind};

#[cfg(test)]
mod tests;
