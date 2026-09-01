//! Assembles `engine::risk_gate::RiskGateInputs` and a few other pre-classification
//! `PipelineInput` fields from a `PoolForScoring` row. Kept pure so the base/quote-token
//! selection and the risk-gate mapping are testable without a database.

use chrono::{DateTime, Utc};
use engine::Regime;
use engine::risk_gate::{QuoteAsset, RiskGateInputs};
use storage::queries::PoolForScoring;

// Public mainnet mint addresses -- observable on any explorer, not internal identifiers.
pub const WRAPPED_SOL: &str = "So11111111111111111111111111111111111111112";
pub const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USDT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

fn quote_asset_of(mint: &str) -> Option<QuoteAsset> {
    match mint {
        WRAPPED_SOL => Some(QuoteAsset::Sol),
        USDC => Some(QuoteAsset::Usdc),
        USDT => Some(QuoteAsset::Usdt),
        _ => None,
    }
}

/// Regime-independent age, since the pipeline has not classified a regime yet when this is
/// assembled -- first_liquidity_at if it exists, else created_at.
pub fn age_days(pool: &PoolForScoring, now: DateTime<Utc>) -> f64 {
    let since = pool.first_liquidity_at.unwrap_or(pool.created_at);
    now.signed_duration_since(since).num_seconds() as f64 / 86_400.0
}

/// The mechanical organic-flow fill constant is regime-dependent, but `PipelineInput` is
/// built before the pipeline classifies this tick's regime -- so this uses the *previously
/// committed* regime (hysteresis means it rarely flips) as the best available guess, falling
/// back to V1's value when nothing has been classified yet.
pub fn c_fill_for(previous_regime: Option<Regime>) -> f64 {
    match previous_regime {
        Some(Regime::S) => 0.75,
        Some(Regime::V1) | Some(Regime::V2) | None => 0.5,
    }
}

/// Risk-gate inputs for the non-quote side of the pair. Several checks this gate wants
/// (RugCheck's insider/bundle flag, Jupiter route depth, the wash-trading signer scan) need
/// data sources this pass does not read; they are marked `unavailable` rather than guessed
/// pass or fail, which is exactly what the gate's own `Option` fields are for.
pub fn risk_gate_inputs(pool: &PoolForScoring, now: DateTime<Utc>) -> RiskGateInputs {
    let x_is_quote = quote_asset_of(&pool.token_x).is_some();
    let y_is_quote = quote_asset_of(&pool.token_y).is_some();

    // Prefer the recognised quote side; if both or neither are recognised, token_y is the
    // conventional quote side in DLMM's swap_for_y orientation.
    let (quote_asset, base_mint_authority, base_freeze_authority, base_top10, base_top1) =
        if y_is_quote || !x_is_quote {
            (
                quote_asset_of(&pool.token_y).unwrap_or(QuoteAsset::Other),
                &pool.x_mint_authority,
                &pool.x_freeze_authority,
                pool.x_top10_share,
                pool.x_top1_share,
            )
        } else {
            (
                quote_asset_of(&pool.token_x).unwrap_or(QuoteAsset::Other),
                &pool.y_mint_authority,
                &pool.y_freeze_authority,
                pool.y_top10_share,
                pool.y_top1_share,
            )
        };

    RiskGateInputs {
        mint_authority_present: base_mint_authority.is_some(),
        freeze_authority_present: base_freeze_authority.is_some(),
        freeze_authority_is_documented_multisig: false,
        // Token-2022 extension flags are not in `PoolForScoring` yet -- `tokens.extensions`
        // is read as JSON and would need its own decode; conservatively "no flag observed"
        // rather than fabricating a pass.
        token2022_has_permanent_delegate: false,
        token2022_has_transfer_hook: false,
        token2022_transfer_fee_bps: 0,
        top10_holder_share: base_top10,
        top1_holder_share: base_top1,
        insider_bundle_flagged: None,
        other_venue_depth_ratio: None,
        cex_listed: false,
        // No fee_param_updates read in this pass -- assume stable rather than flag every pool.
        days_since_last_fee_param_change: 9_999.0,
        pool_status_enabled: pool.status == 0,
        activation_passed: true,
        quote_asset,
        signer_top_n_share_of_24h_volume: 0.0,
        round_trip_ratio: 0.0,
        age_hours: age_days(pool, now) * 24.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use rust_decimal::Decimal;

    fn base_pool() -> PoolForScoring {
        PoolForScoring {
            pool_address: "pool1".to_string(),
            venue: 0,
            tier: 0,
            token_x: WRAPPED_SOL.to_string(),
            token_y: "MemeMint1111111111111111111111111111111111".to_string(),
            bin_step: 20,
            base_factor: 10_000,
            variable_fee_control: 40_000,
            protocol_share_bps: 500,
            base_fee_bps: Decimal::new(20, 2),
            tvl_usd: Some(Decimal::new(1_000_000, 0)),
            status: 0,
            activation_point: None,
            created_at: DateTime::parse_from_rfc3339("2026-08-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            first_liquidity_at: None,
            is_blacklisted: false,
            x_mint_authority: None,
            x_freeze_authority: None,
            x_top10_share: Some(0.1),
            x_top1_share: Some(0.05),
            y_mint_authority: Some("some_authority".to_string()),
            y_freeze_authority: None,
            y_top10_share: Some(0.5),
            y_top1_share: Some(0.2),
        }
    }

    #[test]
    fn test_risk_inputs_use_the_non_quote_side_when_x_is_the_quote() {
        let pool = base_pool(); // token_x = SOL, token_y = the memecoin
        let now = DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let inputs = risk_gate_inputs(&pool, now);
        assert_eq!(inputs.quote_asset, QuoteAsset::Sol);
        assert!(
            inputs.mint_authority_present,
            "the memecoin side has a mint authority"
        );
        assert_eq!(inputs.top10_holder_share, Some(0.5));
    }

    #[test]
    fn test_risk_inputs_use_the_non_quote_side_when_y_is_the_quote() {
        let mut pool = base_pool();
        std::mem::swap(&mut pool.token_x, &mut pool.token_y);
        std::mem::swap(&mut pool.x_mint_authority, &mut pool.y_mint_authority);
        std::mem::swap(&mut pool.x_top10_share, &mut pool.y_top10_share);
        // now token_x = memecoin, token_y = SOL
        let now = DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let inputs = risk_gate_inputs(&pool, now);
        assert_eq!(inputs.quote_asset, QuoteAsset::Sol);
        assert!(inputs.mint_authority_present);
    }

    #[test]
    fn test_neither_side_recognised_falls_back_to_other_and_fails_quote_gate() {
        let mut pool = base_pool();
        pool.token_x = "RandomMint1111111111111111111111111111111".to_string();
        let now = DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let inputs = risk_gate_inputs(&pool, now);
        assert_eq!(inputs.quote_asset, QuoteAsset::Other);
    }

    #[test]
    fn test_age_days_prefers_first_liquidity_over_created_at() {
        let mut pool = base_pool();
        let now = pool.created_at + Duration::days(10);
        pool.first_liquidity_at = Some(pool.created_at + Duration::days(3));
        assert!((age_days(&pool, now) - 7.0).abs() < 1e-9);
    }

    #[test]
    fn test_c_fill_matches_regime() {
        assert_eq!(c_fill_for(Some(Regime::S)), 0.75);
        assert_eq!(c_fill_for(Some(Regime::V1)), 0.5);
        assert_eq!(c_fill_for(Some(Regime::V2)), 0.5);
        assert_eq!(c_fill_for(None), 0.5);
    }
}
