use eyre::WrapErr;
use sqlx::PgPool;

// Expired mutes fall out by predicate (`until > now()`) rather than needing a sweeper job to
// delete rows -- a stale row is harmless disk until the next mute or query for that chat
// touches it.
pub async fn muted_pool_addresses(pool: &PgPool, chat_id: i64) -> eyre::Result<Vec<String>> {
    let rows = sqlx::query!(
        r#"
        SELECT pool_address
        FROM muted_pools
        WHERE chat_id = $1 AND until > now()
        "#,
        chat_id,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying muted pools for chat {chat_id}"))?;

    Ok(rows.into_iter().map(|r| r.pool_address).collect())
}
