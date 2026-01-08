use std::sync::LazyLock;

use prometheus::{Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder};
use tracing::warn;

pub static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

pub static RECORD_COUNT: LazyLock<IntGauge> = LazyLock::new(|| {
    IntGauge::new(
        "proxyd_record_count",
        "Total number of IP records in database",
    )
    .unwrap()
});

pub static LAST_SYNC_TIMESTAMP: LazyLock<IntGauge> = LazyLock::new(|| {
    IntGauge::new(
        "proxyd_last_sync_timestamp",
        "Unix timestamp of last successful sync",
    )
    .unwrap()
});

pub static SYNC_SUCCESS: LazyLock<IntCounter> = LazyLock::new(|| {
    IntCounter::new(
        "proxyd_sync_success_total",
        "Total number of successful syncs",
    )
    .unwrap()
});

pub static SYNC_FAILURES: LazyLock<IntCounter> = LazyLock::new(|| {
    IntCounter::new("proxyd_sync_failures_total", "Total number of failed syncs").unwrap()
});

pub static LOOKUP_REQUESTS: LazyLock<IntCounter> = LazyLock::new(|| {
    IntCounter::new(
        "proxyd_lookup_requests_total",
        "Total number of lookup requests",
    )
    .unwrap()
});

pub static LOOKUP_HITS: LazyLock<IntCounter> = LazyLock::new(|| {
    IntCounter::new("proxyd_lookup_hits_total", "Total number of lookup hits").unwrap()
});

pub static LOOKUP_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    Histogram::with_opts(
        HistogramOpts::new("proxyd_lookup_latency_seconds", "Lookup latency in seconds").buckets(
            vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0],
        ),
    )
    .unwrap()
});

pub static GRPC_REQUESTS: LazyLock<IntCounter> = LazyLock::new(|| {
    IntCounter::new(
        "proxyd_grpc_requests_total",
        "Total number of gRPC requests",
    )
    .unwrap()
});

pub static REST_REQUESTS: LazyLock<IntCounter> = LazyLock::new(|| {
    IntCounter::new(
        "proxyd_rest_requests_total",
        "Total number of REST requests",
    )
    .unwrap()
});

fn register_metric<T: prometheus::core::Collector + Clone + 'static>(metric: &T, name: &str) {
    if let Err(e) = REGISTRY.register(Box::new(metric.clone())) {
        warn!("Failed to register metric {}: {}", name, e);
    }
}

pub fn register_metrics() {
    register_metric(&*RECORD_COUNT, "record_count");
    register_metric(&*LAST_SYNC_TIMESTAMP, "last_sync_timestamp");
    register_metric(&*SYNC_SUCCESS, "sync_success");
    register_metric(&*SYNC_FAILURES, "sync_failures");
    register_metric(&*LOOKUP_REQUESTS, "lookup_requests");
    register_metric(&*LOOKUP_HITS, "lookup_hits");
    register_metric(&*LOOKUP_LATENCY, "lookup_latency");
    register_metric(&*GRPC_REQUESTS, "grpc_requests");
    register_metric(&*REST_REQUESTS, "rest_requests");
}

pub fn gather_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    encoder
        .encode_to_string(&metric_families)
        .unwrap_or_default()
}
