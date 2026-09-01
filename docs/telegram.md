# Telegram bot

A walkthrough for taking the `bot` binary from nothing to answering commands in a real chat.
Everything here was read out of `bin/bot/src/` -- `main.rs`, `cli.rs`, `config.rs`, `auth.rs`,
`handlers.rs`, `render/`, `worker.rs`, `ratelimit.rs`, `mute.rs` -- and cross-checked against
`config/bot.example.yaml` and `.env.example`. The full flag reference for every binary, including
`bot`, lives in [`docs/configuration.md`](configuration.md); this document is only about the
Telegram-facing parts.

The bot is read-only. It never writes indicators, never sizes a position, never moves anything --
it renders rows that `scorer` already computed and persisted. The two exceptions are `/watch` and
`/mute`, which write an operator decision (force/release tier-1 membership, suppress a signal),
never a computed value.

## 1. Creating the bot with BotFather

1. Open a chat with `@BotFather` on Telegram.
2. Send `/newbot`.
3. Give it a display name (shown on the profile, can contain spaces).
4. Give it a username (must end in `bot`, e.g. `FeeFarmBot` or `my_feefarm_bot`).
5. BotFather replies with a token that looks like `123456789:AAExampleExampleExampleExampleExampl`.

That token is a bearer credential for the entire bot. Anyone holding it can send messages as the
bot, read everything it would otherwise see, and change its command list. It is not a secret you
paste into a file that gets committed:

- It goes into `.env` (gitignored, `.gitignore` line 2) or a real `config/*.yaml` file (gitignored
  by `/config/*.yaml` with `!/config/*.example.yaml` as the only exception), or straight into your
  shell environment.
- `.env.example` and `config/bot.example.yaml` are tracked and carry only placeholders
  (`000000000:...`) -- never overwrite one of those files with a real token.
- `bin/bot/src/config.rs` writes `Display` for the config struct by hand specifically so the token
  never reaches a log line; if you ever see it in a log, something upstream of that struct printed
  it directly.

If a token leaks, revoke it from BotFather with `/revoke` (or `/token` to rotate) and update
wherever you stored it.

## 2. Finding your chat id

The bot is allow-listed by chat id (`bin/bot/src/auth.rs`): every message from a chat not in
`ALLOWED_CHATS` gets a one-line refusal and nothing else runs. You need the id before you can
configure anything.

This works for both a direct message and a group, and does not depend on the bot already being
authorized (it talks to the raw Bot API, not to this binary):

1. Send any message to the bot in the chat you want the id for -- a DM to the bot itself, or any
   message in a group it has been added to.
2. In a browser or with `curl`, fetch:
   ```
   https://api.telegram.org/bot<YOUR_TOKEN>/getUpdates
   ```
3. In the JSON response, find the `"chat":{"id": ...}` for the message you just sent.

**Group ids are negative.** A direct-message chat id is a positive integer (e.g. `123456789`); a
group or supergroup id is negative (e.g. `-987654321`, and supergroups often use a larger
`-100...` form). Paste the sign along with the digits into `ALLOWED_CHATS` -- a group id typed in
as a positive number will never match, and every message from that group will be silently refused
with no hint that the sign was the problem. This is the detail most likely to cost real time.

## 3. Configuring the bot

The Telegram-specific settings, from `bin/bot/src/config.rs` (`#[group(id = "telegram")]` groups
these under `--help`, it does not add a flag prefix):

| Flag | Env var | Default | Required |
|---|---|---|---|
| `--bot-token` | `BOT_TOKEN` | none | yes |
| `--allowed-chats` | `ALLOWED_CHATS` | none | yes |
| `--max-rows` | `MAX_ROWS` | `10` | no |

`--allowed-chats` / `ALLOWED_CHATS` takes a comma-separated list, e.g.
`123456789,-987654321`. An empty value is accepted by `clap` but authorizes no one -- the bot
starts and runs, and every chat gets the refusal message; this is a silent no-op, not a startup
error, so an accidentally-empty value is easy to miss.

