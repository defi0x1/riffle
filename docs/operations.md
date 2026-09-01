# Operations

A runbook for someone who did not build this system and needs to bring it up, tell whether it is
healthy, and recover it when it is not. Background on *why* the system is shaped this way is in
[`docs/architecture.md`](architecture.md); every flag referenced here is documented in full in
[`docs/configuration.md`](configuration.md).

## Bringing the system up from nothing

```sh
cp .env.example .env               # then edit it -- see below
make up                            # postgres+timescale, prometheus, grafana (docker compose)
make migrate                       # applies migrations/ via sqlx-cli, reads .env automatically
```

`.env` is only read automatically by `sqlx-cli` (`make migrate`). None of the three Rust binaries
read a `.env` file -- they read the real process environment. Export what you need before running
one directly:

```sh
export DATABASE_URL=postgres://feefarm:feefarm@localhost:5432/feefarm
export RPC_URL=https://your-rpc-provider/your-key
cargo run --bin indexer
```

`indexer` and `scorer` both run `storage::run_migrations` themselves at startup, so `make migrate`
is not strictly required before starting them -- either one racing to migrate a fresh database is
safe, they serialise on a Postgres advisory lock. `make migrate` is still worth running explicitly
the first time, so a typo'd `DATABASE_URL` fails loudly on a one-line command instead of inside a
binary's startup log.

### Start order, and why

There is no code-enforced ordering between the three binaries -- each connects to Postgres
independently and none blocks waiting for another. The order below is what makes the first hour
useful rather than confusing:

1. **`indexer`** first. Nothing else has anything to read until pools exist in the `pools` table
   and state has been written at least once. On a cold start its `TierWorker` runs its first tier
   sweep immediately (not after waiting a full `promotion_interval`), so a watch set starts filling
   within seconds of startup, not minutes.
2. **`scorer`** next. Its four workers (rollup, indicators, signals, paper positions) all read
   tables `indexer` writes. Starting it before `indexer` has written anything is harmless -- every
   query returns empty and every tick logs nothing useful -- but there is nothing to see.
3. **`bot`** last. It is read-only and has no dependency on the other two beyond wanting something
   in the database to render; starting it first just means `/top` and friends return an empty
   table until data exists.

**Every binary defaults to metrics port `9101`.** Running more than one on the same host without
overriding `--metrics-port` on at least two of them fails the second and third binary's metrics
server at bind time -- the rest of the process keeps running, only `/metrics` is down for that
binary. `observability/prometheus/prometheus.yml` expects `9101` (indexer), `9102` (scorer), `9103`
(bot); set `--metrics-port`/`METRICS_PORT` to match on each one. Only `indexer` actually populates
its `/metrics` endpoint with anything today -- `scorer` and `bot` bind and serve correctly, but
return an almost-empty body, since neither registers any Prometheus collectors of its own. Scorer's
operational state is visible through the Grafana dashboards' direct Postgres queries instead.

**Connection budget.** Each binary defaults to a 10-connection pool (`--max-connections`). Running
all three against one Postgres instance is 30 connections against whatever `max_connections` that
instance allows (Postgres's own default is 100) -- comfortable at this scale, but worth knowing
before adding a fourth consumer or raising any binary's pool size.

## Telling whether ingestion is healthy

Three ways to check, cheapest first:

1. **`/status` in Telegram** (once `bot` is running and your chat is in `ALLOWED_CHATS`). Shows
   ingest lag per source and current tier size.
2. **The heartbeat log line.** `indexer`'s health worker logs `"Heartbeat: ingest healthy and making
   progress"` at `info` level only when data is both *fresh* (on-chain time within
   `health_freshness_threshold`, default 60s) and *moving* (more rows written than the previous
   tick). This is deliberate: a wedged-but-connected process does not log a false "healthy" line --
   silence is the signal, not a log line nobody is watching. Absence of the heartbeat for more than
   a couple of `health_interval`s (default 30s each) means something is wrong even if the process
   is still running.
