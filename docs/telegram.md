# Telegram bot

A walkthrough for taking the `bot` binary from nothing to answering commands in a real chat.
Everything here was read out of `bin/bot/src/` -- `main.rs`, `cli.rs`, `config.rs`, `auth.rs`,
`handlers.rs`, `render/`, `worker.rs`, `ratelimit.rs`, `mute.rs`, `secret_guard.rs`, `shape.rs` --
and cross-checked against `config/bot.example.yaml`, `.env.example`, and `miniapp/README.md`. The
full flag reference for every binary, including `bot`, lives in
[`docs/configuration.md`](configuration.md); this document is only about the Telegram-facing
parts.

Commands split into two tiers, and the split matters more than any individual command's syntax:

- **Read-only commands** -- `/top`, `/volume`, `/potential`, `/pool`, `/why`, `/watch`, `/mute`,
  `/status`, `/wallet`, `/balance`, `/positions`, `/profit` -- answer entirely in the chat. Most
  render rows `scorer` already computed and persisted; `/watch`, `/mute`, and `/wallet` write a
  small operator/user decision (force tier-1 membership, suppress a signal, register a public
  key), never a computed ranking value, and none of them ever moves funds.
- **Fund-moving commands** -- `/open`, `/add`, `/remove`, `/claim`, `/close` -- never move funds
  either, by construction. Each one only ever *proposes*: it checks the caller owns what they are
  acting on, applies the same risk gate `/potential` applies, renders exactly what would happen,
  and hands back a button that deep-links into the Telegram Mini App. Approval and signing happen
  there, on the user's own device -- the chat cannot sign anything, there is no signing-capable
  type anywhere in this crate (enforced by `scripts/keyless-guard.sh`, see
  [`docs/security.md`](security.md)), and the bot never learns whether the button was even
  tapped. The user reviews the proposal in the Mini App, signs or declines, and is sent back to
  the chat afterwards. This bot never has, and by design never can have, a user's private key.

