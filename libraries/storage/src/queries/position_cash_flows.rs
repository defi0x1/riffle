use chrono::{DateTime, Utc};
use eyre::WrapErr;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct CashFlowRow {
    pub transaction_intent_id: Uuid,
    pub kind: i16,
    pub ts: DateTime<Utc>,
    pub amount_x: Option<Decimal>,
    pub amount_y: Option<Decimal>,
    pub price_x_usd: Option<Decimal>,
    pub price_y_usd: Option<Decimal>,
    pub value_usd: Option<Decimal>,
}

// The full cost-basis ledger for one position, oldest first -- everything a profit-and-hold
// calculation is derived from (item 3). Deliberately returns every input row rather than a
// pre-summed total, so the caller can recompute in USD or in native token terms and so the
// number stays auditable back to the confirmed transaction that produced each row.
pub async fn cash_flows_for_position(
    pool: &PgPool,
    position_id: Uuid,
) -> eyre::Result<Vec<CashFlowRow>> {
    let rows = sqlx::query_as!(
        CashFlowRow,
        r#"
        SELECT transaction_intent_id, kind, ts, amount_x, amount_y, price_x_usd, price_y_usd, value_usd
        FROM position_cash_flows
        WHERE position_id = $1
        ORDER BY ts
        "#,
        position_id,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Querying cash flows for position {position_id}"))?;

    Ok(rows)
}
