# Running Tests

| Goal                           | Command                                           |
| ------------------------------ | ------------------------------------------------- |
| Fast unit tests (no E2E)       | `cargo test`                                      |
| Run only the Gherkin suite     | `cargo test --test testlens_gherkin -- --ignored` |
| Run all E2E / acceptance tests | `cargo test -- --ignored`                         |
| Run everything including E2E   | `cargo test -- --include-ignored`                 |
