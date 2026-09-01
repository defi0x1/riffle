use std::sync::LazyLock;

use prometheus::{Registry, core::Collector};

// Single process-wide registry. Subsystem modules (e.g. `ingest`) register their own
// metrics into it lazily, on first access of the metric itself.
pub static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

// Registers a collector into the shared registry and hands it back, so a metric can be
// defined and registered in one expression inside a `LazyLock::new` closure.
pub fn register<C: Collector + Clone + 'static>(collector: C) -> C {
    REGISTRY
        .register(Box::new(collector.clone()))
        .expect("metric registration cannot fail for a name defined once");
    collector
}
