pub mod grpc;
pub mod preserialized;
pub mod rest;

use std::time::Instant;

use crate::ip::LookupResult;
use crate::metrics;

pub struct LookupMetrics {
    start: Instant,
}

impl LookupMetrics {
    pub fn start_rest() -> Self {
        metrics::REST_REQUESTS.inc();
        metrics::LOOKUP_REQUESTS.inc();
        Self {
            start: Instant::now(),
        }
    }

    pub fn start_grpc() -> Self {
        metrics::GRPC_REQUESTS.inc();
        metrics::LOOKUP_REQUESTS.inc();
        Self {
            start: Instant::now(),
        }
    }

    pub fn record(&self, result: &LookupResult) {
        let elapsed = self.start.elapsed().as_secs_f64();
        metrics::LOOKUP_LATENCY.observe(elapsed);
        if result.found {
            metrics::LOOKUP_HITS.inc();
        }
    }

    pub fn record_batch(&self, any_found: bool) {
        let elapsed = self.start.elapsed().as_secs_f64();
        metrics::LOOKUP_LATENCY.observe(elapsed);
        if any_found {
            metrics::LOOKUP_HITS.inc();
        }
    }
}
