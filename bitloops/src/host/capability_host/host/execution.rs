use anyhow::{Result, bail};
use serde_json::Value;

use crate::host::devql::RelationalStorage;

use super::super::health::CapabilityHealthResult;
use super::super::policy::with_timeout;
use super::super::registrar::{IngestRequest, IngestResult, StageRequest, StageResponse};
use super::{DevqlCapabilityHost, RegisteredIngester, RegisteredStage, lifecycle};

impl DevqlCapabilityHost {
    pub async fn invoke_ingester(
        &self,
        capability_id: &str,
        ingester_name: &str,
        payload: Value,
    ) -> Result<IngestResult> {
        self.invoke_ingester_with_relational(capability_id, ingester_name, payload, None)
            .await
    }

    pub async fn invoke_ingester_with_relational(
        &self,
        capability_id: &str,
        ingester_name: &str,
        payload: Value,
        devql_relational: Option<&RelationalStorage>,
    ) -> Result<IngestResult> {
        self.ensure_migrations_applied()?;

        let key = (capability_id.to_string(), ingester_name.to_string());
        let handler = self.ingesters.get(&key).cloned();
        let Some(handler) = handler else {
            bail!(
                "[capability_pack:{capability_id}] [ingester:{ingester_name}] not registered on DevqlCapabilityHost"
            );
        };

        let request = IngestRequest::new(payload);
        let declared_mailboxes = self.declared_mailboxes_for_capability(capability_id);
        let mut runtime = self.runtime.runtime_with_relational(
            devql_relational,
            Some(capability_id),
            Some(ingester_name),
            declared_mailboxes.as_slice(),
        );
        let limit = self.invocation_policy.ingester_timeout;
        match handler {
            RegisteredIngester::Core(h) => {
                with_timeout(
                    "capability ingester",
                    limit,
                    h.ingest(request, &mut runtime),
                )
                .await
            }
            RegisteredIngester::Knowledge(h) => {
                with_timeout(
                    "capability ingester",
                    limit,
                    h.ingest(request, &mut runtime),
                )
                .await
            }
        }
    }

    pub async fn invoke_stage(
        &self,
        capability_id: &str,
        stage_name: &str,
        payload: Value,
    ) -> Result<StageResponse> {
        self.ensure_migrations_applied()?;

        let key = (capability_id.to_string(), stage_name.to_string());
        let handler = self.stages.get(&key).cloned();
        let Some(handler) = handler else {
            bail!(
                "[capability_pack:{capability_id}] [stage:{stage_name}] not registered on DevqlCapabilityHost"
            );
        };

        let request = StageRequest::new(payload);
        let declared_mailboxes = self.declared_mailboxes_for_capability(capability_id);
        let mut runtime = self
            .runtime
            .runtime_for_capability(capability_id, declared_mailboxes.as_slice());
        let limit = self.invocation_policy.stage_timeout;
        match handler {
            RegisteredStage::Core(h) => {
                with_timeout("capability stage", limit, h.execute(request, &mut runtime)).await
            }
            RegisteredStage::Knowledge(h) => {
                with_timeout("capability stage", limit, h.execute(request, &mut runtime)).await
            }
        }
    }

    pub fn run_health_checks(&self, capability_id: &str) -> Vec<(String, CapabilityHealthResult)> {
        let checks = self
            .health_checks
            .get(capability_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let declared_mailboxes = self.declared_mailboxes_for_capability(capability_id);
        let runtime = self
            .runtime
            .runtime_for_capability(capability_id, declared_mailboxes.as_slice());
        lifecycle::run_health_checks(capability_id, checks, &runtime)
    }

    pub fn has_stage(&self, capability_id: &str, stage_name: &str) -> bool {
        self.stages
            .contains_key(&(capability_id.to_string(), stage_name.to_string()))
    }

    fn ensure_migrations_applied(&self) -> Result<()> {
        if self
            .migrations_applied
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }
        let _guard = self
            .migration_lock
            .lock()
            .expect("capability host migration lock poisoned");
        if self
            .migrations_applied
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }
        let mut runtime = self.runtime.runtime();
        lifecycle::run_migrations(&self.migrations, &mut runtime)?;
        self.migrations_applied
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    /// Run registered pack migrations synchronously (e.g. during `devql init` before async ingest).
    pub fn ensure_migrations_applied_sync(&self) -> Result<()> {
        self.ensure_migrations_applied()
    }
}
