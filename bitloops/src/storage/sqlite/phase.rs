use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SqliteWritePhaseMetrics {
    pub(crate) phase_name: Option<&'static str>,
    pub(crate) transaction_count: usize,
    pub(crate) max_wait_ms: u64,
    pub(crate) max_hold_ms: u64,
}

impl SqliteWritePhaseMetrics {
    pub(crate) fn record_timing(
        &mut self,
        phase_name: Option<&'static str>,
        waited: Duration,
        held: Duration,
    ) {
        if self.phase_name.is_none() {
            self.phase_name = phase_name;
        }
        self.transaction_count = self.transaction_count.saturating_add(1);
        self.max_wait_ms = self.max_wait_ms.max(waited.as_millis() as u64);
        self.max_hold_ms = self.max_hold_ms.max(held.as_millis() as u64);
    }

    pub(crate) fn merge(&mut self, other: Self) {
        if self.phase_name.is_none() {
            self.phase_name = other.phase_name;
        }
        self.transaction_count = self
            .transaction_count
            .saturating_add(other.transaction_count);
        self.max_wait_ms = self.max_wait_ms.max(other.max_wait_ms);
        self.max_hold_ms = self.max_hold_ms.max(other.max_hold_ms);
    }
}