`--max-rows` / `MAX_ROWS` caps how many rows `/top`, `/volume` and `/potential` render before the
reply would need to paginate (default 10; see `bin/bot/src/config.rs`'s `defaults::MAX_ROWS`).

`bot` also takes the settings every binary in this workspace takes: `common::PostgresConfig`
(`--database-url`/`DATABASE_URL`, required; `--max-connections`/`MAX_CONNECTIONS`, default `10`),
`logger::Config` (`--log-level`/`LOG_LEVEL`, default `info`; `--log-format`/`LOG_FORMAT`, default
`compact`), and `metrics::Config` (`--disable-metrics-server`/`DISABLE_METRICS_SERVER`;
`--metrics-port`/`METRICS_PORT`, default `9101` -- `observability/prometheus/prometheus.yml`
expects `9103` for `bot`, so set it explicitly if you run more than one binary on the same host).

### Precedence

Four layers, most to least authoritative (`libraries/common/src/config.rs`):

1. a CLI flag
2. an environment variable of the same name
3. a key in a YAML file passed with `--config <path>`, in the same snake_case as the field
   (`bot_token`, `allowed_chats`, `max_rows`)
4. the field's own compiled default, where one exists

A config file only fills in a value that neither a flag nor a real environment variable already
supplied. An unrecognised key in the file is a startup error naming the key, not a silent no-op.
`--config` is CLI-only -- its own path cannot itself come from an environment variable or from the
file it names.

One thing worth flagging explicitly: **no Rust binary in this workspace reads `.env`
automatically.** `.env` is read by `sqlx-cli` (so `make migrate` picks it up), but `bot` reads only
the real process environment plus whatever `--config` injects. Filling in `.env` and running
`cargo run --bin bot` without exporting those variables first leaves `BOT_TOKEN`/`ALLOWED_CHATS`
unset and the process refuses to start.

### Minimal working configuration

Either export the two required variables directly:

```sh
export DATABASE_URL=postgres://feefarm:feefarm@localhost:5432/feefarm
export BOT_TOKEN=123456789:AAExampleExampleExampleExampleExampl
export ALLOWED_CHATS=123456789,-987654321
cargo run --bin bot
```

or copy `config/bot.example.yaml` to `config/bot.yaml` (gitignored -- see `.gitignore`), fill in
the real values, and point `--config` at it:

```yaml
database_url: postgres://feefarm:feefarm@localhost:5432/feefarm
bot_token: "123456789:AAExampleExampleExampleExampleExampl"
allowed_chats: "123456789,-987654321"
```

```sh
cargo run --bin bot -- --config config/bot.yaml
```

A flag or a real environment variable still overrides the file, so a value you export ahead of
time -- in CI, say -- wins over what the file says.

## 4. Registering the command list with BotFather

`bin/bot/src/worker.rs` calls Telegram's `setMyCommands` API itself, once, every time the bot
starts (`register_commands`, called from `TelegramWorker::run` before the polling loop starts).
So this step happens automatically -- you do not need to do anything in BotFather for the command
menu to work. It is worth doing anyway if you want the menu populated before you have run the bot
even once (some Telegram clients only refresh the command list on chat open, not live), or if you
want to see the exact command list without reading source.

To do it manually: message `@BotFather`, send `/setcommands`, pick your bot, then paste this block
(taken verbatim from `register_commands` in `bin/bot/src/worker.rs`):

```
top - Activity ranking for a timeframe
volume - Volume-to-TVL ranking with change vs the previous bucket
potential - Our own gate-filtered ranking
pool - Pool metadata and every timeframe
why - Full rationale for a pool, including why it did not qualify
watch - Force or release tier-1 membership for a pool
mute - Suppress signals for a pool for a duration
status - Ingest health and tier size
```

If you ever change the command set in code, this pasted block goes stale until the bot restarts
(which re-registers it automatically) or you paste an updated block by hand.

## 5. Running it

The bot only renders what is already in Postgres -- it computes nothing itself. Bring the rest of
the system up first, in this order:

1. **Postgres.** `make up` (docker compose: postgres+timescale, prometheus, grafana).
2. **Migrations.** `make migrate`, or start `indexer` or `scorer` first -- both call
   `storage::run_migrations` themselves at startup (`bin/indexer/src/lib.rs`,
   `bin/scorer/src/main.rs`). `bot`'s own `main.rs` does **not** call `run_migrations` -- it only
   opens a pool and starts polling. If you point `bot` at a database that has never been migrated,
   it starts up fine (nothing here touches the schema at startup) and then every command fails
   with a rendered `"that command failed unexpectedly; nothing was applied. Check the logs."` --
   the real Postgres error ("relation ... does not exist") only shows up in the bot's own logs,
   not in the chat.
3. **`scorer` running for a while.** `bot` reads tables `scorer` writes (pool metrics rollups,
   indicators, signals, watch set). Until `scorer` has ticked at least once against data `indexer`
   has written, every ranking command correctly reports empty rather than showing anything --
   see the troubleshooting section below.

Then start the bot itself:

```sh
cargo run --bin bot -- --config config/bot.yaml
```

or with `--release` / a built binary, same as any other binary in this workspace. There is no
separate startup flag for polling vs. webhooks -- `worker.rs` always uses long polling
(`teloxide::update_listeners::polling_default`).

## 6. Commands

Every command is dispatched through `bin/bot/src/cli.rs`'s `parse_command`, which runs the raw
message text through `clap`. That means `--help` on any subcommand works in chat (e.g.
`/pool --help`), and a malformed invocation gets `clap`'s own error text sent back rather than
being silently ignored.

Quality tags (`A measured` / `B estimated`, from `bin/bot/src/render/mod.rs`'s `quality_label`)
show up on every ranked row and mean:

- **A measured** -- the pool is in the watched (tier-1) set, and its ranking numbers come from
  real per-bin liquidity state.
- **B estimated** -- the pool is screening-only; its numbers come from a TVL-and-shape estimate,
  not measured bin state. `/potential` excludes these by default (see below); `/top` and
  `/volume` show both and tag which is which, since "what is hot right now" is not the same claim
  as "what actually pays."

### `/top [timeframe]`

Activity ranking -- what is hot right now, by the reproduced venue activity score (`top_score`).
Not gate-filtered. `timeframe` defaults to `5m`; valid values are `5m`, `10m`, `1h`, `4h`, `24h`.
Row count is capped at `--max-rows` (default 10).

```
Top pools (1h)
ranked by top_score (venue activity); not gate-filtered

1. SOL/USDC
   top_score 128.430  r_org 2.15  fee/tvl 0.0042  [A measured]
   7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
2. BONK/SOL
   top_score 96.210  r_org 1.40  fee/tvl 0.0021  [B estimated]
   9yLMzk4DX88e18TYKTEqcE6kCliuUsB94UAVSiJotBtV
```

If nothing has been ranked yet for that timeframe: `no ranked pools for this timeframe yet.`

### `/volume [timeframe]`

Highest volume-to-TVL ranking, with the change against the previous bucket riding along in the
same row. `timeframe` defaults to `5m`, same valid set as `/top`. Not gate-filtered.

```
Volume ranking (1h)
ranked by volume; not gate-filtered

1. SOL/USDC
   volume 1834200  change vs previous bucket 0.183  [A measured]
   7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
```

Empty state: `no ranked pools for this timeframe yet.`

### `/potential [timeframe]`

The one ranking that reads as a suggestion, not a listing: gate-filtered to quality-A (measured)
pools whose `r_org` clears the same breakeven threshold the query itself filtered on (`1.0`, the
`min_r_org` default in `storage::queries::PotentialPoolFilters`). `timeframe` defaults to `5m`.

```
Potential (1h)
gate-filtered: quality-A only, r_org at or above breakeven

1. SOL/USDC
   r_org 2.15 >= 1.00 (breakeven 1.0)  fee/tvl 0.0042  [A measured]
   7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
```

A pool you have muted in this chat (`/mute`) still appears here -- muting suppresses alerts, not
the ranking -- but is tagged `[MUTED]` at the end of its second line, so a suggestion for
something you asked to stay quiet about cannot be mistaken for an active recommendation. The mute
is per chat: muting in one chat does not tag the row for a different chat.

Empty state: `nothing clears the gate right now. Try /why on a specific pool to see the closest misses.`

### `/pool <address>`

Pool metadata plus every timeframe's indicators, all at once (not gate-filtered, no row cap --
this is a lookup, not a ranking).

```
Pool
7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
SOL/USDC  bin_step 20  base fee 20 bps  tier watched  tvl 1834200.50

5m [A measured]: r_org 2.15  fee/tvl 0.0042  vol/tvl 0.0810  regime V1
10m [A measured]: r_org 2.10  fee/tvl 0.0039  vol/tvl 0.0750  regime V1
1h: no data yet
4h: no data yet
24h: no data yet
```

`tier` is `watched` (tier-1, measured) or `universe` (screening-only). A timeframe with no
indicator row yet renders as `<label>: no data yet` rather than being omitted, so a reader can
tell "not computed yet" apart from "computed and empty."

Address not found in the `pools` table: `no pool found at <address>.`

### `/why <address>`

Full rationale for a pool: every condition the signal engine evaluated, and whether it passed.

```
Why 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU

kind potential  timeframe 1h  regime V1

PASS r_org: observed 2.15 >= threshold 1.50
FAIL vol_tvl: observed 0.90 >= threshold 1.00 note: below floor for V1
```

Two distinct "nothing here" states, worded differently on purpose:

- No signal on record at all for this pool: `no evaluation on record for this pool. Either it has
  never been promoted to measured status, or scoring has not run since it was -- a screening-only
  (quality B) pool is never gated into a signal.` -- silence, not a failed threshold.
- A signal exists but recorded no evaluated conditions: `no evaluated conditions were recorded for
  this signal.`

### `/watch <address> [off]`

Forces tier-1 (measured) membership for a pool, or releases it. With no third argument, promotes:

- Already watched: `<address> is already watched.`
- Not yet watched: promotes it and replies `<address> is now watched (tier-1, measured indicators
  from the next tick).`

With `off` as the third argument, releases:

- Not currently watched: `<address> is not currently watched (nothing to release).`
- Watched and released: `<address> released back to screening-only.`
- Watched but has an open paper position: the release is refused -- `<address> has an open paper
  position and stays watched until it closes -- releasing it now would corrupt the measurement in
  progress.` This is a known, deliberate limitation: `/watch off` cannot force a pool out of the
  watched set while `scorer` has an open paper position on it.

Unknown address (not in the `pools` table): `no pool found at <address>.`

### `/mute <address> <duration>`

Suppresses `/potential` alerts for a pool, in this chat only, for a duration (`humantime` syntax,
e.g. `2h`, `45m`, `90m`, `1h30m`). Requires the pool to already exist (`/pool` lookup happens
first): `no pool found at <address>.` otherwise.

```
7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU muted until 2026-09-01 18:30 UTC.
```

A malformed duration (e.g. `/mute <address> notaduration`) is rejected by `clap` before it ever
reaches this handler, and comes back as a parse error (see Troubleshooting).

### `/status`

Ingest lag per source, slot gaps, and current tier (watched-set) size. No arguments.

```
Status

config: a1b2c3d4 (as of 2026-08-30 12:05:00)
watched pools: 42

rpc: last_slot 301829123  slot_gap 0  decode_errors 0  write_latency 12 ms  at 2026-08-30 12:05:03
geyser: last_slot 301829130  slot_gap 0  decode_errors 0  write_latency 4 ms  at 2026-08-30 12:05:03
```

If no signal has ever been written: `config: no signals recorded yet.` If no ingest health row
exists yet: `no ingest health rows recorded yet.`

## 7. Group versus direct use

The bot behaves the same way in a group as in a DM, with two differences:

- **Command mentions.** Telegram sends `/top@YourBotName` (not bare `/top`) in a group whenever a
  command is ambiguous with another bot in the same chat, and some clients always append it.
  `bin/bot/src/cli.rs`'s `normalize_first_token` strips the leading `/` and truncates at `@` before
  handing the token to `clap`, so both `/top` and `/top@YourBotName` parse identically -- you do
  not need to do anything for this to work, and it is covered by
  `test_strips_group_mention_suffix`.
- **Everything else is ignored.** `handle_message` in `bin/bot/src/worker.rs` returns immediately
  for any text that does not start with `/` (`if !text.starts_with('/') { return Ok(()); }`) --
  the bot never replies to ordinary conversation, in a group or a DM.

On privacy mode: Telegram's Bot API always delivers a message that starts with `/` to a bot in a
group, regardless of that bot's group-privacy setting -- privacy mode only withholds ordinary,
non-command text. Since this bot only ever acts on messages starting with `/`, the default privacy
mode (enabled, set via BotFather when the bot was created) does not need to be turned off for any
command here to work.

The allow-list applies identically in both cases -- a group chat id has to be in `ALLOWED_CHATS`
(negative, see section 2) exactly the same way a DM chat id does.

## 8. Troubleshooting

**Invalid or revoked token.** `register_commands` (`bin/bot/src/worker.rs`) calls Telegram's
`setMyCommands` before the polling loop ever starts, so a bad token fails at startup, not on the
first message. Telegram's API returns `401 Unauthorized` for a bad token; that surfaces wrapped as
`Registering bot commands: ...` with Telegram's own response underneath. Since every worker in
this binary's `JoinSet` exiting brings the whole process down (`bin/bot/src/main.rs`'s comment on
`MetricsWorker`), the bot process exits rather than limping along partially configured.

