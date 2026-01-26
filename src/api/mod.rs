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
        metrics::inc_rest_requests();
        Self {
            start: Instant::now(),
        }
    }

    pub fn start_grpc() -> Self {
        metrics::inc_grpc_requests();
        Self {
            start: Instant::now(),
        }
    }

    pub fn record(&self, result: &LookupResult) {
        let elapsed = self.start.elapsed().as_secs_f64();
        metrics::record_lookup_latency(elapsed);
        if result.found {
            metrics::inc_lookup_hits();
        }
    }

    pub fn record_batch(&self, any_found: bool) {
        let elapsed = self.start.elapsed().as_secs_f64();
        metrics::record_lookup_latency(elapsed);
        if any_found {
            metrics::inc_lookup_hits();
        }
    }
}