3. **Grafana.** Two provisioned dashboards under the `fee-farming` folder:
   - **Ingestion health** -- indexer up/down, wedged detection (`processing_slot` not advancing),
     decode error rate, stream reconnects, ingest lag in slots, RPC call rate and latency
     percentiles.
   - **Pipeline output** -- pools by tier, seconds since the last 5m/10m rollup bucket, signals
     emitted (last 24h, by kind), pools with a fresh indicator row in the last hour.

   The Grafana instance also has a plain Postgres datasource (`Timescale`) alongside Prometheus,
   specifically because pool tier counts, rollup freshness and signal counts live only in
   Timescale -- the indicator/signal/rollup workers export nothing to `/metrics`, so a panel that
   needs that data queries the database directly rather than inventing a metric that does not exist.

## Alerts (`observability/prometheus/alerts.yml`)

All five rules are scoped to `indexer` -- `scorer` and `bot` export nothing for Prometheus to alert
on today; watch them through the Grafana Postgres panels and the heartbeat log line instead.

| Alert | Condition | What it means | What to do |
|---|---|---|---|
| `IndexerDown` | `up{job="indexer"} == 0` for 2m | Prometheus cannot scrape `/metrics` at all. | Check whether the process is still running -- it may have panicked (see "one worker exits, the process dies" below) or been killed. Check the last log lines before it went quiet. |
| `IndexerWedgedNoProgress` | up, but `processing_slot` unchanged for 10m+ | The wedged-process case: `/metrics` still answers, so a plain liveness probe calls this healthy, but a worker task has hung -- a stream stopped reading, a write is stuck, a lock is held forever. | Restart the indexer. Check the last log line from each worker (`state`, `event`, `discovery`, `tier`) to see which one stopped mid-tick before restarting blind. |
| `IndexerIngestLagGrowing` | `ingest_lag_slots > 750` for 5m (roughly 5 minutes behind, at ~0.4s/slot) | The indexer is falling behind the chain tip but still moving. | Check RPC/Geyser call latency and error rates; confirm the source backend is actually reachable. A source that has silently stopped delivering updates shows as lag that only grows, never plateaus. |
| `IndexerIngestLagCritical` | `ingest_lag_slots > 3000` for 5m (roughly 20 minutes behind) | Active incident: state and event data being written now is stale enough to affect tiering and scoring decisions downstream. | Restart the source connection or the process if RPC/Geyser health looks fine but lag keeps climbing regardless. |
| `IndexerStreamReconnectingRepeatedly` | `stream_reconnect_total` up more than 5 in 15m | Geyser-only. A single reconnect is normal; a steady stream of them points at an unstable endpoint or an auth/token problem upstream. | Check the Geyser endpoint's own status page and the token's expiry before assuming it's a local network issue. |
| `IndexerDecodeFailureRateRising` | `decode_error_total` rate > ~6/min for a given `event_type`, sustained 10m | An on-chain account or event layout changed and `dlmm_decode` no longer matches it, or a specific pool is producing state the decoder was not built to handle. | Check the paired error logs (they carry the pool address) before rolling anything back. If this follows a known DLMM program upgrade, the golden tests in `libraries/dlmm_decode` are the first thing to re-run against a fresh account fetch. |

Two things the health worker tracks that are **not** wired up as Prometheus metrics: write latency
(`ingest_health.write_latency_ms` is always written `NULL`) and rows-written throughput (kept
in-process only). There is deliberately no "write latency degrading" alert, because there is
nothing for it to read yet.

## Restart semantics

**Every restart is safe; none of them retroactively fill a gap.**

- `indexer`: `DiscoveryWorker`, `StateWorker` and `EventWorker` all resume from "now" -- they poll
  or subscribe to current state, they do not replay history. Any swap or liquidity event that
  happened on the Geyser backend while the process was down is not fetched by `indexer` itself on
  restart. The one exception inside `indexer` is Geyser's own reconnect-with-replay inside a single
  stream session (it resumes from the last observed slot on a mid-session reconnect) -- that does
  not cover a full process restart. Separately, `crawler` (a distinct, manually-run binary, not
  restarted automatically with `indexer`) can recover the swap/liquidity-event half of a gap after
  the fact; see "A backfill that needs restarting" below for what it can and cannot recover.
- `scorer`: `regime_state` and `volatility_state` persist per pool/timeframe, so a restart does not
  reset the regime classifier's hysteresis clock or the volatility EWMAs to zero. `SignalsWorker`'s
  cooldown persists too -- it is read back from the `signals` table itself
  (`storage::queries::last_signal_broadcast`) rather than kept in memory, so a restart does not
  cause a still-true condition to be re-announced early. Paper positions re-derive "already open"
  from the database on every tick, so they need no cursor to resume correctly.
