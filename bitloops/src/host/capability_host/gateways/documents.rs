use anyhow::Result;

/// Host document (columnar) store port.
pub trait DocumentStoreGateway: Send + Sync {
    fn initialise_schema(&self) -> Result<()>;
}
