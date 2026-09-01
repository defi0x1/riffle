use eyre::WrapErr;
use sqlx::PgPool;

// Baked in at compile time from the workspace-root migrations directory, so `make migrate`
// (sqlx-cli against the same directory) and this in-process runner never drift apart.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub async fn run_migrations(pool: &PgPool) -> eyre::Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .wrap_err_with(|| "Running database migrations")
}
