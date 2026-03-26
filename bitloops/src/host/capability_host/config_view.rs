use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityConfigView {
    capability_id: String,
    root: Value,
}

impl CapabilityConfigView {
    pub fn new(capability_id: impl Into<String>, root: Value) -> Self {
        Self {
            capability_id: capability_id.into(),
            root,
        }
    }

    pub fn empty(capability_id: impl Into<String>) -> Self {
        Self::new(capability_id, Value::Object(Map::new()))
    }

    pub fn capability_id(&self) -> &str {
        &self.capability_id
    }

    pub fn root(&self) -> &Value {
        &self.root
    }

    pub fn scoped(&self) -> Option<&Value> {
        self.root.get(&self.capability_id)
    }

    pub fn as_object(&self) -> Option<&Map<String, Value>> {
        self.root.as_object()
    }
}
