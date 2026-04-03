# Running Tests

| Goal                           | Command                                                              |
| ------------------------------ | -------------------------------------------------------------------- |
| Fast unit tests (no E2E)       | `cargo test`                                                         |
| Run only the Gherkin suite     | `cargo test --test testlens_gherkin -- --ignored`                    |
| Run all E2E / acceptance tests | `cargo test -- --ignored`                                            |
| Run everything including E2E   | `cargo test -- --include-ignored`                                    |
| QAT smoke suite                | `cargo test --test qat_acceptance qat_smoke -- --ignored`            |
| QAT DevQL suite                | `cargo test --test qat_acceptance qat_devql -- --ignored`            |
| QAT Claude Code suite          | `cargo test --test qat_acceptance qat_claude_code -- --ignored`      |
| QAT all suites                 | `cargo test --test qat_acceptance -- --ignored`                      |

## Testing a separate binary (CI)

To test a production binary built separately:

```bash
cargo build --release
BITLOOPS_QAT_BINARY=target/release/bitloops \
  cargo test --test qat_acceptance -- --ignored
```

The `BITLOOPS_QAT_BINARY` env var tells the test harness which binary to exercise.
When unset, it falls back to the binary built alongside the test (`CARGO_BIN_EXE_bitloops`).
