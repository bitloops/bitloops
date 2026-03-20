use crate::engine::devql::capability_host::QueryExample;

pub static TEST_HARNESS_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: "test_harness",
        name: "Scaffold verification summary",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> test_harness_tests_summary()",
        description: "Dependency-gated scaffold stage for verification summary until tests()/coverage() cutover is completed",
    },
    QueryExample {
        capability_id: "test_harness",
        name: "Scaffold tests listing",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> test_harness_tests()",
        description: "Dependency-gated scaffold stage for tests() capability-pack migration",
    },
    QueryExample {
        capability_id: "test_harness",
        name: "Scaffold coverage mapping",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> test_harness_coverage()",
        description: "Dependency-gated scaffold stage for coverage() capability-pack migration",
    },
];
