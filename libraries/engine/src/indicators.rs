use chrono::{DateTime, Utc};
use dlmm_math::VenueId;
use rust_decimal::Decimal;

/// Provenance of a pool's `Indicators` row. `A` is measured from real bin state (the rank
/// stage, watched pools only); `B` is estimated from `TVL × φ_shape` (the screen stage,
/// every pool). Only `A` counts toward outcome scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    A,
    B,
}

impl Quality {
    pub fn as_char(self) -> char {
        match self {
            Quality::A => 'A',
            Quality::B => 'B',
        }
    }
}

impl std::fmt::Display for Quality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_char())
    }
}

/// The three tradeable regimes. `None` on an `Indicators` row means unclassifiable (e.g.
/// cold start), matching the migration's nullable `regime` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Regime {
    S,
    V1,
    V2,
}

impl std::fmt::Display for Regime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Regime::S => "S",
            Regime::V1 => "V1",
            Regime::V2 => "V2",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for Regime {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "S" => Ok(Regime::S),
            "V1" => Ok(Regime::V1),
            "V2" => Ok(Regime::V2),
            other => Err(eyre::eyre!("Unknown regime {other}")),
        }
    }
}

/// One row of `indicators_{tf}`. Column names and nullability mirror
/// `migrations/0015_indicators.sql` exactly; only `f_hat` uses `Decimal` rather than
/// `f64`, matching the column's `NUMERIC(20,6)` type.
#[derive(Debug, Clone)]
pub struct Indicators {
    pub pool_address: String,
    pub venue: VenueId,
    pub bucket_start: DateTime<Utc>,
    pub quality: Quality,
    pub regime: Option<Regime>,

    pub vol_change: Option<f64>,
    pub fee_change: Option<f64>,
    pub tvl_change: Option<f64>,
    pub price_change: Option<f64>,
    pub active_tvl_change: Option<f64>,
    pub holders_change: Option<f64>,

    pub vol_tvl: Option<f64>,
    pub fee_tvl: Option<f64>,
    pub fee_active_tvl: Option<f64>,
    pub tau_a: Option<f64>,

    pub sigma_gk: Option<f64>,
    pub sigma_fast: Option<f64>,
    pub sigma_slow: Option<f64>,
    pub sigma_d: Option<f64>,
    pub sigma_jump: Option<f64>,

    pub f_hat: Option<Decimal>,
    pub phi_org: Option<f64>,
    pub phi_mech: Option<f64>,
    pub phi_time: Option<f64>,
    pub phi_size: Option<f64>,
    pub r_gross: Option<f64>,
    pub r_org: Option<f64>,
    pub y_fee: Option<f64>,

    pub top_score: Option<f64>,
}

impl Indicators {
    /// An empty row for `pool_address`/`venue`/`bucket_start`/`quality` — the pipeline
    /// fills the rest in stage order.
    pub fn empty(
        pool_address: String,
        venue: VenueId,
        bucket_start: DateTime<Utc>,
        quality: Quality,
    ) -> Self {
        Self {
            pool_address,
            venue,
            bucket_start,
            quality,
            regime: None,
            vol_change: None,
            fee_change: None,
            tvl_change: None,
            price_change: None,
            active_tvl_change: None,
            holders_change: None,
            vol_tvl: None,
            fee_tvl: None,
            fee_active_tvl: None,
            tau_a: None,
            sigma_gk: None,
            sigma_fast: None,
            sigma_slow: None,
            sigma_d: None,
            sigma_jump: None,
            f_hat: None,
            phi_org: None,
            phi_mech: None,
            phi_time: None,
            phi_size: None,
            r_gross: None,
            r_org: None,
            y_fee: None,
            top_score: None,
        }
    }
}

/// `venue` as the `SMALLINT` the migration stores (`0 = DLMM, 1 = DAMM_V2`).
pub fn venue_smallint(venue: VenueId) -> i16 {
    match venue {
        VenueId::Dlmm => 0,
        VenueId::DammV2 => 1,
    }
}
