use crate::engine::devql::capability_host::QueryExample;

pub static TEST_HARNESS_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: "test_harness",
        name: "Verification summary for artefacts",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> tests().summary()",
        description: "Return verification-level summary signals for artefacts",
    },
    QueryExample {
        capability_id: "test_harness",
        name: "List covering tests",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> tests()",
        description: "List test inventory and scoring metadata for artefacts",
    },
    QueryExample {
        capability_id: "test_harness",
        name: "Inspect artefact coverage",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> coverage()",
        description: "Return branch-level and line-level coverage mapped to artefacts",
    },
];
