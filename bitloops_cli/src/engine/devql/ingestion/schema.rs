// Database schema initialisation split by backend/domain.

include!("schema/events_backends.rs");
include!("schema/relational_initialisation.rs");
include!("schema/relational_sqlite_schema.rs");
include!("schema/relational_postgres_schema.rs");
include!("schema/checkpoint_postgres_schema.rs");
include!("schema/relational_postgres_migrations.rs");
include!("schema/checkpoint_sqlite_schema.rs");
