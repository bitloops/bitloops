Feature: Structural Test Mapping

  Background:
    Given a Rust production file at "src/user/service.rs":
      """
      pub fn create_user(name: &str) -> String {
          name.to_string()
      }

      pub fn delete_user(id: u64) -> bool {
          true
      }
      """

  # ---- Happy paths ----

  @S1
  Scenario: S1 Discover Rust test artefacts from source
    Given a Rust test file at "src/user/service_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_create_user() {
              create_user("Alice");
          }
      }
      """
    When test discovery runs
    Then test suites include:
      | name  | scenario_count |
      | tests | 1              |
    And test scenarios include:
      | name             | discovery_source |
      | test_create_user | source           |

  @S2
  Scenario: S2 Direct call-site linkage creates test-to-production edge
    Given a Rust test file at "src/user/service_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_create_user() {
              create_user("Alice");
          }
      }
      """
    When linkage resolution runs
    Then direct links include:
      | production_name | confidence | linkage_status |
      | create_user     | 0.6        | resolved       |

  @S3
  Scenario: S3 DevQL tests() query returns covering tests with confidence
    Given a Rust test file at "src/user/service_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_create_user() {
              create_user("Alice");
          }
      }
      """
    When linkage resolution runs and tests() query executes for "create_user"
    Then the response has covering_tests with:
      | test_name        | confidence |
      | test_create_user | 0.6        |

  @S4
  Scenario: S4 Alias-based call resolves to correct production artefact
    Given a Rust test file at "src/user/alias_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          use crate::user::service::create_user as cu;
          #[test]
          fn test_alias() {
              cu("Bob");
          }
      }
      """
    When test discovery runs
    Then test suites include:
      | name  | scenario_count |
      | tests | 1              |
    And test scenarios include:
      | name       | discovery_source |
      | test_alias | source           |

  @S5
  Scenario: S5 Nested mod tests with inner modules creates correct hierarchy
    Given a Rust test file at "src/user/nested_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          mod inner {
              use super::super::*;
              #[test]
              fn test_inner_create() {
                  create_user("Charlie");
              }
          }
          #[test]
          fn test_outer() {
              create_user("Dave");
          }
      }
      """
    When test discovery runs
    Then test suites include:
      | name         | scenario_count |
      | tests        | 1              |
      | tests::inner | 1              |

  # ---- Alternative paths ----

  @S6 @deferred
  Scenario: S6 High fan-in production artefact shows cross-cutting flag
    Given a Rust test file at "src/user/fanin_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_fanin() {
              create_user("Eve");
          }
      }
      """
    When test discovery runs
    Then test scenarios include:
      | name        | discovery_source |
      | test_fanin  | source           |

  @S7
  Scenario: S7 Helper-wrapped call does not create deep link
    Given a Rust test file at "src/user/helper_tests.rs":
      """
      fn helper(name: &str) -> String {
          create_user(name)
      }

      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_via_helper() {
              helper("Frank");
          }
      }
      """
    When test discovery runs
    And linkage resolution runs
    Then no links to "create_user" from "test_via_helper"

  # ---- Edge cases ----

  @E1
  Scenario: E1 Trait object test creates zero false direct edges
    Given a Rust test file at "src/user/trait_tests.rs":
      """
      trait Saveable {
          fn save(&self);
      }

      #[cfg(test)]
      mod tests {
          use super::*;
          #[test]
          fn test_trait() {
              let x: &dyn Saveable = todo!();
              x.save();
          }
      }
      """
    When test discovery runs
    And linkage resolution runs
    Then no links are created

  @E2
  Scenario: E2 Method on dyn Trait creates zero speculative edges
    Given a Rust test file at "src/user/dyn_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          #[test]
          fn test_dyn_dispatch() {
              let x: Box<dyn std::fmt::Debug> = Box::new(42);
              format!("{:?}", x);
          }
      }
      """
    When test discovery runs
    And linkage resolution runs
    Then no links are created

  @E3
  Scenario: E3 Proptest macro tests produce partial extraction
    Given a Rust test file at "src/user/proptest_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          use proptest::prelude::*;
          proptest! {
              #[test]
              fn test_proptest_add(a in 0i32..100, b in 0i32..100) {
                  assert!(a + b >= 0);
              }
          }
      }
      """
    When test discovery runs
    Then test suites include:
      | name  | scenario_count |
      | tests | 1              |

  @E4
  Scenario: E4 Test exercising CLI boundary only creates no deep links
    Given a Rust test file at "src/user/cli_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          #[test]
          fn test_cli_boundary() {
              let output = std::process::Command::new("echo")
                  .arg("hello")
                  .output()
                  .unwrap();
              assert!(output.status.success());
          }
      }
      """
    When test discovery runs
    And linkage resolution runs
    Then no links are created

  @E5
  Scenario: E5 Disambiguate same-named methods via import scope
    Given a Rust production file at "src/order/service.rs":
      """
      pub fn save() -> bool { true }
      """
    And a Rust test file at "src/order/save_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          use crate::order::service::save;
          #[test]
          fn test_order_save() {
              save();
          }
      }
      """
    When test discovery runs
    Then test scenarios include:
      | name            | discovery_source |
      | test_order_save | source           |

  # ---- Error cases ----

  @ERR1
  Scenario: ERR1 Syntax error in one file does not block others
    Given a Rust test file at "src/user/broken_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          #[test]
          fn test_broken() {
      """
    When test discovery runs with diagnostics
    Then diagnostics include:
      | path                       | severity |
      | src/user/broken_tests.rs   | warning  |

  @ERR2
  Scenario: ERR2 Unsupported construct produces partial extraction
    Given a Rust test file at "src/user/unsupported_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          macro_rules! gen_tests {
              ($name:ident) => {
                  #[test]
                  fn $name() {}
              };
          }
          gen_tests!(test_generated);
      }
      """
    When test discovery runs
    Then test suites include:
      | name  | scenario_count |
      | tests | 1              |

  @ERR3
  Scenario: ERR3 Unknown symbol call creates no speculative edge
    Given a Rust test file at "src/user/unknown_tests.rs":
      """
      #[cfg(test)]
      mod tests {
          #[test]
          fn test_unknown_call() {
              nonexistent_function();
          }
      }
      """
    When test discovery runs
    And linkage resolution runs
    Then no links are created
