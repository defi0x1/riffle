use clap::Parser;
use rust_decimal::Decimal;

pub mod defaults {
    use rust_decimal::Decimal;

    // Rows shown per ranking command before the caller has to ask for /pool on a specific
    // address. Kept small deliberately -- a phone screen and a 4096-character message both
    // punish a long table.
    pub const MAX_ROWS: usize = 10;

    // Per-`/add` cap, in USD, above which the proposal is refused rather than shown. Only
    // enforced when a position already carries a priced valuation to estimate against (see
    // handlers::add) -- an advisory limit on top of, not instead of, the Mini App's own review.
    pub fn max_add_value_usd() -> Decimal {
        Decimal::new(5_000, 0)
    }
}

#[derive(Parser, Debug, Clone)]
#[group(id = "telegram")]
pub struct Config {
    /// Telegram bot token issued by BotFather. Never logged: see the Display impl below.
    #[arg(long, env)]
    pub bot_token: String,

    /// Chat IDs allowed to use the bot, comma-separated (e.g. "123456789,-987654321";
    /// negative IDs are groups/channels). Every other chat gets a refusal and nothing else.
    /// Required -- there is no default that would make sense here.
    #[arg(long, env, value_delimiter = ',', required = true)]
    pub allowed_chats: Vec<i64>,

    /// Rows rendered per ranking command before the message paginates.
    #[arg(long, env, default_value_t = defaults::MAX_ROWS)]
    pub max_rows: usize,

    /// Base URL for the Mini App's direct link (e.g. "https://t.me/FeeFarmBot/app"), used to
    /// build the "review and sign" button under every fund-moving proposal. Required -- there
    /// is no fallback location for where signing happens; the chat itself can never sign.
    #[arg(long, env)]
    pub miniapp_base_url: reqwest::Url,

    /// Advisory per-`/add` cap in USD; see `defaults::max_add_value_usd`.
    #[arg(long, env, default_value_t = defaults::max_add_value_usd())]
    pub max_add_value_usd: Decimal,
}

// bot_token is a bearer credential for the whole bot; it must never reach a log line, so
// Display is written by hand rather than derived.
impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "telegram::Config {{ bot_token: <redacted>, allowed_chats: {:?}, max_rows: {}, \
             miniapp_base_url: {}, max_add_value_usd: {} }}",
            self.allowed_chats, self.max_rows, self.miniapp_base_url, self.max_add_value_usd
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> Config {
        Config {
            bot_token: "000000:AAsecretsecretsecret".to_string(),
            allowed_chats: vec![1],
            max_rows: defaults::MAX_ROWS,
            miniapp_base_url: "https://t.me/FeeFarmBot/app".parse().unwrap(),
            max_add_value_usd: defaults::max_add_value_usd(),
        }
    }

    #[test]
    fn test_display_redacts_bot_token() {
        let rendered = sample_config().to_string();
        assert!(!rendered.contains("AAsecretsecretsecret"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn test_display_still_carries_non_secret_fields() {
        let rendered = sample_config().to_string();
        assert!(rendered.contains("t.me/FeeFarmBot"));
        assert!(rendered.contains("5000"));
    }
}
