use std::sync::Once;

use clap::{Parser, ValueEnum};
use eyre::WrapErr;
use tracing_subscriber::EnvFilter;

// Available for downstream crates that prefer `logger::info!` over a direct `tracing` import.
// House style is the direct import; this only avoids forcing a second dependency line.
pub use tracing::{self, debug, error, info, trace, warn};

static INIT: Once = Once::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    Compact,
    Full,
    Json,
    Pretty,
}

#[derive(Parser, Debug, Clone)]
#[group(id = "log")]
pub struct Config {
    /// Log level, passed through to `tracing_subscriber::EnvFilter` (e.g. "info",
    /// "debug", "warn,sqlx=info").
    #[arg(long, env, default_value = "info")]
    pub log_level: String,

    /// Log output format.
    #[arg(long, env, value_enum, default_value_t = LogFormat::Compact)]
    pub log_format: LogFormat,
}

impl Config {
    // Guarded so a binary composing several config groups that each call `init()`
    // (or a test harness that calls it repeatedly) does not panic on the second install.
    pub fn init(&self) -> eyre::Result<()> {
        let mut result = Ok(());
        INIT.call_once(|| {
            result = self.install();
        });
        result
    }

    fn install(&self) -> eyre::Result<()> {
        let filter = EnvFilter::try_new(&self.log_level).wrap_err_with(|| "Parsing log level")?;
        let subscriber = tracing_subscriber::fmt().with_env_filter(filter).with_line_number(true);

        match self.log_format {
            LogFormat::Compact => subscriber.compact().init(),
            LogFormat::Full => subscriber.init(),
            LogFormat::Json => subscriber.json().init(),
            LogFormat::Pretty => subscriber.pretty().init(),
        }

        Ok(())
    }
}