Read [`docs/security.md`](security.md) before running this for real -- in particular, never paste
a private key or recovery phrase into this chat, or into any chat, ever; see
["Never paste a key"](#never-paste-a-key) below for what happens if you do anyway.

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
| `--miniapp-base-url` | `MINIAPP_BASE_URL` | none | yes |
| `--max-add-value-usd` | `MAX_ADD_VALUE_USD` | `5000` | no |

`--allowed-chats` / `ALLOWED_CHATS` takes a comma-separated list, e.g.
`123456789,-987654321`. An empty value is accepted by `clap` but authorizes no one -- the bot
starts and runs, and every chat gets the refusal message; this is a silent no-op, not a startup
error, so an accidentally-empty value is easy to miss.

`--max-rows` / `MAX_ROWS` caps how many rows `/top`, `/volume` and `/potential` render before the
reply would need to paginate (default 10; see `bin/bot/src/config.rs`'s `defaults::MAX_ROWS`).

`--miniapp-base-url` / `MINIAPP_BASE_URL` is the base URL for the Mini App's direct link, e.g.
`https://t.me/FeeFarmBot/app` (set it up via `@BotFather` -> your bot -> Bot Settings -> Menu
Button, or `/newapp`). Every fund-moving command appends `?startapp=<action>_<...>` to this URL
and puts it on the button under its proposal -- it is the only place any of those commands ever
points a user, since the chat itself can never sign anything. There is no fallback: without this
set, the bot refuses to start rather than proposing something with nowhere to send the user to
sign it.

`--max-add-value-usd` / `MAX_ADD_VALUE_USD` (default `5000`) is an advisory per-`/add` cap in USD
(`bin/bot/src/config.rs`'s `defaults::max_add_value_usd`). It is only enforced when the target
position already carries a priced valuation to estimate against; an `/add` above the cap is
refused outright, not silently clamped. This is advisory, the same way every backend-side cap is
advisory in a keyless design (see [`docs/security.md`](security.md) and `miniapp/README.md`'s
"Per-user notional caps" section) -- it stops an accidental fat-fingered amount from this chat,
it does not stop a modified client from building the same transaction some other way.

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

Either export the required variables directly:

```sh
export DATABASE_URL=postgres://feefarm:feefarm@localhost:5432/feefarm
export BOT_TOKEN=123456789:AAExampleExampleExampleExampleExampl
export ALLOWED_CHATS=123456789,-987654321
export MINIAPP_BASE_URL=https://t.me/FeeFarmBot/app
cargo run --bin bot
```

or copy `config/bot.example.yaml` to `config/bot.yaml` (gitignored -- see `.gitignore`), fill in
the real values, and point `--config` at it:

```yaml
database_url: postgres://feefarm:feefarm@localhost:5432/feefarm
bot_token: "123456789:AAExampleExampleExampleExampleExampl"
allowed_chats: "123456789,-987654321"
miniapp_base_url: "https://t.me/FeeFarmBot/app"
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
wallet - Register your public key, or list your wallets
balance - Latest token balances for a registered wallet
positions - Open positions for a registered wallet
profit - Profit for one of your positions
open - Propose opening a new position (signed in the Mini App)
add - Propose adding liquidity (signed in the Mini App)
remove - Propose removing liquidity (signed in the Mini App)
claim - Propose claiming fees (signed in the Mini App)
close - Propose closing a position (signed in the Mini App)
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
4. **The Mini App, deployed and registered with BotFather.** `/open`, `/add`, `/remove`,
   `/claim`, and `/close` all end with a button pointing at `miniapp_base_url`; that button is
   only usable if something real is actually deployed and registered at that URL. Building and
   running the Mini App itself is a separate codebase and toolchain -- see `miniapp/README.md`,
   in particular its "Local development against Telegram" and "Deployment" sections. Nothing
   about the bot itself checks this at startup; a proposal renders and a button appears either
   way, it just opens nothing useful if the Mini App is not actually there yet.

Then start the bot itself:

```sh
cargo run --bin bot -- --config config/bot.yaml
```

or with `--release` / a built binary, same as any other binary in this workspace. There is no
separate startup flag for polling vs. webhooks -- `worker.rs` always uses long polling
(`teloxide::update_listeners::polling_default`).

## 6. Walkthrough: registering a wallet and farming a position

This is the same sequence of commands end to end, using placeholder addresses throughout --
`POOL_ADDR`, `WALLET_PUBKEY`, and `POSITION_ADDR` stand in for real base58 Solana addresses. At
every step where a button appears, tapping it opens the Mini App, where the actual review and
signature happen; nothing here moves anything by itself.

1. **Register a wallet.** Generate or import a wallet in the Mini App (open it from the bot's
   menu button, or from any command's button, the first time), then register its public key here:
   ```
   /wallet WALLET_PUBKEY main
   ```
   The bot never sees, asks for, or accepts anything but the public key -- see
   ["Never paste a key"](#never-paste-a-key) below.
2. **Find a pool worth farming.**
   ```
   /potential 1h
   ```
   Gate-filtered to quality-A pools clearing breakeven `r_org`. Pick one from the list, or check
   a specific candidate with `/pool POOL_ADDR` or `/why POOL_ADDR` first.
3. **Open a position.**
   ```
   /open POOL_ADDR 20
   ```
   Proposes a 20-bin-wide range centered on the pool's most recently observed active bin (see
   the `/open` reference below for why it takes a width instead of a bin range). Tap the
   button, review the range and the strategy in the Mini App, sign. The new position is created
   empty.
4. **Deposit into it.**
   ```
   /add POSITION_ADDR 10 250
   ```
   Proposes depositing `10` of token X and `250` of token Y (whatever the pool's pair actually
   is -- `/pool` or the proposal itself names them). Refused if the pool no longer clears the
   gate, or if the estimated USD value is over the configured cap.
5. **Watch it.** `/positions` lists every open position for a wallet; `/profit POSITION_ADDR`
   shows deposited-vs-withdrawn-plus-current-value at any time, no gate or ownership friction
   beyond the position being yours.
6. **Claim fees as they accrue.**
   ```
   /claim POSITION_ADDR
   ```
7. **Close it out.**
   ```
   /close POSITION_ADDR
   ```
   Proposes withdrawing everything and closing the position. `/remove POSITION_ADDR <percent>`
   is the partial version of the same thing, for taking some liquidity out without closing.

## 7. Commands

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

### Never paste a key

Before the wallet and position commands: `/wallet` only ever accepts a public key. It never needs,
and will never ask for, a private key, a seed phrase, a recovery phrase, or a passphrase --
those exist only inside the Mini App, on your own device (see `miniapp/README.md`'s "Custody
model"). Do not paste one into this chat, or into any chat, on the assumption that this is how a
wallet gets "connected" -- it is not, for this bot or for any legitimate one.

If you paste something shaped like key material anyway -- a raw base58 secret key, a Solana CLI
on-disk key file (`[12,34,...]`), or a 12/15/18/21/24-word recovery phrase -- `bin/bot/src/
secret_guard.rs` catches it on the raw message text, before `clap` ever tokenizes it, and the bot
refuses:

```
refused -- this looked like a private key or seed phrase

/wallet only ever accepts a public key; signing happens on your own device inside the Mini App,
and this bot and its backend never see, store, or log a private key. I have not stored or logged
what you just sent, but Telegram has already kept a copy of it in this chat's history -- delete
that message now, and treat whatever key or phrase it contained as compromised: abandon it, and
create or import a new wallet in the Mini App instead.
```

The detection is structural (length, word count, character shape -- `bin/bot/src/shape.rs`), not
an attempt to decode or validate the value as a real key, and it never echoes the offending text
back. But the refusal message says the important part plainly: the paste itself already happened,
Telegram already has it in this chat's history, and that key must be treated as burned -- the
bot's refusal to store or log it does not undo that.

### `/wallet [pubkey] [label]`

Registers a Solana public key against your Telegram identity, or lists what you already have
registered.

With no arguments, lists your registered wallets:

```
Your wallets

7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU -- main, registered 2026-08-15
9yLMzk4DX88e18TYKTEqcE6kCliuUsB94UAVSiJotBtV -- (no label), registered 2026-08-20
```

Empty: `no wallet registered yet. Send /wallet <pubkey> with the public key shown in the Mini App.`

With a `pubkey` (and an optional `label`), registers it:

- Not base58, or not 32-44 characters: `that does not look like a Solana public key (expected a
  base58 string, 32-44 characters). If you meant to paste a private key or seed phrase, stop --
  /wallet never needs one; register the public key shown in the Mini App instead.`
- New registration: `7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU registered to your account.`
- Already yours (re-registering, e.g. to change the label): `7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU is already yours -- label refreshed.`
- Registered to a different Telegram account: `7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU is
  already registered to a different Telegram account. Its owner has to revoke it before it can be
  registered here.` A pubkey belongs to exactly one Telegram account at a time -- the bot cannot
  verify who actually controls the corresponding key, so it refuses to reassign ownership rather
  than guessing.

`pubkey` and `label` are both plain positional arguments -- a third word is rejected by `clap`
before the handler ever runs.

Needs a Telegram user identity on the message (see below); a channel post or some anonymous-admin
group messages do not carry one: `this command needs your Telegram user identity, which was not
present on this message (channel posts and some anonymous-admin messages do not carry one) --
send it as yourself in a normal message.`

### `/balance [wallet]`

Latest token balances for a registered wallet. `wallet` is optional and resolves to yours
automatically if you have exactly one registered wallet; with zero or more than one, it is
required.

```
Balances for 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU

BXk7hwx4X6vGncdU4V6wSKFwtP7yYVjEyGHwvA7dKtmA  12.5  1834.20
5q4kNQTgqW3zWNjJzXTvSGr6L1GvBqcaXGr5x4WQz1wc  500  500.00
```

Each row is `mint  amount  value_usd`. No balances on record yet (they refresh on a fixed poll
cadence once a wallet is registered): `no balances on record yet -- they refresh on a fixed poll
cadence once a wallet is registered.`

Refusals:

- No wallet registered at all: `no wallet registered yet. Send /wallet <pubkey> with the public
  key shown in the Mini App.`
- `wallet` given but not registered to you: `<wallet> is not registered to you. Register it first
  with /wallet, or use one of your own registered wallets.`
- No `wallet` given and you have more than one registered:
  ```
  you have more than one registered wallet -- say which one:

  7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
  9yLMzk4DX88e18TYKTEqcE6kCliuUsB94UAVSiJotBtV

  e.g. /balance <pubkey>
  ```

### `/positions [wallet]`

Open positions for a registered wallet. Same optional `wallet` resolution and the same three
refusal cases as `/balance` (with the example in the "say which one" case reading `/positions
<pubkey>` instead).

```
Open positions

Position
3aQXvBTQ2Z8HneCzKvV1FskV3hjJyzE9j5qmSybVoWjR
SOL/USDC
bin range 991 - 1010
opened 2026-08-20 14:03
```

No open positions for that wallet: `no open positions for this wallet.`

### `/profit <position>`

Deposits-vs-withdrawals-plus-current-value for one of your positions, at any time (not just when
closing it).

```
Position
3aQXvBTQ2Z8HneCzKvV1FskV3hjJyzE9j5qmSybVoWjR
SOL/USDC
bin range 991 - 1010

deposited 1000  withdrawn/claimed 120  current value 950
profit (USD, realized + unrealized): 70
vs. holding: -30 (value now vs. simply holding the deposited tokens)

as of 2026-08-31 09:00:00: price 148.20 / 1.00  value 950  in range yes
```

`vs. holding` only appears when a hold-value baseline is on record for the position; profit
itself is always shown, computed as `withdrawn/claimed + current value - deposited`. If no mark
has ever been taken for the position, the valuation line reads `no live valuation on record yet
for this position -- current price and value will be shown in the Mini App before you confirm.`
instead of a price/value row.

Refusals: no such position (`no position found at <address>`), or a position that belongs to a
wallet not registered to you (`<address> does not belong to a wallet registered to you.`).

### Fund-moving commands

`/open`, `/add`, `/remove`, `/claim`, and `/close` share the same shape: check ownership, apply
the risk gate where relevant, render exactly what would happen, and end with a button that opens
the Mini App to review and sign. Every proposal below ends with the same notice
(`bin/bot/src/render/mod.rs`'s `miniapp_notice`):

```
this chat cannot sign anything. Tap the button below to review and sign this in the Mini App --
nothing moves until you approve it there, and you will be sent back here once it confirms.
```

The bot never learns whether the button was tapped, let alone what happened after -- the Mini App
and its own backend own everything from that point on. Every command in this group needs your
Telegram user identity the same way `/wallet` does, and refuses with the same message if it is
missing (see above).

### `/open <address> <width>`

Proposes opening a new position in pool `address`, sized by `width` (an integer bin count, `1`
to `70`) rather than an explicit bin range. This is deliberate: a width plus the pool address is
everything a row from `/potential` already gives you -- there is no bin id to read off a chart or
copy by hand, and the range is centered on the pool's own most recently observed active bin
instead of one you would otherwise have to guess.

The centering arithmetic (`bin/bot/src/handlers.rs`): `lower = active_bin - (width - 1) / 2`,
`upper = lower + width - 1`, both using integer division. For an odd width the range is exactly
symmetric around the active bin. For an even width it is not quite symmetric -- one more bin
lands above the active bin than below it (e.g. `width 20` centered on active bin `1000` gives the
range `991 - 1010`: 9 bins below, the active bin itself, 10 bins above). This only matters at the
margin; it is not a reason to prefer odd widths.

```
Open position
SOL/USDC
7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
bin range 991 - 1010 (width 20)
price range 0.048852 - 0.051842 (token_y per token_x, raw pool units -- not decimal-adjusted or priced in USD)
strategy SpotBalanced

a new position account will be created for this range -- signing this opens it empty; deposit
into it afterward with /add.

this chat cannot sign anything. Tap the button below to review and sign this in the Mini App --
nothing moves until you approve it there, and you will be sent back here once it confirms.
```

The button is labeled **Open position**.

Refusals, checked in this order:

- No Telegram user identity: see above.
- No wallet registered: `no wallet registered yet. Send /wallet <pubkey> with the public key
  shown in the Mini App.` (`/open` has no existing position to infer a wallet from, unlike
  `/add`/`/remove`/`/claim`/`/close`, so it checks registration directly instead.)
- Pool not found: `no pool found at <address>.`
- Pool does not currently clear the risk gate -- the same gate `/potential` applies, since
  opening a brand new position is at least as consequential as adding to one that already
  cleared it:
  ```
  refused -- this pool does not currently clear the risk gate
  7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU

  latest evaluation: GATE_FAIL, timeframe 1h

  FAIL vol_tvl: observed 0.90 >= threshold 1.00 note: below floor for V1
  ```
  or, if no evaluation exists on record for the pool yet: `no evaluation on record for this pool
  yet, so it cannot be verified against the gate -- try again once scoring has run for it.`
- No active-bin snapshot on record yet -- there is nothing honest to center a range on:
  `no active-bin reading on record yet for <address> -- opening a position needs to know where to
  center the range, and none has arrived from ingestion yet. Try again shortly, or check the pool
  with /pool first.` This is expected for a pool `indexer` has only just started watching; it is
  not an error to retry against.
- The computed range falls outside what the pool's bin step can price (only possible near the
  edge of what that bin step can represent at all): `refused -- bin range is not valid for this
  pool` followed by the pool address and `the range [<lower>, <upper>] falls outside what this
  pool's bin step can price -- try a narrower width.`

### `/add <position> <amount_x> <amount_y>`

Proposes depositing `amount_x` of the pool's token X and `amount_y` of its token Y into an
existing open position (both decimal amounts, in the token's own units, not lamports/raw).

```
Position
3aQXvBTQ2Z8HneCzKvV1FskV3hjJyzE9j5qmSybVoWjR
SOL/USDC
bin range 991 - 1010

proposed: add 10 / 250
strategy SpotBalanced

as of 2026-08-31 09:00:00: price 148.20 / 1.00  value 950  in range yes

this chat cannot sign anything. Tap the button below to review and sign this in the Mini App --
nothing moves until you approve it there, and you will be sent back here once it confirms.
```

The button is labeled **Add liquidity**.

Refusals:

- Position not found, not yours, or already closed (shared with `/remove`/`/claim`/`/close`):
  `no position found at <address>`; `<address> does not belong to a wallet registered to you.`;
  `<address> is already closed -- there is nothing left to act on.`
- Either amount is zero or negative: `both amounts must be greater than zero.`
- The pool no longer clears the risk gate (same check and same message shape as `/open`'s gate
  refusal above).
- Estimated value over the configured cap (only checked when the position already has a priced
  valuation to estimate against -- see `--max-add-value-usd` in section 3): `refused -- over the
  per-transaction cap` followed by the position address and `estimated value <est> exceeds the
  configured cap <cap> -- split this into smaller adds, or ask an operator to raise the configured
  cap.` Without a priced valuation on record, the cap cannot be checked and the proposal proceeds,
  saying so plainly in the valuation line rather than silently skipping the check.

### `/remove <position> <percent>`

Proposes withdrawing `percent` (an integer, `1` to `100`) of a position's liquidity, without
closing it.

```
Position
3aQXvBTQ2Z8HneCzKvV1FskV3hjJyzE9j5qmSybVoWjR
SOL/USDC
bin range 991 - 1010

proposed: withdraw 50% of this position

as of 2026-08-31 09:00:00: price 148.20 / 1.00  value 950  in range yes

this chat cannot sign anything. Tap the button below to review and sign this in the Mini App --
nothing moves until you approve it there, and you will be sent back here once it confirms.
```

The button is labeled **Remove liquidity**. Not gated the same way `/open`/`/add` are -- removing
liquidity reduces exposure rather than growing it. Refusals: the same position not-found /
not-owned / already-closed cases as `/add`.

### `/claim <position>`

Proposes claiming accrued fees on a position.

```
Position
3aQXvBTQ2Z8HneCzKvV1FskV3hjJyzE9j5qmSybVoWjR
SOL/USDC
bin range 991 - 1010

proposed: claim accrued fees

uncollected fees: 0.42 / 3.10

as of 2026-08-31 09:00:00: price 148.20 / 1.00  value 950  in range yes

this chat cannot sign anything. Tap the button below to review and sign this in the Mini App --
nothing moves until you approve it there, and you will be sent back here once it confirms.
```

The uncollected-fees line only appears when a valuation is on record. The button is labeled
**Claim fees**. Not gated. Refusals: the same position not-found / not-owned / already-closed
cases as `/add`.

### `/close <position>`

Proposes withdrawing everything and closing a position entirely.

```
Position
3aQXvBTQ2Z8HneCzKvV1FskV3hjJyzE9j5qmSybVoWjR
SOL/USDC
bin range 991 - 1010

proposed: withdraw everything and close this position

as of 2026-08-31 09:00:00: price 148.20 / 1.00  value 950  in range yes

this chat cannot sign anything. Tap the button below to review and sign this in the Mini App --
nothing moves until you approve it there, and you will be sent back here once it confirms.
```

The button is labeled **Close position**. Not gated. Refusals: the same position not-found /
not-owned / already-closed cases as `/add`.

## 8. Group versus direct use

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

## 9. Troubleshooting

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
has produced anything, not a bug -- see each command's empty-state text in section 7
(`no ranked pools for this timeframe yet.`, `nothing clears the gate right now. ...`, `no
evaluation on record for this pool. ...`, and so on). Confirm with `/status`: `watched pools: 0`
and `no ingest health rows recorded yet.` both mean nothing has flowed through the pipeline yet.
Separately, if migrations were never applied (`make migrate`, or `indexer`/`scorer` never started),
every command instead fails with `that command failed unexpectedly; nothing was applied. Check the
logs.` -- check the bot's own log output for the underlying Postgres error rather than assuming
the tables are just empty.

**No wallet registered.** `/balance`, `/positions`, and `/open` (and, indirectly, `/add`,
`/remove`, `/claim`, `/close` -- they resolve a wallet through the position they act on) all
refuse with `no wallet registered yet. Send /wallet <pubkey> with the public key shown in the
Mini App.` Register the public key the Mini App shows for the wallet you created or imported
there -- never a private key or phrase, see ["Never paste a key"](#never-paste-a-key).

**Wallet or position not yours.** `<address> is not registered to you. ...` (for a `wallet`
argument someone else registered) or `<address> does not belong to a wallet registered to you.`
(for a `position` argument) means exactly what it says -- ownership is keyed on the Telegram
user id that sent the message, checked against `wallets.telegram_user_id`, not on anything the
caller can assert. If you expected to own it, confirm you registered the same wallet from the
same Telegram account with `/wallet` (no arguments) first.

**Pool fails the risk gate.** `/open` and `/add` both refuse with `refused -- this pool does not
currently clear the risk gate` when the pool's latest signal is not `POTENTIAL` -- the same gate
`/potential` filters on. Run `/why <address>` on the same pool to see which condition failed and
by how much; this is not a bug, it is the same protection `/potential` gives a browsing user
applied to a command that would actually grow exposure.

**No active-bin snapshot for `/open`.** `no active-bin reading on record yet for <address> --
opening a position needs to know where to center the range, and none has arrived from ingestion
yet.` `indexer` has to have observed at least one active-bin reading for that specific pool
before `/open` has anything honest to center a range on -- this is common for a pool that was
only just promoted to watched, or one `indexer` has not subscribed to yet. Retry after a short
wait, or check `/pool <address>` to confirm the pool is known at all first.

**Amount above the configured cap.** `/add` refuses with `refused -- over the per-transaction
cap` when the proposed deposit's estimated USD value exceeds `--max-add-value-usd` (default
`5000`, see section 3) and the position already has a priced valuation to check against. This is
an advisory limit enforced by this chat only, not a property of the transaction itself -- split
the deposit into smaller `/add` calls, or have an operator raise `MAX_ADD_VALUE_USD`. If the
position has no priced valuation yet, the cap cannot be checked at all and the proposal goes
through regardless -- the valuation line says so plainly rather than silently skipping the check.

**Tapped nothing, or tapped the button and nothing happened.** The bot never learns whether a
Mini App button was tapped, let alone what happened afterward (see section 7's "Fund-moving
commands") -- there is no follow-up message, no timeout, and no "did you mean to finish this"
nudge, by design. If a proposal's button was never tapped, nothing was proposed to Solana and
nothing needs cleaning up; just send the command again if you still want to go through with it.
If the button was tapped but the Mini App did not open, confirm `miniapp_base_url` actually
points at a deployed Mini App registered with BotFather (section 5, step 4) -- a proposal renders
correctly even if the URL it links to serves nothing.

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
