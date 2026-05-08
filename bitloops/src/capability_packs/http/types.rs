pub const HTTP_CAPABILITY_ID: &str = "http";
pub const HTTP_CONSUMER_ID: &str = "http.current_state";
pub const HTTP_OWNER: &str = "http";

pub const HTTP_PRIMITIVE_BEHAVIOUR_INVARIANT: &str = "BehaviourInvariant";
pub const HTTP_PRIMITIVE_HEADER_SEMANTIC: &str = "HeaderSemantic";
pub const HTTP_PRIMITIVE_DERIVED_VALUE_RULE: &str = "DerivedValueRule";
pub const HTTP_PRIMITIVE_LIFECYCLE_PHASE_RULE: &str = "LifecyclePhaseRule";
pub const HTTP_PRIMITIVE_CONTROL_FLOW_PATH: &str = "ControlFlowPath";
pub const HTTP_PRIMITIVE_DATA_TRANSFORM: &str = "DataTransform";
pub const HTTP_PRIMITIVE_LOSSY_TRANSFORM: &str = "LossyTransform";
pub const HTTP_PRIMITIVE_API_CAPABILITY: &str = "ApiCapability";
pub const HTTP_PRIMITIVE_TRAIT_IMPLEMENTATION: &str = "TraitImplementation";
pub const HTTP_PRIMITIVE_RUNTIME_BOUNDARY: &str = "RuntimeBoundary";
pub const HTTP_PRIMITIVE_CAUSAL_RISK: &str = "CausalRisk";
pub const HTTP_PRIMITIVE_INVALIDATED_ASSUMPTION: &str = "InvalidatedAssumption";
pub const HTTP_PRIMITIVE_PROPAGATION_OBLIGATION: &str = "PropagationObligation";

pub const HTTP_ROLE_HEAD_METHOD: &str = "http.request.method.head";
pub const HTTP_ROLE_GET_EQUIVALENT_HEADERS: &str = "http.response.headers.get_equivalent";
pub const HTTP_ROLE_CONTENT_LENGTH_HEADER: &str = "http.header.content_length";
pub const HTTP_ROLE_HEADER_DERIVATION: &str = "http.header.derivation";
pub const HTTP_ROLE_BODY_EXACT_SIZE_SIGNAL: &str = "http.body.exact_size_signal";
pub const HTTP_ROLE_BODY_REPLACEMENT: &str = "http.response.body_replacement";
pub const HTTP_ROLE_BODY_STRIPPING: &str = "http.response.body_stripping";
pub const HTTP_ROLE_WIRE_SERIALISATION_BOUNDARY: &str =
    "http.lifecycle.wire_serialisation_boundary";
pub const HTTP_ROLE_FRAMEWORK_RUNTIME_BOUNDARY: &str = "http.lifecycle.framework_runtime_boundary";

pub const HTTP_BUNDLE_CONTENT_LENGTH_LOSS_BEFORE_WIRE_SERIALISATION: &str =
    "http.bundle.content_length_loss.before_wire_serialisation";
pub const HTTP_RISK_CONTENT_LENGTH_LOSS: &str = "CONTENT_LENGTH_LOSS";

pub const HTTP_QUERY_INDEX_TABLE: &str = "http_query_index_current";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_taxonomy_uses_protocol_roles_without_ecosystem_symbols() {
        let taxonomy = [
            HTTP_ROLE_HEAD_METHOD,
            HTTP_ROLE_GET_EQUIVALENT_HEADERS,
            HTTP_ROLE_CONTENT_LENGTH_HEADER,
            HTTP_ROLE_HEADER_DERIVATION,
            HTTP_ROLE_BODY_EXACT_SIZE_SIGNAL,
            HTTP_ROLE_BODY_REPLACEMENT,
            HTTP_ROLE_BODY_STRIPPING,
            HTTP_ROLE_WIRE_SERIALISATION_BOUNDARY,
            HTTP_ROLE_FRAMEWORK_RUNTIME_BOUNDARY,
            HTTP_BUNDLE_CONTENT_LENGTH_LOSS_BEFORE_WIRE_SERIALISATION,
        ];

        for value in taxonomy {
            assert!(value.starts_with("http."), "`{value}` should be HTTP-owned");
            assert!(
                !value.contains("axum") && !value.contains("hyper") && !value.contains("rust"),
                "`{value}` should not store ecosystem-specific ownership"
            );
        }
    }
}
