// Database schema initialisation split by backend/domain.
use super::*;

#[path = "schema/checkpoint_postgres_schema.rs"]
mod checkpoint_postgres_schema;
#[path = "schema/checkpoint_sqlite_schema.rs"]
mod checkpoint_sqlite_schema;
#[path = "schema/events_backends.rs"]
mod events_backends;
#[path = "schema/knowledge_schema.rs"]
mod knowledge_schema;
#[path = "schema/relational_initialisation.rs"]
mod relational_initialisation;
#[path = "schema/relational_postgres_migrations.rs"]
mod relational_postgres_migrations;
#[path = "schema/relational_postgres_schema.rs"]
mod relational_postgres_schema;
#[path = "schema/relational_sqlite_schema.rs"]
mod relational_sqlite_schema;

pub(crate) use self::checkpoint_postgres_schema::*;
pub(crate) use self::checkpoint_sqlite_schema::*;
pub(super) use self::events_backends::*;
pub(crate) use self::knowledge_schema::*;
pub(super) use self::relational_initialisation::*;
pub(crate) use self::relational_postgres_migrations::*;
pub(super) use self::relational_postgres_schema::*;
pub(crate) use self::relational_sqlite_schema::*;
