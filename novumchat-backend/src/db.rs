//! SeaORM entities and the SQLite connection.
//!
//! Tables are created and extended automatically from the entities (SeaORM's
//! entity-first schema sync), so adding a column is a one-line change.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};

pub mod chat {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "chats")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        /// Possibly encrypted.
        #[sea_orm(column_type = "Text")]
        pub title: String,
        /// Possibly encrypted.
        #[sea_orm(column_type = "Text", nullable)]
        pub system_prompt: Option<String>,
        /// JSON `ChatSettings`, possibly encrypted.
        #[sea_orm(column_type = "Text")]
        pub settings: String,
        pub pinned: bool,
        pub archived: bool,
        #[sea_orm(nullable)]
        pub current_leaf: Option<String>,
        pub created_at: i64,
        #[sea_orm(indexed)]
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod message {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "messages")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(indexed)]
        pub chat_id: String,
        #[sea_orm(nullable)]
        pub parent_id: Option<String>,
        pub role: String,
        /// JSON `Vec<Part>`, possibly encrypted.
        #[sea_orm(column_type = "Text")]
        pub content: String,
        /// JSON `MessageMeta` (model, usage, timings, error), possibly encrypted.
        #[sea_orm(column_type = "Text")]
        pub meta: String,
        #[sea_orm(nullable)]
        pub group_id: Option<String>,
        pub synthesis: bool,
        pub status: String,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod attachment {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "attachments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        #[sea_orm(indexed)]
        pub chat_id: String,
        #[sea_orm(indexed, nullable)]
        pub message_id: Option<String>,
        /// Possibly encrypted.
        #[sea_orm(column_type = "Text")]
        pub name: String,
        pub mime: String,
        pub kind: String,
        pub size: i64,
        /// File bytes, possibly encrypted.
        #[sea_orm(column_type = "Blob")]
        pub data: Vec<u8>,
        /// Extracted text for documents, possibly encrypted.
        #[sea_orm(column_type = "Text", nullable)]
        pub text: Option<String>,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod kv {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "kv")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
        #[sea_orm(column_type = "Text")]
        pub value: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Opens (creating if needed) the database and syncs the schema.
pub async fn connect(path: &Path) -> Result<DatabaseConnection> {
    let file = path.to_path_buf();
    let mut opts = ConnectOptions::new("sqlite://novumchat.db?mode=rwc");
    opts.max_connections(8)
        .min_connections(1)
        .sqlx_logging(false)
        .acquire_timeout(Duration::from_secs(30))
        .map_sqlx_sqlite_opts(move |o| {
            o.filename(&file)
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(Duration::from_secs(15))
                .foreign_keys(true)
                // Overwrite deleted content so plaintext doesn't linger in free pages.
                .pragma("secure_delete", "ON")
        });
    let db = Database::connect(opts).await.with_context(|| format!("opening database {}", path.display()))?;
    sync_schema(&db).await?;
    Ok(db)
}

pub async fn sync_schema(db: &DatabaseConnection) -> Result<()> {
    db.get_schema_builder()
        .register(chat::Entity)
        .register(message::Entity)
        .register(attachment::Entity)
        .register(kv::Entity)
        .sync(db)
        .await
        .context("syncing database schema")?;
    Ok(())
}

/// Reclaims space and flushes the WAL so old plaintext pages disappear.
pub async fn compact(db: &DatabaseConnection) -> Result<()> {
    db.execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE);").await?;
    db.execute_unprepared("VACUUM;").await?;
    db.execute_unprepared("PRAGMA wal_checkpoint(TRUNCATE);").await?;
    Ok(())
}
