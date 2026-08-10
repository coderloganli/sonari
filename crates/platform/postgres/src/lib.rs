use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresConfig {
    pub dsn: String,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            dsn: "postgres://sonari:sonari@localhost:5432/sonari".to_owned(),
        }
    }
}

pub mod migrate;
pub use migrate::{connect, run_migrations};
