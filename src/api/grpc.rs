use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::LookupMetrics;

const MAX_BATCH_SIZE: usize = 1000;
use crate::db::Database;
use crate::ip::{
    lookup_ip as do_lookup_ip, lookup_ips_batch, lookup_range as do_lookup_range,
    lookup_ranges_batch, LookupError, LookupResult, MatchedEntry as DomainMatchedEntry,
    ReputationFlags as DomainFlags,
};

pub mod proto {
    #![allow(
        clippy::struct_excessive_bools,
        clippy::doc_markdown,
        clippy::default_trait_access,
        clippy::too_many_lines
    )]
    tonic::include_proto!("proxyd");
}

use proto::proxy_d_server::{ProxyD, ProxyDServer};
use proto::{
    BatchIpRequest, BatchRangeRequest, BatchReputationResponse, IpRequest,
    MatchedEntry as ProtoMatchedEntry, RangeRequest, ReputationFlags as ProtoFlags,
    ReputationResponse,
};

pub struct ProxyDService {
    db: Arc<Database>,
}

impl ProxyDService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn into_server(self) -> ProxyDServer<Self> {
        ProxyDServer::new(self)
    }
}

fn domain_flags_to_proto(flags: &DomainFlags) -> ProtoFlags {
    ProtoFlags {
        anonblock: flags.anonblock,
        proxy: flags.proxy,
        vpn: flags.vpn,
        cdn: flags.cdn,
        public_wifi: flags.public_wifi,
        rangeblock: flags.rangeblock,
        school_block: flags.school_block,
        tor: flags.tor,
        webhost: flags.webhost,
    }
}

fn domain_entry_to_proto(entry: DomainMatchedEntry) -> ProtoMatchedEntry {
    ProtoMatchedEntry {
        entry: entry.entry,
        flags: Some(domain_flags_to_proto(&entry.flags)),
    }
}

fn result_to_proto(result: LookupResult) -> ReputationResponse {
    ReputationResponse {
        found: result.found,
        query: result.query,
        flags: Some(domain_flags_to_proto(&result.flags)),
        matched_entries: result
            .matched_entries
            .into_iter()
            .map(domain_entry_to_proto)
            .collect(),
    }
}

fn lookup_error_to_status(err: &LookupError) -> Status {
    match err {
        LookupError::InvalidIp(_) | LookupError::InvalidCidr(_) => {
            Status::invalid_argument(err.to_string())
        }
        LookupError::Database(_) => Status::internal(err.to_string()),
    }
}

#[tonic::async_trait]
impl ProxyD for ProxyDService {
    async fn lookup_ip(
        &self,
        request: Request<IpRequest>,
    ) -> Result<Response<ReputationResponse>, Status> {
        let metrics = LookupMetrics::start_grpc();
        let ip_str = &request.get_ref().ip;

        match do_lookup_ip(&self.db, ip_str) {
            Ok(result) => {
                metrics.record(&result);
                Ok(Response::new(result_to_proto(result)))
            }
            Err(ref e) => Err(lookup_error_to_status(e)),
        }
    }

    async fn lookup_range(
        &self,
        request: Request<RangeRequest>,
    ) -> Result<Response<ReputationResponse>, Status> {
        let metrics = LookupMetrics::start_grpc();
        let cidr_str = &request.get_ref().cidr;

        match do_lookup_range(&self.db, cidr_str) {
            Ok(result) => {
                metrics.record(&result);
                Ok(Response::new(result_to_proto(result)))
            }
            Err(ref e) => Err(lookup_error_to_status(e)),
        }
    }

    async fn batch_lookup_ip(
        &self,
        request: Request<BatchIpRequest>,
    ) -> Result<Response<BatchReputationResponse>, Status> {
        let ips = &request.get_ref().ips;

        if ips.len() > MAX_BATCH_SIZE {
            return Err(Status::invalid_argument(format!(
                "Batch size exceeds maximum of {MAX_BATCH_SIZE}"
            )));
        }

        let metrics = LookupMetrics::start_grpc();
        let ip_strs: Vec<&str> = ips.iter().map(String::as_str).collect();

        match lookup_ips_batch(&self.db, &ip_strs) {
            Ok(lookup_results) => {
                let any_found = lookup_results.iter().any(|r| r.found);
                let results: Vec<ReputationResponse> =
                    lookup_results.into_iter().map(result_to_proto).collect();
                metrics.record_batch(any_found);
                Ok(Response::new(BatchReputationResponse { results }))
            }
            Err(ref e) => Err(lookup_error_to_status(e)),
        }
    }

    async fn batch_lookup_range(
        &self,
        request: Request<BatchRangeRequest>,
    ) -> Result<Response<BatchReputationResponse>, Status> {
        let cidrs = &request.get_ref().cidrs;

        if cidrs.len() > MAX_BATCH_SIZE {
            return Err(Status::invalid_argument(format!(
                "Batch size exceeds maximum of {MAX_BATCH_SIZE}"
            )));
        }

        let metrics = LookupMetrics::start_grpc();
        let cidr_strs: Vec<&str> = cidrs.iter().map(String::as_str).collect();

        match lookup_ranges_batch(&self.db, &cidr_strs) {
            Ok(lookup_results) => {
                let any_found = lookup_results.iter().any(|r| r.found);
                let results: Vec<ReputationResponse> =
                    lookup_results.into_iter().map(result_to_proto).collect();
                metrics.record_batch(any_found);
                Ok(Response::new(BatchReputationResponse { results }))
            }
            Err(ref e) => Err(lookup_error_to_status(e)),
        }
    }
}
