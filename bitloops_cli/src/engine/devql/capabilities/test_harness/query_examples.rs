use crate::engine::devql::capability_host::QueryExample;

pub static TEST_HARNESS_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: "test_harness",
        name: "Verification summary scaffold",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> test_harness_tests_summary()",
        description: "Dependency-gated scaffold stage for tests().summary() until dot-chained stage syntax is available",
    },
    QueryExample {
        capability_id: "test_harness",
        name: "Tests listing",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> tests()",
        description: "Lists covering tests for selected artefacts through the Test Harness capability pack",
    },
    QueryExample {
        capability_id: "test_harness",
        name: "Coverage mapping",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> coverage()",
        description: "Maps line and branch coverage to selected artefacts through the Test Harness capability pack",
    },
];
