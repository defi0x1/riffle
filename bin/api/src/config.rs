use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

pub mod defaults {
    pub const PORT: u16 = 8090;

    /// initData carries no single-use nonce by Telegram's own design (see telegram_auth's
    /// module comment); auth_date recency is the only replay defense. A day is generous but
    /// still bounds how long a captured header stays useful if it ever leaked.
    pub const INIT_DATA_MAX_AGE: &str = "86400s";

    /// A deposit beyond this many raw base units on either side of the pair is refused at
    /// build time. This is advisory, not a security control -- the miniapp's own README
    /// explains why a keyless backend cannot enforce a real spending cap (it never holds a key
    /// capable of refusing to *sign*) -- but it is still a real refusal before a transaction is
    /// ever built, which is what this task asks for.
    pub const MAX_AMOUNT_RAW: u64 = 1_000_000_000_000;

    pub const COMPUTE_UNIT_LIMIT: u32 = 400_000;
    pub const INTENT_EXPIRY: &str = "90s";
    pub const CONFIRMATION_TIMEOUT: &str = "45s";
    pub const CONFIRMATION_POLL_INTERVAL: &str = "1500ms";
}

#[derive(Parser, Debug, Clone)]
pub struct Args {
    #[clap(flatten)]
    pub logging: logger::Config,

    #[clap(flatten)]
    pub postgres: common::PostgresConfig,

    #[clap(flatten)]
    pub metrics: metrics::Config,

    /// Telegram bot token issued by BotFather -- used only to recompute the HMAC that
    /// authenticates initData server-side, never sent anywhere or returned in a response. See
    /// the Display impl below.
    #[arg(long, env)]
    pub bot_token: String,

    /// Solana RPC endpoint used for simulation, submission, and every account read this
    /// service needs (pool state, position state, mint programs, wallet balances). May carry
    /// an embedded API key, so it is redacted in Display below, same as
    /// PostgresConfig::database_url.
    #[arg(long, env)]
    pub rpc_url: String,

    /// Port the HTTP API listens on.
    #[arg(long, env, default_value_t = defaults::PORT)]
    pub port: u16,

    /// Maximum age of a Telegram initData `auth_date` before a request is refused.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = defaults::INIT_DATA_MAX_AGE)]
    pub init_data_max_age: Duration,

    /// Per-side cap on a deposit's raw token amount. See MAX_AMOUNT_RAW's own comment.
    #[arg(long, env, default_value_t = defaults::MAX_AMOUNT_RAW)]
    pub max_amount_raw: u64,

    /// Compute unit limit attached to every built transaction.
    #[arg(long, env, default_value_t = defaults::COMPUTE_UNIT_LIMIT)]
    pub compute_unit_limit: u32,

    /// Compute unit price, in micro-lamports, attached to every built transaction. Zero omits
    /// the SetComputeUnitPrice instruction entirely rather than sending a literal zero-price
    /// one.
    #[arg(long, env, default_value_t = 0)]
    pub compute_unit_price_micro_lamports: u64,

    /// How far in the future transaction_intents.expires_at is set from creation -- long
    /// enough to outlive a Solana blockhash (roughly 60-90s) plus a user's review time.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = defaults::INTENT_EXPIRY)]
    pub intent_expiry: Duration,

    /// How long POST /tx/submit blocks polling getSignatureStatuses before giving up and
    /// returning a "submitted" (not yet confirmed) status for the client to poll further via
    /// GET /tx/status.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = defaults::CONFIRMATION_TIMEOUT)]
    pub confirmation_timeout: Duration,

    /// Interval between getSignatureStatuses polls while waiting on confirmation.
    #[arg(long, env, value_parser = humantime::parse_duration, default_value = defaults::CONFIRMATION_POLL_INTERVAL)]
    pub confirmation_poll_interval: Duration,

    /// Load settings from a YAML file. A flag or environment variable of the same name still
    /// overrides anything set here.
    #[arg(long)]
    pub config: Option<PathBuf>,
}

// bot_token, database_url and rpc_url must never reach a log line, so this is written by hand
// rather than derived, matching every other binary's config Display in this workspace.
impl std::fmt::Display for Args {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "api::Args {{ log_level: {}, log_format: {:?}, postgres: {}, metrics_port: {}, \
             bot_token: <redacted>, rpc_url: <redacted>, port: {}, init_data_max_age: {:?}, \
             max_amount_raw: {}, compute_unit_limit: {}, compute_unit_price_micro_lamports: {}, \
             intent_expiry: {:?}, confirmation_timeout: {:?}, confirmation_poll_interval: {:?} }}",
            self.logging.log_level,
            self.logging.log_format,
            self.postgres,
            self.metrics.metrics_port,
            self.port,
            self.init_data_max_age,
            self.max_amount_raw,
            self.compute_unit_limit,
            self.compute_unit_price_micro_lamports,
            self.intent_expiry,
            self.confirmation_timeout,
            self.confirmation_poll_interval,
        )
    }
}
