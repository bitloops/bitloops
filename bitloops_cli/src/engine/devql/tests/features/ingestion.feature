Feature: DevQL ingestion BDD scenarios

  @S1
  Scenario: S1 TS/JS artefact extraction succeeds
    Given a TypeScript source file at "src/sample.ts":
      """
      import defaultHelper, { helper } from "./helpers";
      export { helper };
      export { helper };
      export { helper as helperAlias };

      interface User {
        id: string;
      }

      type UserId = string;

      function normalizeId(id: UserId): UserId {
        return id;
      }

      class BaseService {}

      class Service extends BaseService {
        constructor(private readonly prefix: string) {}

        run(user: User): UserId {
          normalizeId(user.id);
          defaultHelper(user.id);
          missing();
          return helper(user.id);
        }
      }
      """
    When devql ingest extracts artefacts
    Then artefacts include:
      | language_kind           | canonical_kind | name        |
      | import_statement        | import         | import@2    |
      | interface_declaration   | interface      | User        |
      | type_alias_declaration  | type           | UserId      |
      | class_declaration       | -              | Service     |
      | constructor             | -              | constructor |
      | method_definition       | method         | run         |
      | function_declaration    | function       | normalizeId |

  @S2
  Scenario: S2 Rust artefact extraction succeeds
    Given a Rust source file at "src/lib.rs":
      """
      use crate::math::sum;

      trait Reader {}
      trait Writer {}

      trait Repository: Reader + Writer {
          fn load(&self);
      }

      struct PgRepository;

      impl Repository for PgRepository {
          fn load(&self) {
              helper();
              sum();
              println!("hi");
          }
      }

      pub fn helper() {}
      pub use self::helper;
      """
    When devql ingest extracts artefacts
    Then artefacts include:
      | language_kind    | canonical_kind | name         |
      | use_declaration  | import         | use@2        |
      | struct_item      | -              | PgRepository |
      | trait_item       | interface      | Repository   |
      | impl_item        | -              | impl@13      |
      | function_item    | function       | helper       |
      | function_item    | method         | load         |

  @S3
  Scenario: S3 Dependency edges are emitted
    Given a TypeScript source file at "src/sample.ts":
      """
      import defaultHelper, { helper } from "./helpers";
      export { helper };
      export { helper };
      export { helper as helperAlias };

      interface User {
        id: string;
      }

      type UserId = string;

      function normalizeId(id: UserId): UserId {
        return id;
      }

      class BaseService {}

      class Service extends BaseService {
        constructor(private readonly prefix: string) {}

        run(user: User): UserId {
          normalizeId(user.id);
          defaultHelper(user.id);
          missing();
          return helper(user.id);
        }
      }
      """
    And a Rust source file at "src/lib.rs":
      """
      use crate::math::sum;

      trait Reader {}
      trait Writer {}

      trait Repository: Reader + Writer {
          fn load(&self);
      }

      struct PgRepository;

      impl Repository for PgRepository {
          fn load(&self) {
              helper();
              sum();
          }
      }

      pub fn helper() {}
      pub use self::helper;
      """
    When devql ingest extracts artefacts
    And devql ingest extracts dependency edges
    Then edges include:
      | edge_kind  | from                      | to_target                | to_ref                 | metadata_key | metadata_value |
      | imports    | src/sample.ts             | -                        | ./helpers              | -            | -              |
      | calls      | src/sample.ts::Service::run | src/sample.ts::normalizeId | -                   | resolution   | local          |
      | references | src/sample.ts::Service::run | src/sample.ts::User    | -                      | ref_kind     | type           |
      | extends    | src/sample.ts::Service   | src/sample.ts::BaseService | -                    | -            | -              |
      | exports    | src/sample.ts            | -                        | ./helpers::helper      | export_name  | helperAlias    |
      | imports    | src/lib.rs               | -                        | crate::math::sum       | -            | -              |
      | implements | src/lib.rs::impl@13      | -                        | Repository             | -            | -              |
      | calls      | src/lib.rs::impl@13::load | src/lib.rs::helper      | -                      | resolution   | local          |
      | extends    | src/lib.rs::Repository   | src/lib.rs::Reader       | -                      | -            | -              |
      | exports    | src/lib.rs               | src/lib.rs::helper       | -                      | export_form  | pub_use        |

  @E1
  Scenario: E1 Macro invocation is dropped when unresolved
    Given a Rust source file at "src/lib.rs":
      """
      fn project() {
          println!("hi");
      }
      """
    When devql ingest extracts artefacts
    And devql ingest extracts dependency edges
    Then no edges are emitted

  @E2
  Scenario: E2 Unresolved symbol fallback
    Given a TypeScript source file at "src/sample.ts":
      """
      function caller() {
        mystery();
      }
      """
    When devql ingest extracts artefacts
    And devql ingest extracts dependency edges
    Then edges include:
      | edge_kind | from                 | to_target | to_ref                 | metadata_key | metadata_value |
      | calls     | src/sample.ts::caller | -       | src/sample.ts::mystery | resolution   | unresolved     |

  @E3
  Scenario: E3 Export and re-export dedup
    Given a TypeScript source file at "src/sample.ts":
      """
      import { helper } from "./helpers";
      export { helper };
      export { helper };
      export { helper as helperAlias };
      """
    When devql ingest extracts artefacts
    And devql ingest extracts dependency edges
    Then the export edge named "helper" appears 1 time(s)
    And edges include:
      | edge_kind | from          | to_target | to_ref            | metadata_key | metadata_value |
      | exports   | src/sample.ts | -         | ./helpers::helper | export_name  | helperAlias    |
