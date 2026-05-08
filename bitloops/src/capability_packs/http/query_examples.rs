use crate::host::capability_host::QueryExample;

use super::types::HTTP_CAPABILITY_ID;

pub static HTTP_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: HTTP_CAPABILITY_ID,
        name: "http.search_terms",
        query: "repo(\"my-repo\")->httpSearch(terms:\"HEAD,Content-Length,runtime\")->limit(10)",
        description: "Find HTTP causal bundles and role-matched facts from failing behaviour terms.",
    },
    QueryExample {
        capability_id: HTTP_CAPABILITY_ID,
        name: "http.selection_overview",
        query: "selectArtefacts(search:\"HEAD Content-Length runtime\")->overview()",
        description: "Get the compact artefact overview with the HTTP risk entry and expand hint.",
    },
    QueryExample {
        capability_id: HTTP_CAPABILITY_ID,
        name: "http.selected_context",
        query: "selectArtefacts(search:\"HEAD Content-Length runtime\")->httpContext()",
        description: "Expand HTTP context for selected artefacts.",
    },
    QueryExample {
        capability_id: HTTP_CAPABILITY_ID,
        name: "http.header_producers",
        query: "repo(\"my-repo\")->httpHeaderProducers(header:\"content-length\")->limit(20)",
        description: "Find structured producers and derivation rules for an HTTP header.",
    },
    QueryExample {
        capability_id: HTTP_CAPABILITY_ID,
        name: "http.lifecycle_boundaries",
        query: "repo(\"my-repo\")->httpLifecycleBoundaries(terms:\"framework,runtime,serialisation\")->limit(20)",
        description: "Inspect HTTP framework-to-runtime lifecycle boundaries.",
    },
    QueryExample {
        capability_id: HTTP_CAPABILITY_ID,
        name: "http.lossy_transforms",
        query: "repo(\"my-repo\")->artefacts(symbol_fqn:\"...\")->httpLossyTransforms()->limit(20)",
        description: "Find lossy HTTP transforms connected to a selected source artefact.",
    },
    QueryExample {
        capability_id: HTTP_CAPABILITY_ID,
        name: "http.patch_impact",
        query: "repo(\"my-repo\")->httpPatchImpact(patch_fingerprint:\"...\")",
        description: "Inspect invalidated assumptions and propagation obligations associated with a patch fingerprint.",
    },
];

#[cfg(test)]
mod tests {
    use super::HTTP_QUERY_EXAMPLES;

    #[test]
    fn http_query_examples_use_supported_stage_names() {
        for example in HTTP_QUERY_EXAMPLES {
            assert!(
                example.name.starts_with("http."),
                "example `{}` should be namespaced to HTTP",
                example.name
            );
        }
    }
}
