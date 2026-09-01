use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use eyre::WrapErr;
use futures::StreamExt;
use sqlx::PgPool;
use teloxide::Bot;
use teloxide::payloads::SendMessageSetters;
use teloxide::requests::Requester;
use teloxide::types::{BotCommand, ChatId, ParseMode, UpdateKind};
use teloxide::update_listeners::{AsUpdateStream, polling_default};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::auth::is_authorized;
use crate::cli;
use crate::config::Config;
use crate::mute::MuteStore;
use crate::ratelimit;
use crate::{handlers, render};

// Telegram allows roughly 1 message/second per chat. A paginated /why dump to one chat is
// the case that would actually hit this; the allow-list stays small enough in practice that
// the ~30/second global cap is never the binding constraint.
const PER_CHAT_MIN_GAP: Duration = Duration::from_millis(1050);

pub struct TelegramWorker {
    config: Config,
    pool: PgPool,
}

impl TelegramWorker {
    pub fn new(config: Config, pool: PgPool) -> Self {
        Self { config, pool }
    }
}

#[async_trait]
impl common::Worker for TelegramWorker {
    fn name(&self) -> &'static str {
        "telegram_bot"
    }

    async fn run(&self, ct: CancellationToken) -> eyre::Result<()> {
        let bot = Bot::new(self.config.bot_token.clone());
        register_commands(&bot).await?;

        let mutes = Arc::new(MuteStore::new());
        let last_sent: Arc<AsyncMutex<HashMap<ChatId, Instant>>> =
            Arc::new(AsyncMutex::new(HashMap::new()));

        let mut listener = polling_default(bot.clone()).await;
        let updates = listener.as_stream();
        tokio::pin!(updates);

        loop {
            tokio::select! {
                biased;
                _ = ct.cancelled() => break,
                next = updates.next() => {
                    let Some(update) = next else { break };
                    let update = match update {
                        Ok(update) => update,
                        Err(e) => {
                            tracing::warn!(error = ?e, "Update listener error");
                            continue;
                        }
                    };

                    let UpdateKind::Message(message) = update.kind else { continue };
                    let Some(text) = message.text().map(str::to_string) else { continue };
                    let chat_id = message.chat.id;

                    let bot = bot.clone();
                    let pool = self.pool.clone();
                    let allowed = self.config.allowed_chats.clone();
                    let max_rows = self.config.max_rows;
                    let mutes = mutes.clone();
                    let last_sent = last_sent.clone();

                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_message(&bot, chat_id, &text, &pool, &allowed, max_rows, &mutes, &last_sent).await
                        {
                            tracing::error!(error = ?e, "Failed to handle a message");
                        }
                    });
                }
            }
        }

        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    pool: &PgPool,
    allowed: &[i64],
    max_rows: usize,
    mutes: &MuteStore,
    last_sent: &AsyncMutex<HashMap<ChatId, Instant>>,
) -> eyre::Result<()> {
    if !is_authorized(chat_id.0, allowed) {
        tracing::warn!(chat = chat_id.0, "Refused a message from an unlisted chat");
        return send(bot, chat_id, render::render_refusal(), last_sent).await;
    }

    if !text.starts_with('/') {
        return Ok(());
    }

    let command = match cli::parse_command(text) {
        Ok(command) => command,
        Err(e) => return send(bot, chat_id, render::render_parse_error(&e), last_sent).await,
    };

    let body = handlers::dispatch(pool, mutes, max_rows, command)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = ?e, "Command handler failed");
            render::render_internal_error()
        });

    send(bot, chat_id, body, last_sent).await
}

async fn send(
    bot: &Bot,
    chat_id: ChatId,
    body: String,
    last_sent: &AsyncMutex<HashMap<ChatId, Instant>>,
) -> eyre::Result<()> {
    for page in render::paginate(&body, render::MESSAGE_LIMIT) {
        {
            let mut map = last_sent.lock().await;
            let wait =
                ratelimit::wait_for(map.get(&chat_id).copied(), Instant::now(), PER_CHAT_MIN_GAP);
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            map.insert(chat_id, Instant::now());
        }

        bot.send_message(chat_id, page)
            .parse_mode(ParseMode::MarkdownV2)
            .await
            .wrap_err_with(|| format!("Sending a message to chat {}", chat_id.0))?;
    }

    Ok(())
}

async fn register_commands(bot: &Bot) -> eyre::Result<()> {
    let commands = vec![
        BotCommand::new("top", "Activity ranking for a timeframe"),
        BotCommand::new(
            "volume",
            "Volume-to-TVL ranking with change vs the previous bucket",
        ),
        BotCommand::new("potential", "Our own gate-filtered ranking"),
        BotCommand::new("pool", "Pool metadata and every timeframe"),
        BotCommand::new(
            "why",
            "Full rationale for a pool, including why it did not qualify",
        ),
        BotCommand::new("watch", "Force or release tier-1 membership for a pool"),
        BotCommand::new("mute", "Suppress signals for a pool for a duration"),
        BotCommand::new("status", "Ingest health and tier size"),
    ];

    bot.set_my_commands(commands)
        .await
        .wrap_err_with(|| "Registering bot commands")?;

    Ok(())
}
