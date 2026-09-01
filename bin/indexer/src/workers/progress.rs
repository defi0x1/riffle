use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// Shared between the ingestion workers and the health worker. Cheap to update from any
/// worker's tick and read from the health worker's, without a lock -- nothing here needs
/// cross-field consistency, each is meaningful on its own.
#[derive(Default)]
pub struct Progress {
    last_slot: AtomicI64,
    // On-chain unix time of the most recent successful write, used to derive a
    // backend-agnostic staleness figure without a live "current slot" call.
    last_block_time: AtomicI64,
    rows_written_total: AtomicU64,
    decode_errors_total: AtomicU64,
}

impl Progress {
    pub fn record_write(&self, slot: u64, block_time: i64, rows: u64) {
        self.last_slot.store(slot as i64, Ordering::Relaxed);
        self.last_block_time.store(block_time, Ordering::Relaxed);
        self.rows_written_total.fetch_add(rows, Ordering::Relaxed);
    }

    pub fn record_decode_error(&self) {
        self.decode_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn last_slot(&self) -> i64 {
        self.last_slot.load(Ordering::Relaxed)
    }

    pub fn last_block_time(&self) -> i64 {
        self.last_block_time.load(Ordering::Relaxed)
    }

    pub fn rows_written_total(&self) -> u64 {
        self.rows_written_total.load(Ordering::Relaxed)
    }

    pub fn take_decode_errors(&self) -> u64 {
        self.decode_errors_total.swap(0, Ordering::Relaxed)
    }
}