**Chat not on the allow-list.** The chat gets exactly one line back, whatever it sent:
`this chat is not on the allow-list, so nothing here answers to it.` The bot's own log records
`Refused a message from an unlisted chat` at `warn` with the chat id, which is the fastest way to
confirm you have the sign wrong on a group id (see section 2) -- compare the logged id against
what is actually in `ALLOWED_CHATS`.

**Every command returns empty, but the bot answers.** This is the expected state before `scorer`
has produced anything, not a bug -- see each command's empty-state text in section 6
(`no ranked pools for this timeframe yet.`, `nothing clears the gate right now. ...`, `no
evaluation on record for this pool. ...`, and so on). Confirm with `/status`: `watched pools: 0`
and `no ingest health rows recorded yet.` both mean nothing has flowed through the pipeline yet.
Separately, if migrations were never applied (`make migrate`, or `indexer`/`scorer` never started),
every command instead fails with `that command failed unexpectedly; nothing was applied. Check the
logs.` -- check the bot's own log output for the underlying Postgres error rather than assuming
the tables are just empty.

**Rate limiting.** Telegram allows roughly one message per second per chat. `bin/bot/src/worker.rs`
enforces a 1050ms minimum gap between sends to the same chat (`PER_CHAT_MIN_GAP`,
`bin/bot/src/ratelimit.rs`'s `wait_for`) -- a burst of commands, or a single reply that paginates
into several messages, is spaced out automatically rather than hitting Telegram's own limit and
erroring. There is no user-visible error for this; replies just arrive slightly slower than the
commands that triggered them.

**Long replies.** Telegram caps a single message at 4096 characters
(`bin/bot/src/render/paginate.rs`'s `MESSAGE_LIMIT`). A reply that would exceed it -- most often a
`/why` with a long rationale trail -- is split on line boundaries into multiple messages rather
than truncated; every page after the first carries a `(continued i/n)` footer. No content is ever
dropped to make something fit in one message.
