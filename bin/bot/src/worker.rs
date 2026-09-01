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
use teloxide::types::{
    BotCommand, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode, UpdateKind,
};
use teloxide::update_listeners::{AsUpdateStream, polling_default};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::auth::is_authorized;
use crate::cli;
use crate::config::Config;
use crate::handlers::{Context, DispatchOutcome};
use crate::ratelimit;
use crate::secret_guard::wallet_message_carries_key_material;
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
                    // u64 -> i64: real Telegram user ids sit nowhere near i64::MAX, and this is
                    // the same BIGINT convention wallets.telegram_user_id already uses.
                    let telegram_user_id = message.from.as_ref().map(|u| u.id.0 as i64);

                    let bot = bot.clone();
                    let pool = self.pool.clone();
                    let config = self.config.clone();
                    let last_sent = last_sent.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_message(
                            &bot,
                            chat_id,
                            telegram_user_id,
                            &text,
                            &pool,
                            &config,
                            &last_sent,
                        )
                        .await
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

async fn handle_message(
    bot: &Bot,
    chat_id: ChatId,
    telegram_user_id: Option<i64>,
    text: &str,
    pool: &PgPool,
    config: &Config,
    last_sent: &AsyncMutex<HashMap<ChatId, Instant>>,
) -> eyre::Result<()> {
    if !is_authorized(chat_id.0, &config.allowed_chats) {
        tracing::warn!(chat = chat_id.0, "Refused a message from an unlisted chat");
        return send(
            bot,
            chat_id,
            DispatchOutcome::text(render::render_refusal()),
            last_sent,
        )
        .await;
    }

    if !text.starts_with('/') {
        return Ok(());
    }

    // Checked on the raw text, before clap ever tokenizes it: a seed phrase is many
    // whitespace-separated tokens, and clap's own parse-error text would otherwise echo one of
    // them straight back into the chat. Nothing about `text` is logged either way.
    if wallet_message_carries_key_material(text) {
        tracing::warn!(
            chat = chat_id.0,
            "Refused a /wallet message shaped like key material"
        );
        return send(
            bot,
            chat_id,
            DispatchOutcome::text(render::render_key_material_refusal()),
            last_sent,
        )
        .await;
    }

    let command = match cli::parse_command(text) {
        Ok(command) => command,
        Err(e) => {
            return send(
                bot,
                chat_id,
                DispatchOutcome::text(render::render_parse_error(&e)),
                last_sent,
            )
            .await;
        }
    };

    let ctx = Context {
        chat_id: chat_id.0,
        telegram_user_id,
        max_rows: config.max_rows,
        miniapp_base_url: config.miniapp_base_url.clone(),
        max_add_value_usd: config.max_add_value_usd,
    };

    let outcome = handlers::dispatch(pool, &ctx, command)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = ?e, "Command handler failed");
            DispatchOutcome::text(render::render_internal_error())
        });

    send(bot, chat_id, outcome, last_sent).await
}

async fn send(
    bot: &Bot,
    chat_id: ChatId,
    outcome: DispatchOutcome,
    last_sent: &AsyncMutex<HashMap<ChatId, Instant>>,
) -> eyre::Result<()> {
    let pages = render::paginate(&outcome.body, render::MESSAGE_LIMIT);
    let last_index = pages.len().saturating_sub(1);

    for (i, page) in pages.into_iter().enumerate() {
        {
            let mut map = last_sent.lock().await;
            let wait =
                ratelimit::wait_for(map.get(&chat_id).copied(), Instant::now(), PER_CHAT_MIN_GAP);
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            map.insert(chat_id, Instant::now());
        }

        let mut request = bot
            .send_message(chat_id, page)
            .parse_mode(ParseMode::MarkdownV2);
        // The button only ever rides on the last page -- there is exactly one proposal per
        // command, however many pages its text takes to say it.
        if i == last_index
            && let Some(button) = &outcome.button
        {
            let keyboard = InlineKeyboardMarkup::new([[InlineKeyboardButton::url(
                button.label.clone(),
                button.url.clone(),
            )]]);
            request = request.reply_markup(keyboard);
        }

        request
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
        BotCommand::new("wallet", "Register your public key, or list your wallets"),
        BotCommand::new("balance", "Latest token balances for a registered wallet"),
        BotCommand::new("positions", "Open positions for a registered wallet"),
        BotCommand::new("profit", "Profit for one of your positions"),
        BotCommand::new(
            "open",
            "Propose opening a new position (signed in the Mini App)",
        ),
        BotCommand::new("add", "Propose adding liquidity (signed in the Mini App)"),
        BotCommand::new(
            "remove",
            "Propose removing liquidity (signed in the Mini App)",
        ),
        BotCommand::new("claim", "Propose claiming fees (signed in the Mini App)"),
        BotCommand::new(
            "close",
            "Propose closing a position (signed in the Mini App)",
        ),
    ];

    bot.set_my_commands(commands)
        .await
        .wrap_err_with(|| "Registering bot commands")?;

    Ok(())
}
