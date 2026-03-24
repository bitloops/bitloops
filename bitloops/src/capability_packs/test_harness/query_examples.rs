use crate::host::capability_host::QueryExample;

pub static TEST_HARNESS_QUERY_EXAMPLES: &[QueryExample] = &[
    QueryExample {
        capability_id: "test_harness",
        name: "Commit-level test harness snapshot",
        query: "repo(\"my-repo\") -> asOf(ref:\"main\") -> artefacts(name:\"findById\") -> test_harness_tests_summary()",
        description: "Per-commit row counts and coverage presence for the test harness store; requires asOf(ref:...) or asOf(commit:...) so DevQL resolves resolved_commit_sha",
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
