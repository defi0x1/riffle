use clap::Parser;
use eyre::WrapErr;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

#[derive(Parser, Debug, Clone)]
#[group(id = "postgres")]
pub struct PostgresConfig {
    /// Postgres connection string, e.g. postgres://user:pass@host/db
    #[arg(long, env)]
    pub database_url: String,

    /// Maximum size of the connection pool.
    #[arg(long, env, default_value_t = 10)]
    pub max_connections: u32,
}

impl PostgresConfig {
    pub async fn connect(&self) -> eyre::Result<PgPool> {
        PgPoolOptions::new()
            .max_connections(self.max_connections)
            .connect(&self.database_url)
            .await
            .wrap_err_with(|| "Connecting to postgres")
    }
}

// database_url may carry embedded credentials, so any logging of this config must go
// through this impl rather than the derived Debug.
impl std::fmt::Display for PostgresConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PostgresConfig {{ database_url: <redacted>, max_connections: {} }}",
            self.max_connections
        )
    }
}
