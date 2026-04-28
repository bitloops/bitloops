mod delta;
mod full;
mod persistence;

#[cfg(test)]
mod tests;

use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult, ReconcileMode,
};

use super::types::{TEST_HARNESS_CAPABILITY_ID, TEST_HARNESS_CURRENT_STATE_CONSUMER_ID};

#[cfg(test)]
use self::persistence::ensure_unique_test_artefact_ids;

pub struct TestHarnessCurrentStateConsumer;

impl CurrentStateConsumer for TestHarnessCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        TEST_HARNESS_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        TEST_HARNESS_CURRENT_STATE_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            match request.reconcile_mode {
                ReconcileMode::MergedDelta => delta::reconcile_delta(request, context).await?,
                ReconcileMode::FullReconcile => full::reconcile_full(request, context).await?,
            }
            Ok(CurrentStateConsumerResult::applied(
                request.to_generation_seq_inclusive,
            ))
        })
    }
}
