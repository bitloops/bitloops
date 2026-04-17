/// Host-owned progress snapshot for streaming work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalProgressUpdate {
    pub label: Option<String>,
    pub message: Option<String>,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub percentage: Option<u8>,
}

impl CanonicalProgressUpdate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_label(mut self, label: impl AsRef<str>) -> Self {
        let label = label.as_ref().trim();
        if !label.is_empty() {
            self.label = Some(label.to_string());
        }
        self
    }

    pub fn with_message(mut self, message: impl AsRef<str>) -> Self {
        let message = message.as_ref().trim();
        if !message.is_empty() {
            self.message = Some(message.to_string());
        }
        self
    }

    pub fn with_completed(mut self, completed: u64) -> Self {
        self.completed = Some(completed);
        self
    }

    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }

    pub fn with_percentage(mut self, percentage: u8) -> Self {
        self.percentage = Some(percentage.min(100));
        self
    }

    pub fn with_counts(mut self, completed: u64, total: u64) -> Self {
        self.completed = Some(completed);
        self.total = Some(total);
        self.percentage = completed
            .saturating_mul(100)
            .checked_div(total)
            .map(|percentage| percentage.min(100) as u8);
        self
    }

    pub fn is_complete(&self) -> bool {
        match (self.completed, self.total) {
            (Some(completed), Some(total)) if completed >= total => true,
            _ => self.percentage == Some(100),
        }
    }
}
