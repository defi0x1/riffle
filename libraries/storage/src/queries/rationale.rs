use chrono::{DateTime, Utc};
use eyre::WrapErr;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct RationaleItem {
    pub signal_id: Uuid,
    pub seq: i32,
    pub venue: i16,
    pub signal: String,
    pub observed: Option<String>,
    pub cmp: Option<String>,
    pub threshold: Option<String>,
    pub passed: bool,
    pub note: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SignalWithRationale {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    pub pool_address: String,
    pub venue: i16,
    pub timeframe: String,
    pub kind: String,
    pub regime: Option<String>,
    pub items: Vec<RationaleItem>,
}

// Every evaluated condition, pass or fail, for the most recent signal a pool produced at or
// before `at` -- this is what `/why` explains, including a pool with no signal at all (an
// evaluation that passed every gate but never crossed the POTENTIAL threshold still writes
// rationale).
pub async fn rationale_for(
    pool: &PgPool,
    pool_address: &str,
    at: DateTime<Utc>,
) -> eyre::Result<Option<SignalWithRationale>> {
    let signal = sqlx::query!(
        r#"
        SELECT id, ts, pool_address, venue, timeframe, kind, regime
        FROM signals
        WHERE pool_address = $1 AND ts <= $2
        ORDER BY ts DESC
        LIMIT 1
        "#,
        pool_address,
        at,
    )
    .fetch_optional(pool)
    .await
    .wrap_err_with(|| format!("Fetching latest signal for {pool_address}"))?;

    let Some(signal) = signal else {
        return Ok(None);
    };

    let items = sqlx::query_as!(
        RationaleItem,
        r#"
        SELECT signal_id, seq, venue, signal, observed, cmp, threshold, passed, note
        FROM rationale
        WHERE signal_id = $1
        ORDER BY seq
        "#,
        signal.id,
    )
    .fetch_all(pool)
    .await
    .wrap_err_with(|| format!("Fetching rationale for signal {}", signal.id))?;

    Ok(Some(SignalWithRationale {
        id: signal.id,
        ts: signal.ts,
        pool_address: signal.pool_address,
        venue: signal.venue,
        timeframe: signal.timeframe,
        kind: signal.kind,
        regime: signal.regime,
        items,
    }))
}
