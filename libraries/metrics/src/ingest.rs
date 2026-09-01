use std::sync::LazyLock;

use prometheus::{Gauge, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, Opts};

use crate::registry::register;

pub static PROCESSING_SLOT: LazyLock<Gauge> = LazyLock::new(|| {
    register(Gauge::new("processing_slot", "Slot currently being processed").unwrap())
});

pub static RPC_CALL_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register(
        IntCounterVec::new(Opts::new("rpc_call_total", "Total RPC calls made"), &["method", "status"]).unwrap(),
    )
});

pub static RPC_CALL_DURATION_SECS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register(
        HistogramVec::new(
            HistogramOpts::new("rpc_call_duration_secs", "RPC call duration in seconds"),
            &["method"],
        )
        .unwrap(),
    )
});

pub static STREAM_RECONNECT_TOTAL: LazyLock<IntCounter> = LazyLock::new(|| {
    register(IntCounter::new("stream_reconnect_total", "Total stream reconnect attempts").unwrap())
});

pub static INGEST_LAG_SLOTS: LazyLock<Gauge> =
    LazyLock::new(|| register(Gauge::new("ingest_lag_slots", "Slots behind the chain tip").unwrap()));

pub static DECODE_ERROR_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register(IntCounterVec::new(Opts::new("decode_error_total", "Total event decode failures"), &["event_type"]).unwrap())
});