- `bot`: stateless beyond mutes (persisted in `muted_pools`) and rate-limit bookkeeping (in-memory,
  resets on restart -- harmless, it only delays the very next message per chat by up to ~1s).

**One worker exiting brings the whole binary down.** `common::run_workers` spawns every worker into
one `JoinSet`, waits for the first to finish for *any* reason, and cancels the rest. A tick that
returns `Err` is caught inside that worker's own loop, logged, and retried next interval -- normal
operation, not a crash. A worker whose `run()` itself returns `Err`, or panics, ends the process.
There is no per-worker supervision or automatic restart; process supervision (systemd, a container
restart policy, or equivalent) is what brings the binary back, and it comes back cleanly per the
restart semantics above.

## Failure modes

### The stream is falling behind (`IndexerIngestLagGrowing` / `Critical`)

`ingest_lag_slots` is derived from on-chain block time, not a live "current slot" RPC call, so it
reflects how stale the *last successfully written* row is. Check, in order:

1. RPC/Geyser call latency and error rate on the relevant Grafana panel.
2. Whether `poll_interval_state` (RPC) is set tighter than the provider can sustain -- a provider
   silently rate-limiting looks identical to lag that only grows.
3. On Geyser, `stream_reconnect_total` -- repeated reconnects and growing lag together point at the
   endpoint, not the code.

If the backend itself looks fine and lag keeps climbing regardless, restart the process. `indexer`
itself does not backfill the gap this creates -- it only ever writes forward from "now" -- but
`crawler` can recover the swap/liquidity-event half of it after the fact; see "A backfill that
needs restarting" below.

### A wedged process that still answers health checks (`IndexerWedgedNoProgress`)

`up{job="indexer"} == 1` only proves the metrics HTTP server's event loop is alive, not that the
worker tasks are making progress -- that is exactly the gap this alert exists to close. Do not treat
"the port is open" as "the process is fine." Check `processing_slot` directly, check which worker's
log line was last seen before the process went quiet, and restart. There is no live diagnostic
short of a restart today -- no `SIGUSR1`-style dump, no admin endpoint.

### Postgres running out of disk under retention pressure

Retention and compression are baked into `migrations/0022_retention_and_compression.sql` as
TimescaleDB policies, not runtime configuration -- there is no flag on any binary that changes them,
and changing a retention window means a new migration, not an environment variable. Roughly:
short-lived raw detail (bin-level state) is kept for the shortest window and compressed soonest;
mid-detail raw tables (swaps, liquidity events, pool/bin snapshots) hold longer; the durable record
-- rollups, indicators, signals, outcomes -- is kept indefinitely and only ever compressed, never
dropped. Compression runs on a schedule per table (most tables: after the table's own retention
consideration, on the order of days to a month); retention drops whole chunks past their window.

Because these policies run on Timescale's own background jobs, not application code, **nothing
in this codebase alerts on disk usage** -- there is no metric for it. Monitor the Postgres data
volume directly (host disk usage, or `SELECT pg_database_size(current_database());`) rather than
waiting for a query to start failing. If disk is filling faster than retention drops it:

- Confirm the compression and retention jobs are actually running:
  `SELECT * FROM timescaledb_information.jobs WHERE proc_name IN ('policy_retention',
  'policy_compression');` alongside `timescaledb_information.job_stats` for last-run status.
  A job that has been failing silently is a real way for disk to fill unexpectedly.
- The uncompressed, unretained table most likely to grow unexpectedly is whichever raw table the
  active source backend feeds fastest -- `swaps` and `bin_states` on a busy Geyser deployment, since
  both are windowed to the shortest retention specifically because their volume is the least
  predictable in advance.
- There is no config knob to shrink a window without a migration; if the box is at genuine risk,
  the fastest safe lever is to stop the binary writing the fastest-growing table rather than try to
  tune anything live.

### A backfill that needs restarting

Three different things can mean "backfill" here, and they fail (and recover) differently:

