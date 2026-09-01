use std::time::Duration;

use backon::{BackoffBuilder, ExponentialBuilder};
use eyre::WrapErr;
use futures::{Sink, SinkExt, Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::time::Instant;
use yellowstone_grpc_client::{GeyserGrpcClient, Interceptor};
use yellowstone_grpc_proto::prelude::{
    SubscribeRequest, SubscribeRequestPing, SubscribeUpdate, subscribe_update::UpdateOneof,
};

use metrics::STREAM_RECONNECT_TOTAL;

use crate::GeyserConfig;

/// One slot observed further ahead than expected. Geyser gives no ordering or completeness
/// guarantee, so this is the only way a dropped notification becomes visible instead of
/// silently vanishing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotGap {
    pub from: u64,
    pub to: u64,
}

impl SlotGap {
    pub fn missed(&self) -> u64 {
        self.to - self.from - 1
    }
}

#[derive(Default)]
pub struct SlotTracker {
    high_water_mark: Option<u64>,
}

impl SlotTracker {
    pub fn observe(&mut self, slot: u64) -> Option<SlotGap> {
        let gap = match self.high_water_mark {
            Some(hwm) if slot > hwm + 1 => Some(SlotGap {
                from: hwm,
                to: slot,
            }),
            _ => None,
        };
        if self.high_water_mark.is_none_or(|hwm| slot > hwm) {
            self.high_water_mark = Some(slot);
        }
        gap
    }

