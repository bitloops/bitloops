use std::sync::Arc;

use anyhow::{Result, bail};

use super::super::events::HostEventHandler;
use super::super::registrar::{
    CapabilityMailboxRegistration, KnowledgeIngesterRegistration, KnowledgeStageRegistration,
    QueryExample, SchemaModule, StageRegistration,
};
use super::{CapabilityRegistrar, DevqlCapabilityHost, RegisteredIngester, RegisteredStage};
use crate::host::capability_host::CurrentStateConsumerRegistration;

impl CapabilityRegistrar for DevqlCapabilityHost {
    fn register_stage(&mut self, stage: StageRegistration) -> Result<()> {
        let key = (
            stage.capability_id.to_string(),
            stage.stage_name.to_string(),
        );
        if self.stages.contains_key(&key) {
            bail!(
                "[capability_pack:{}] [stage:{}] duplicate registration",
                stage.capability_id,
                stage.stage_name
            );
        }
        self.stages
            .insert(key, RegisteredStage::Core(stage.handler));
        Ok(())
    }

    fn register_ingester(
        &mut self,
        ingester: super::super::registrar::IngesterRegistration,
    ) -> Result<()> {
        let key = (
            ingester.capability_id.to_string(),
            ingester.ingester_name.to_string(),
        );
        if self.ingesters.contains_key(&key) {
            bail!(
                "[capability_pack:{}] [ingester:{}] duplicate registration",
                ingester.capability_id,
                ingester.ingester_name
            );
        }
        self.ingesters
            .insert(key, RegisteredIngester::Core(ingester.handler));
        Ok(())
    }

    fn register_mailbox(&mut self, registration: CapabilityMailboxRegistration) -> Result<()> {
        let duplicate = self.mailboxes.iter().any(|existing| {
            existing.capability_id == registration.capability_id
                && existing.mailbox_name == registration.mailbox_name
        });
        if duplicate {
            bail!(
                "[capability_pack:{}] [mailbox:{}] duplicate registration",
                registration.capability_id,
                registration.mailbox_name
            );
        }
        self.mailboxes.push(registration);
        Ok(())
    }

    fn register_knowledge_stage(&mut self, stage: KnowledgeStageRegistration) -> Result<()> {
        let key = (
            stage.capability_id.to_string(),
            stage.stage_name.to_string(),
        );
        if self.stages.contains_key(&key) {
            bail!(
                "[capability_pack:{}] [stage:{}] duplicate registration",
                stage.capability_id,
                stage.stage_name
            );
        }
        self.stages
            .insert(key, RegisteredStage::Knowledge(stage.handler));
        Ok(())
    }

    fn register_knowledge_ingester(
        &mut self,
        ingester: KnowledgeIngesterRegistration,
    ) -> Result<()> {
        let key = (
            ingester.capability_id.to_string(),
            ingester.ingester_name.to_string(),
        );
        if self.ingesters.contains_key(&key) {
            bail!(
                "[capability_pack:{}] [ingester:{}] duplicate registration",
                ingester.capability_id,
                ingester.ingester_name
            );
        }
        self.ingesters
            .insert(key, RegisteredIngester::Knowledge(ingester.handler));
        Ok(())
    }

    fn register_schema_module(&mut self, module: SchemaModule) -> Result<()> {
        self.schema_modules.push(module);
        Ok(())
    }

    fn register_query_examples(&mut self, examples: &'static [QueryExample]) -> Result<()> {
        self.query_examples.push(examples);
        Ok(())
    }

    fn register_event_handler(&mut self, handler: Arc<dyn HostEventHandler>) -> Result<()> {
        self.event_handlers.push(handler);
        Ok(())
    }

    fn register_current_state_consumer(
        &mut self,
        registration: CurrentStateConsumerRegistration,
    ) -> Result<()> {
        let duplicate = self.current_state_consumers.iter().any(|existing| {
            existing.capability_id == registration.capability_id
                && existing.consumer_id == registration.consumer_id
        });
        if duplicate {
            bail!(
                "[capability_pack:{}] [current_state_consumer:{}] duplicate registration",
                registration.capability_id,
                registration.consumer_id
            );
        }
        self.current_state_consumers.push(registration);
        Ok(())
    }
}