- **A swap/liquidity-event gap on the raw layer** (`indexer` was down, or the Geyser stream dropped
  transactions for a window). This is what `crawler` exists for -- it is a separate, one-shot binary
  (`cargo run --bin crawler`, not started by `indexer` or `scorer`), run by hand against the
  affected pools and range:
  ```sh
  cargo run --bin crawler -- \
    --pools <affected pool address(es), comma-separated, or --pools-file pools.txt> \
    --from-time 2026-08-30T00:00:00Z --to-time 2026-08-30T06:00:00Z
  ```
  It walks `getSignaturesForAddress` backward per pool, decodes each in-range transaction's
  swap/liquidity events, and writes them into the same `swaps`/`liquidity_events` tables `indexer`
  writes, on the same `(pool_address, ts, signature, ix_index)` uniqueness -- so it is safe to
  re-run or to overlap with what `indexer` already wrote; nothing is double-counted. It checkpoints
  to a JSON file (`--checkpoint-file`, default `crawler_checkpoint.json`) after every page, so an
  interrupted backfill resumes near where it stopped rather than re-walking from the start. Pass
  `--dry-run` first against a large or expensive range to see what it would fetch without touching
  Postgres. See [`docs/configuration.md`](configuration.md#crawler) for every flag.

  What it cannot do: recover **state** -- `pool_snapshots`, `dlmm_pool_state`, `active_bin_snapshots`,
  `bin_states`. A signature walk only ever sees what a transaction changed, not a full account
  snapshot at a past slot, and reconstructing one would need an archival RPC node. A gap in pool
  *state* during an outage (as opposed to swap/liquidity flow) has no recovery path today.

- **A rollup gap** (`scorer` was down, or fell behind). `RollupWorker` only ever builds the
  **current** bucket on each tick -- there is no code path that walks back over buckets missed while
  the process was stopped, and running `crawler` to fill in the underlying `swaps`/`liquidity_events`
  rows does **not** retroactively fix this: `RollupWorker` never re-visits a past bucket, so a
  `pool_metrics_5m`/`10m` gap stays a gap even after the raw data behind it exists again. It also
  propagates forward into `pool_metrics_1h`/`4h`/`24h`, since those three are continuous aggregates
  chained off `10m`, not off raw data -- Timescale's refresh jobs cannot materialise a bucket whose
  source rows were never written. If that window matters, the rollup row has to be built by hand
  (a manual `INSERT` into `pool_metrics_5m`/`10m` built from the same aggregation `RollupWorker`
  itself does, now that `crawler` has backfilled the raw rows it would read) -- there is no existing
  tool that does this automatically.
- **A discovery/state gap** (`indexer` was down). `DiscoveryWorker` and `StateWorker` both resume
  from current chain state on restart, with no historical replay -- a newly-created pool that
  appeared and was later delisted entirely within the outage window would never be seen at all.
  This is rarely worth chasing; the far more common case is simply that state polling picks back up
  and the watch set repopulates within one `promotion_interval` of the restart. `crawler` does not
  help here either, since pool state (as opposed to swap/liquidity flow) is exactly what it cannot
  recover (above).

In every case the fix for "the process was down" is the same -- restart it -- and `crawler` closes
part, not all, of the "I need the missing window's data" gap: the swap/liquidity-event raw layer,
not pool state and not the rollup/indicator layers built on top of it.

## First bring-up checklist

- `DATABASE_URL` set and reachable (`postgres://feefarm:feefarm@localhost:5432/feefarm` against
  `make up`'s default compose stack).
- `RPC_URL` set regardless of which backend you run -- `clap` requires it unconditionally even on
  `--backend geyser`, since the requirement is declared on the RPC config group itself, not
  conditioned on the backend flag.
- If running `--backend geyser`: `GEYSER_ENDPOINT` and `GEYSER_X_TOKEN` also set, or the source
  fails to construct at startup rather than at first stream use.
- `BOT_TOKEN` and `ALLOWED_CHATS` set before starting `bot` -- both are required, and an *empty*
  `ALLOWED_CHATS` is not the same failure as a missing one: the bot starts and answers nothing to
  anyone, silently, rather than refusing to start.
- Distinct `--metrics-port` per binary if running more than one on the same host.
- `.env` and any real `config/*.yaml` stay out of version control (`.gitignore` already covers
  both) -- these hold the four secrets in [`docs/configuration.md`](configuration.md#secrets).