    pub fn high_water_mark(&self) -> Option<u64> {
        self.high_water_mark
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LivenessConfig {
    pub ping_interval: Duration,
    pub ping_send_timeout: Duration,
    pub pong_timeout: Duration,
}

impl Default for LivenessConfig {
    fn default() -> Self {
        Self {
            ping_interval: Duration::from_secs(5),
            ping_send_timeout: Duration::from_secs(2),
            pong_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ReconnectPolicy {
    pub min_delay: Duration,
    pub max_delay: Duration,
    pub max_attempts: usize,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            min_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            // Reconnecting forever hides an outage. After this many attempts the loop gives
            // up and ends the stream, so a wrapping supervisor sees the task exit and
            // restarts the process rather than an indexer quietly stuck offline for days.
            max_attempts: 240,
        }
    }
}

impl ReconnectPolicy {
    fn backoff(&self) -> impl Iterator<Item = Duration> + Send + Sync + Unpin + use<> {
        ExponentialBuilder::default()
            .with_min_delay(self.min_delay)
            .with_max_delay(self.max_delay)
            .with_jitter()
            .with_max_times(self.max_attempts)
            .build()
    }
}

pub(super) enum StreamError {
    DownstreamClosed,
    Failed(String),
}

pub(super) struct RunOutcome {
    pub last_slot: Option<u64>,
    pub result: Result<(), StreamError>,
}

/// Drives one already-subscribed stream: two independent timers in one `select!`. A short
/// one sends a ping up the sink (itself timed out, so a wedged sink can't block the loop
/// forever); a longer one is reset only by an inbound ping or pong and, if it fires, the
/// stream is declared dead rather than left to hang silently -- the actual Geyser failure
/// mode, which looks nothing like a disconnect.
///
/// Generic over the sink/stream so this can run against fakes in tests without a live
/// connection; the real caller passes the tonic-backed pair from `subscribe_with_request`.
pub(super) async fn run_once<Tx, Rx, RxErr>(
    mut sink: Tx,
    mut stream: Rx,
    cfg: LivenessConfig,
    tx_out: &mpsc::Sender<SubscribeUpdate>,
) -> RunOutcome
where
    Tx: Sink<SubscribeRequest> + Unpin,
    Tx::Error: std::fmt::Display,
    Rx: Stream<Item = Result<SubscribeUpdate, RxErr>> + Unpin,
    RxErr: std::fmt::Display,
{
    let mut slots = SlotTracker::default();

    let mut ping_ticker = tokio::time::interval(cfg.ping_interval);
    ping_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping_ticker.tick().await; // interval fires immediately; consume so the first ping is one interval out

    let dead_at = tokio::time::sleep(cfg.pong_timeout);
    tokio::pin!(dead_at);
    let mut ping_id: i32 = 0;

    loop {
        tokio::select! {
            _ = ping_ticker.tick() => {
                ping_id = ping_id.wrapping_add(1);
                let req = SubscribeRequest {
                    ping: Some(SubscribeRequestPing { id: ping_id }),
                    ..Default::default()
                };
                match tokio::time::timeout(cfg.ping_send_timeout, sink.send(req)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        return RunOutcome {
                            last_slot: slots.high_water_mark(),
                            result: Err(StreamError::Failed(format!("Sending Geyser ping: {e}"))),
                        };
                    }
                    Err(_) => {
                        return RunOutcome {
                            last_slot: slots.high_water_mark(),
                            result: Err(StreamError::Failed(format!(
                                "Sending Geyser ping timed out after {:?}; sink appears wedged",
                                cfg.ping_send_timeout
                            ))),
                        };
                    }
                }
            }
            () = &mut dead_at => {
                return RunOutcome {
                    last_slot: slots.high_water_mark(),
                    result: Err(StreamError::Failed(format!(
                        "No ping/pong activity within {:?}; stream considered dead",
                        cfg.pong_timeout
                    ))),
                };
            }
            item = stream.next() => {
                match item {
                    Some(Ok(update)) => {
                        match &update.update_oneof {
                            Some(UpdateOneof::Ping(_)) | Some(UpdateOneof::Pong(_)) => {
                                dead_at.as_mut().reset(Instant::now() + cfg.pong_timeout);
                            }
                            Some(UpdateOneof::Slot(s)) => {
                                if let Some(gap) = slots.observe(s.slot) {
                                    tracing::warn!(
                                        from = gap.from,
                                        to = gap.to,
                                        missed = gap.missed(),
                                        "Geyser slot gap detected"
                                    );
                                    STREAM_RECONNECT_TOTAL.inc();
                                }
                            }
                            _ => {
                                if tx_out.send(update).await.is_err() {
                                    return RunOutcome {
                                        last_slot: slots.high_water_mark(),
                                        result: Err(StreamError::DownstreamClosed),
                                    };
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        return RunOutcome {
                            last_slot: slots.high_water_mark(),
                            result: Err(StreamError::Failed(format!("Geyser stream error: {e}"))),
                        };
                    }
                    None => {
                        return RunOutcome {
                            last_slot: slots.high_water_mark(),
                            result: Err(StreamError::Failed("Geyser stream closed by server".to_string())),
                        };
                    }
                }
            }
        }
    }
}

pub(super) struct ConnectionConfig {
    pub endpoint: String,
    pub x_token: Option<String>,
    pub liveness: LivenessConfig,
}

impl ConnectionConfig {
    pub(super) fn new(config: &GeyserConfig) -> eyre::Result<Self> {
        let endpoint = config
            .geyser_endpoint
            .clone()
            .ok_or_else(|| eyre::eyre!("geyser_endpoint is required when backend=geyser"))?;
        Ok(Self {
            endpoint,
            x_token: config.geyser_x_token.clone(),
            liveness: LivenessConfig::default(),
        })
    }
}

pub(super) async fn connect(
    cfg: &ConnectionConfig,
) -> eyre::Result<GeyserGrpcClient<impl Interceptor>> {
    GeyserGrpcClient::build_from_shared(cfg.endpoint.clone())
        .wrap_err_with(|| "Building Geyser client")?
        .x_token(cfg.x_token.clone())
        .wrap_err_with(|| "Setting Geyser x-token")?
        .connect()
        .await
        .wrap_err_with(|| "Connecting to Geyser endpoint")
}

/// Reconnect loop with slot replay: each attempt asks `build_request` for a fresh
/// subscription seeded with the highest slot seen so far, so a reconnect replays the gap
/// instead of losing it. Exponential backoff with jitter and a hard attempt cap -- past the
/// cap this simply returns, ending the stream, rather than retrying forever and hiding an
/// outage from whatever is meant to notice the process died.
pub(super) async fn run_resilient(
    cfg: ConnectionConfig,
    policy: ReconnectPolicy,
    mut build_request: impl FnMut(Option<u64>) -> SubscribeRequest,
    tx_out: mpsc::Sender<SubscribeUpdate>,
) {
    let mut last_slot: Option<u64> = None;
    let mut backoff = policy.backoff();

    loop {
        let outcome = match connect(&cfg).await {
            Ok(mut client) => {
                let request = build_request(last_slot);
                match client.subscribe_with_request(Some(request)).await {
                    Ok((sink, stream)) => run_once(sink, stream, cfg.liveness, &tx_out).await,
                    Err(e) => RunOutcome {
                        last_slot: None,
                        result: Err(StreamError::Failed(format!(
                            "Subscribing to Geyser stream: {e}"
                        ))),
                    },
                }
            }
            Err(e) => RunOutcome {
                last_slot: None,
                result: Err(StreamError::Failed(format!("{e:#}"))),
            },
        };

        if let Some(slot) = outcome.last_slot {
            last_slot = Some(slot);
        }

        match outcome.result {
            Ok(()) => return,
            Err(StreamError::DownstreamClosed) => return,
            Err(StreamError::Failed(msg)) => {
                STREAM_RECONNECT_TOTAL.inc();
                match backoff.next() {
                    Some(delay) => {
                        tracing::warn!(error = %msg, delay = ?delay, resume_from_slot = ?last_slot, "Geyser stream failed, reconnecting");
                        tokio::time::sleep(delay).await;
                    }
                    None => {
                        tracing::error!(error = %msg, "Exhausted Geyser reconnect attempts, giving up");
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::channel::mpsc as fmpsc;
    use yellowstone_grpc_proto::prelude::{
        SubscribeUpdateAccount, SubscribeUpdateAccountInfo, SubscribeUpdatePong,
        SubscribeUpdateSlot,
    };

    use super::*;

    // --- SlotTracker / gap detection -------------------------------------------------

    #[test]
    fn test_no_gap_on_first_slot_observed() {
        let mut t = SlotTracker::default();
        assert_eq!(t.observe(100), None);
        assert_eq!(t.high_water_mark(), Some(100));
    }

    #[test]
    fn test_no_gap_on_consecutive_slots() {
        let mut t = SlotTracker::default();
        t.observe(100);
        assert_eq!(t.observe(101), None);
        assert_eq!(t.high_water_mark(), Some(101));
    }

    #[test]
    fn test_gap_detected_when_a_slot_is_skipped() {
        let mut t = SlotTracker::default();
        t.observe(100);
        let gap = t.observe(105).expect("gap expected");
        assert_eq!(gap, SlotGap { from: 100, to: 105 });
        assert_eq!(gap.missed(), 4);
        assert_eq!(t.high_water_mark(), Some(105));
    }

    #[test]
    fn test_out_of_order_slot_does_not_report_a_gap_or_regress_hwm() {
        let mut t = SlotTracker::default();
        t.observe(100);
        t.observe(110);
        // a late, older slot arrives after a newer one was already seen
        assert_eq!(t.observe(105), None);
        assert_eq!(t.high_water_mark(), Some(110));
    }

    #[test]
    fn test_duplicate_slot_is_a_noop() {
        let mut t = SlotTracker::default();
        t.observe(100);
        assert_eq!(t.observe(100), None);
        assert_eq!(t.high_water_mark(), Some(100));
    }

    // --- ReconnectPolicy backoff cap ---------------------------------------------------

    #[test]
    fn test_backoff_stops_after_max_attempts() {
        let policy = ReconnectPolicy {
            min_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(10),
            max_attempts: 5,
        };
        let delays: Vec<Duration> = policy.backoff().collect();
        assert_eq!(delays.len(), 5);
    }

    #[test]
    fn test_backoff_delays_stay_bounded_by_max_delay_even_with_jitter() {
        // jitter is added on top of the capped delay (up to the delay's own value again),
        // so the bound to check is 2x max_delay, not max_delay itself
        let policy = ReconnectPolicy {
            min_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(50),
            max_attempts: 20,
        };
        for delay in policy.backoff() {
            assert!(
                delay <= Duration::from_millis(100),
                "delay {delay:?} grew unbounded"
            );
        }
    }

    // --- liveness timer arithmetic, without any live stream -----------------------------

    fn account_update(slot: u64) -> SubscribeUpdate {
        SubscribeUpdate {
            filters: vec![],
            update_oneof: Some(UpdateOneof::Account(SubscribeUpdateAccount {
                account: Some(SubscribeUpdateAccountInfo {
                    pubkey: vec![1; 32],
                    lamports: 0,
                    owner: vec![2; 32],
                    executable: false,
                    rent_epoch: 0,
                    data: vec![],
                    write_version: 0,
                    txn_signature: None,
                }),
                slot,
                is_startup: false,
            })),
            created_at: None,
        }
    }

    fn pong_update(id: i32) -> SubscribeUpdate {
        SubscribeUpdate {
            filters: vec![],
            update_oneof: Some(UpdateOneof::Pong(SubscribeUpdatePong { id })),
            created_at: None,
        }
    }

    fn slot_update(slot: u64) -> SubscribeUpdate {
        SubscribeUpdate {
            filters: vec![],
            update_oneof: Some(UpdateOneof::Slot(SubscribeUpdateSlot {
                slot,
                parent: None,
                status: 0,
                dead_error: None,
            })),
            created_at: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_ping_is_sent_at_the_configured_interval() {
        let (req_tx, mut req_rx) = fmpsc::unbounded::<SubscribeRequest>();
        let (_upd_tx, upd_rx) = fmpsc::unbounded::<Result<SubscribeUpdate, String>>();
        let (out_tx, _out_rx) = mpsc::channel(8);

        let cfg = LivenessConfig {
            ping_interval: Duration::from_secs(5),
            ping_send_timeout: Duration::from_secs(2),
            pong_timeout: Duration::from_secs(600),
        };

        let handle = tokio::spawn(async move { run_once(req_tx, upd_rx, cfg, &out_tx).await });

        tokio::time::advance(Duration::from_secs(5)).await;
        let first = req_rx.next().await.expect("expected a ping request");
        assert!(first.ping.is_some());

        tokio::time::advance(Duration::from_secs(5)).await;
        let second = req_rx.next().await.expect("expected a second ping request");
        assert!(second.ping.is_some());
        assert_ne!(first.ping.unwrap().id, second.ping.unwrap().id);

        handle.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn test_dead_timer_fires_without_any_pong() {
        let (req_tx, _req_rx) = fmpsc::unbounded::<SubscribeRequest>();
        let (_upd_tx, upd_rx) = fmpsc::unbounded::<Result<SubscribeUpdate, String>>();
        let (out_tx, _out_rx) = mpsc::channel(8);

        let cfg = LivenessConfig {
            ping_interval: Duration::from_secs(1_000_000), // never fires within the test
            ping_send_timeout: Duration::from_secs(2),
            pong_timeout: Duration::from_secs(30),
        };

        let handle = tokio::spawn(async move { run_once(req_tx, upd_rx, cfg, &out_tx).await });

        tokio::time::advance(Duration::from_secs(31)).await;
        let outcome = handle.await.unwrap();
        match outcome.result {
            Err(StreamError::Failed(msg)) => assert!(msg.contains("dead")),
            _ => panic!("expected the stream to be declared dead"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_pong_resets_the_dead_timer_and_keeps_the_stream_alive() {
        let (req_tx, _req_rx) = fmpsc::unbounded::<SubscribeRequest>();
        let (mut upd_tx, upd_rx) = fmpsc::unbounded::<Result<SubscribeUpdate, String>>();
        let (out_tx, _out_rx) = mpsc::channel(8);

        let cfg = LivenessConfig {
            ping_interval: Duration::from_secs(1_000_000),
            ping_send_timeout: Duration::from_secs(2),
            pong_timeout: Duration::from_secs(30),
        };

        let handle = tokio::spawn(async move { run_once(req_tx, upd_rx, cfg, &out_tx).await });

        // without a reset this would exceed the 30s pong_timeout at the second advance
        for i in 0..3 {
            tokio::time::advance(Duration::from_secs(20)).await;
            upd_tx.send(Ok(pong_update(i))).await.unwrap();
            // let run_once observe the pong and reset its timer
            tokio::time::sleep(Duration::from_millis(0)).await;
        }

        // now close the stream to end the test deterministically, distinguishing
        // "closed by server" from "declared dead"
        drop(upd_tx);
        let outcome = handle.await.unwrap();
        match outcome.result {
            Err(StreamError::Failed(msg)) => assert!(msg.contains("closed by server")),
            other => panic!("expected a clean stream closure, got {other:?}"),
        }
    }

    impl std::fmt::Debug for StreamError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                StreamError::DownstreamClosed => write!(f, "DownstreamClosed"),
                StreamError::Failed(m) => write!(f, "Failed({m})"),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_slot_updates_do_not_reach_the_downstream_channel() {
        let (req_tx, _req_rx) = fmpsc::unbounded::<SubscribeRequest>();
        let (mut upd_tx, upd_rx) = fmpsc::unbounded::<Result<SubscribeUpdate, String>>();
        let (out_tx, mut out_rx) = mpsc::channel(8);

        let cfg = LivenessConfig {
            ping_interval: Duration::from_secs(1_000_000),
            ping_send_timeout: Duration::from_secs(2),
            pong_timeout: Duration::from_secs(30),
        };

        let _handle = tokio::spawn(async move { run_once(req_tx, upd_rx, cfg, &out_tx).await });

        upd_tx.send(Ok(slot_update(1))).await.unwrap();
        upd_tx.send(Ok(account_update(1))).await.unwrap();

        let forwarded = out_rx
            .recv()
            .await
            .expect("expected the account update forwarded");
        assert!(matches!(
            forwarded.update_oneof,
            Some(UpdateOneof::Account(_))
        ));
    }
}
