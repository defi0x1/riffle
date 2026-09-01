use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

/// Worker tick intervals and other scorer-only knobs. Durations are `humantime`, matching
/// house style. Every other subsystem's config (`logger`, `storage`, `engine`, `metrics`) is
/// flattened alongside this in `Args`.
#[derive(Parser, Debug, Clone)]
#[group(id = "tick")]
pub struct TickConfig {
    /// How often the rollup worker builds a `pool_metrics_5m`/`pool_metrics_10m` bucket.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "5m")]
    pub rollup_interval: Duration,

    /// How often the indicator worker screens the universe and ranks the watch set.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "5m")]
    pub indicators_interval: Duration,

    /// How often the signal worker re-evaluates trigger conditions.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "5m")]
    pub signals_interval: Duration,

    /// How often a persistent signal condition may be re-announced.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "1h")]
    pub signal_cooldown: Duration,

    /// How often open paper positions are marked against real fee accrual.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "5m")]
    pub paper_position_mark_interval: Duration,

    /// How often the paper-position worker checks for outcomes due to finalise.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = "15m")]
    pub outcomes_interval: Duration,
}

/// Pipeline inputs the engine needs but does not itself expose as config -- estimated
/// constants and portfolio-sizing placeholders. No calibrated value ships with this repo;
/// these are neutral defaults, the same convention `engine`'s own config groups use.
#[derive(Parser, Debug, Clone)]
#[group(id = "pipeline_defaults")]
pub struct PipelineDefaultsConfig {
    /// Fee-clustering multiplier for the endogenous fee forecast.
    #[arg(long, env, default_value_t = 3.0)]
    pub kappa_c: f64,
    /// Decay window (seconds) feeding the forecast-fee volatility term.
    #[arg(long, env, default_value_t = 600.0)]
    pub decay_window_secs: f64,
    /// Organic-flow shrinkage prior mean, until per-class priors are estimated.
    #[arg(long, env, default_value_t = 0.6)]
    pub organic_class_prior_mu: f64,
    /// Organic-flow shrinkage prior variance, until per-class priors are estimated.
    #[arg(long, env, default_value_t = 0.05)]
    pub organic_class_prior_tau_sq: f64,
    /// Capital assumed available to the current regime bucket for sizing. No portfolio
    /// ledger exists yet, so this is a flat placeholder rather than a tracked balance.
    #[arg(long, env, default_value_t = 1_000_000.0)]
    pub regime_capital: f64,
    /// Capital assumed free (uncommitted to other positions) for sizing.
    #[arg(long, env, default_value_t = 200_000.0)]
    pub free_capital: f64,
    /// Expected fee return used by the quarter-Kelly sizing input.
    #[arg(long, env, default_value_t = 0.001)]
    pub mu_fee: f64,
    /// Expected adverse-selection cost used by the quarter-Kelly sizing input.
    #[arg(long, env, default_value_t = 0.0002)]
    pub mu_arb: f64,
}

/// Thin clap dispatcher: every subsystem's config flattened together, logged once at
/// startup with secrets redacted.
#[derive(Parser, Debug, Clone)]
pub struct Args {
    #[clap(flatten)]
    pub logging: logger::Config,
    #[clap(flatten)]
    pub postgres: common::PostgresConfig,
    #[clap(flatten)]
    pub engine: engine::EngineConfig,
    #[clap(flatten)]
    pub metrics: metrics::Config,
    #[clap(flatten)]
    pub tick: TickConfig,
    #[clap(flatten)]
    pub pipeline_defaults: PipelineDefaultsConfig,

    /// Load settings from a YAML file (see config/scorer.example.yaml). A flag or
    /// environment variable of the same name still overrides anything set here. Omit this
    /// and the binary behaves exactly as it always has: flags and environment variables
    /// only.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

// engine::EngineConfig and TickConfig carry no secrets, so this only needs to redact the
// postgres URL -- the one field the `Display` convention exists for.
impl std::fmt::Display for Args {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Args {{ logging: {:?}, postgres: {}, metrics: {:?}, tick: {:?}, pipeline_defaults: {:?} }}",
            self.logging, self.postgres, self.metrics, self.tick, self.pipeline_defaults
        )
    }
}
