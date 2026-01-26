use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

const LOOKUP_LATENCY_BUCKETS: &[f64] = &[
    0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0,
];

const SYNC_DURATION_BUCKETS: &[f64] = &[1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0];

pub fn init_metrics() -> &'static PrometheusHandle {
    PROMETHEUS_HANDLE.get_or_init(|| {
        let handle = PrometheusBuilder::new()
            .set_buckets_for_metric(
                Matcher::Full("proxyd_lookup_latency_seconds".to_string()),
                LOOKUP_LATENCY_BUCKETS,
            )
            .expect("failed to set lookup latency buckets")
            .set_buckets_for_metric(
                Matcher::Full("proxyd_sync_duration_seconds".to_string()),
                SYNC_DURATION_BUCKETS,
            )
            .expect("failed to set sync duration buckets")
            .install_recorder()
            .expect("failed to install Prometheus recorder");

        register_metric_descriptions();
        set_build_info();

        handle
    })
}

fn register_metric_descriptions() {
    describe_gauge!("proxyd_build_info", "Build information with version label");
    describe_gauge!("proxyd_up", "Service health status (1 = healthy, 0 = unhealthy)");
    describe_gauge!("proxyd_record_count", "Current number of IP records in database");
    describe_gauge!(
        "proxyd_last_sync_timestamp",
        "Unix timestamp of the last successful sync"
    );
    describe_counter!("proxyd_sync_success_total", "Total number of successful syncs");
    describe_counter!("proxyd_sync_failures_total", "Total number of failed syncs");
    describe_counter!("proxyd_lookup_hits_total", "Total number of lookup hits");
    describe_counter!("proxyd_grpc_requests_total", "Total number of gRPC requests");
    describe_counter!("proxyd_rest_requests_total", "Total number of REST requests");
    describe_histogram!(
        "proxyd_lookup_latency_seconds",
        "Lookup request latency in seconds"
    );
    describe_histogram!(
        "proxyd_sync_duration_seconds",
        "Sync operation duration in seconds"
    );
}

fn set_build_info() {
    gauge!("proxyd_build_info", "version" => env!("CARGO_PKG_VERSION")).set(1.0);
}

pub fn set_record_count(count: i64) {
    gauge!("proxyd_record_count").set(count as f64);
}

pub fn set_last_sync_timestamp(timestamp: i64) {
    gauge!("proxyd_last_sync_timestamp").set(timestamp as f64);
}

pub fn inc_sync_success() {
    counter!("proxyd_sync_success_total").increment(1);
}

pub fn inc_sync_failures() {
    counter!("proxyd_sync_failures_total").increment(1);
}

pub fn set_health_status(healthy: bool) {
    gauge!("proxyd_up").set(if healthy { 1.0 } else { 0.0 });
}

pub fn record_sync_duration(seconds: f64) {
    histogram!("proxyd_sync_duration_seconds").record(seconds);
}

pub fn inc_lookup_hits() {
    counter!("proxyd_lookup_hits_total").increment(1);
}

pub fn record_lookup_latency(seconds: f64) {
    histogram!("proxyd_lookup_latency_seconds").record(seconds);
}

pub fn inc_grpc_requests() {
    counter!("proxyd_grpc_requests_total").increment(1);
}

pub fn inc_rest_requests() {
    counter!("proxyd_rest_requests_total").increment(1);
}

pub fn gather_metrics() -> String {
    PROMETHEUS_HANDLE
        .get()
        .map(|h| h.render())
        .unwrap_or_default()
}
