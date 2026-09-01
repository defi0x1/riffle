use clap::Parser;

pub mod defaults {
    // Rows shown per ranking command before the caller has to ask for /pool on a specific
    // address. Kept small deliberately -- a phone screen and a 4096-character message both
    // punish a long table.
    pub const MAX_ROWS: usize = 10;
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
}

// bot_token is a bearer credential for the whole bot; it must never reach a log line, so
// Display is written by hand rather than derived.
impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "telegram::Config {{ bot_token: <redacted>, allowed_chats: {:?}, max_rows: {} }}",
            self.allowed_chats, self.max_rows
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_redacts_bot_token() {
        let config = Config {
            bot_token: "000000:AAsecretsecretsecret".to_string(),
            allowed_chats: vec![1],
            max_rows: defaults::MAX_ROWS,
        };
        let rendered = config.to_string();
        assert!(!rendered.contains("AAsecretsecretsecret"));
        assert!(rendered.contains("<redacted>"));
    }
}
