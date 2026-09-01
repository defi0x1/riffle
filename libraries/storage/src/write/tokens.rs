use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgExecutor;

#[derive(Clone, Debug)]
pub struct NewToken {
    pub mint: String,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub decimals: i16,
    pub mint_authority: Option<String>,
    pub freeze_authority: Option<String>,
    pub token_program: String,
    pub extensions: Option<serde_json::Value>,
    pub supply: Option<Decimal>,
    pub holder_count: Option<i32>,
    pub top10_share: Option<f64>,
    pub top1_share: Option<f64>,
    pub is_verified: Option<bool>,
    pub rugcheck_score: Option<i32>,
    pub rugcheck_flags: Option<serde_json::Value>,
    pub rugcheck_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

pub async fn upsert_token<'e, E: PgExecutor<'e>>(executor: E, row: &NewToken) -> eyre::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO tokens (
            mint, symbol, name, decimals, mint_authority, freeze_authority, token_program,
            extensions, supply, holder_count, top10_share, top1_share, is_verified,
            rugcheck_score, rugcheck_flags, rugcheck_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        ON CONFLICT (mint) DO UPDATE SET
            symbol           = EXCLUDED.symbol,
            name             = EXCLUDED.name,
            decimals         = EXCLUDED.decimals,
            mint_authority   = EXCLUDED.mint_authority,
            freeze_authority = EXCLUDED.freeze_authority,
            token_program    = EXCLUDED.token_program,
            extensions       = EXCLUDED.extensions,
            supply           = EXCLUDED.supply,
            holder_count     = EXCLUDED.holder_count,
            top10_share      = EXCLUDED.top10_share,
            top1_share       = EXCLUDED.top1_share,
            is_verified      = EXCLUDED.is_verified,
            rugcheck_score   = EXCLUDED.rugcheck_score,
            rugcheck_flags   = EXCLUDED.rugcheck_flags,
            rugcheck_at      = EXCLUDED.rugcheck_at,
            updated_at       = EXCLUDED.updated_at
        "#,
        row.mint,
        row.symbol,
        row.name,
        row.decimals,
        row.mint_authority,
        row.freeze_authority,
        row.token_program,
        row.extensions,
        row.supply,
        row.holder_count,
        row.top10_share,
        row.top1_share,
        row.is_verified,
        row.rugcheck_score,
        row.rugcheck_flags,
        row.rugcheck_at,
        row.updated_at,
    )
    .execute(executor)
    .await
    .wrap_err_with(|| format!("Upserting token {}", row.mint))?;

    Ok(())
}

#[cfg(all(test, feature = "db-tests"))]
mod tests {
    use super::*;
    use crate::test_support::test_pool;

    #[tokio::test]
    async fn test_upsert_token_is_idempotent() {
        let pool = test_pool().await;
        let now = Utc::now();
        let row = NewToken {
            mint: "token_upsert_idempotent".to_string(),
            symbol: Some("TEST".to_string()),
            name: Some("Test Token".to_string()),
            decimals: 6,
            mint_authority: None,
            freeze_authority: None,
            token_program: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            extensions: None,
            supply: Some(Decimal::new(1_000_000, 0)),
            holder_count: Some(42),
            top10_share: Some(0.2),
            top1_share: Some(0.05),
            is_verified: Some(true),
            rugcheck_score: None,
            rugcheck_flags: None,
            rugcheck_at: None,
            updated_at: now,
        };

        upsert_token(&pool, &row).await.unwrap();
        upsert_token(&pool, &row).await.unwrap();

        let count = sqlx::query_scalar!("SELECT count(*) FROM tokens WHERE mint = $1", row.mint)
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(count, Some(1));
    }
}
