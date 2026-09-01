//! This binary's own Prometheus metrics, registered into the same process-wide registry
//! `libraries/metrics` exposes via `/metrics` (see `metrics::Config::serve`, reused unmodified
//! by `main.rs`). Defined here rather than in `libraries/metrics` because these are specific to
//! this HTTP surface, the same way `metrics::ingest` holds indexer-specific series in that
//! crate only because indexer's own workers live in several modules that all need them.

use std::sync::LazyLock;

use metrics::register;
use prometheus::{HistogramOpts, HistogramVec, IntCounterVec, Opts};

pub static HTTP_REQUESTS_TOTAL: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register(
        IntCounterVec::new(
            Opts::new("api_http_requests_total", "Total HTTP requests handled"),
            &["method", "path", "status"],
        )
        .unwrap(),
    )
});

pub static HTTP_REQUEST_DURATION_SECS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register(
        HistogramVec::new(
            HistogramOpts::new(
                "api_http_request_duration_secs",
                "HTTP request duration in seconds",
            ),
            &["method", "path"],
        )
        .unwrap(),
    )
});
