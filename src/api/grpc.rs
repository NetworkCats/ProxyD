use std::sync::Arc;
use std::time::Duration;

use tonic::codec::CompressionEncoding;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use tonic_reflection::server::Builder as ReflectionBuilder;

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

    pub const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("proxyd_descriptor");
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
            .accept_compressed(CompressionEncoding::Gzip)
            .accept_compressed(CompressionEncoding::Zstd)
            .send_compressed(CompressionEncoding::Gzip)
            .send_compressed(CompressionEncoding::Zstd)
    }
}

impl From<&DomainFlags> for ProtoFlags {
    fn from(flags: &DomainFlags) -> Self {
        Self {
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
}

impl From<DomainMatchedEntry> for ProtoMatchedEntry {
    fn from(entry: DomainMatchedEntry) -> Self {
        Self {
            entry: entry.entry,
            flags: Some(ProtoFlags::from(&entry.flags)),
        }
    }
}

impl From<LookupResult> for ReputationResponse {
    fn from(result: LookupResult) -> Self {
        let matched_entries: Vec<ProtoMatchedEntry> = result
            .matched_entries
            .into_iter()
            .map(ProtoMatchedEntry::from)
            .collect();

        Self {
            found: result.found,
            query: result.query,
            flags: Some(ProtoFlags::from(&result.flags)),
            matched_entries,
        }
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

pub fn create_reflection_service(
) -> tonic_reflection::server::ServerReflectionServer<impl tonic_reflection::server::ServerReflection>
{
    ReflectionBuilder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("Failed to build reflection service")
}

pub struct GrpcServerConfig {
    pub http2_keepalive_interval: Duration,
    pub http2_keepalive_timeout: Duration,
    pub tcp_keepalive: Duration,
    pub tcp_nodelay: bool,
    pub concurrency_limit: usize,
    pub initial_connection_window_size: u32,
    pub initial_stream_window_size: u32,
}

impl Default for GrpcServerConfig {
    fn default() -> Self {
        Self {
            http2_keepalive_interval: Duration::from_secs(30),
            http2_keepalive_timeout: Duration::from_secs(10),
            tcp_keepalive: Duration::from_secs(60),
            tcp_nodelay: true,
            concurrency_limit: 1000,
            initial_connection_window_size: 4 * 1024 * 1024,
            initial_stream_window_size: 2 * 1024 * 1024,
        }
    }
}

pub fn configure_server(config: &GrpcServerConfig) -> Server {
    Server::builder()
        .http2_keepalive_interval(Some(config.http2_keepalive_interval))
        .http2_keepalive_timeout(Some(config.http2_keepalive_timeout))
        .tcp_keepalive(Some(config.tcp_keepalive))
        .tcp_nodelay(config.tcp_nodelay)
        .concurrency_limit_per_connection(config.concurrency_limit)
        .initial_connection_window_size(config.initial_connection_window_size)
        .initial_stream_window_size(config.initial_stream_window_size)
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
                Ok(Response::new(result.into()))
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
                Ok(Response::new(result.into()))
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
                    lookup_results.into_iter().map(Into::into).collect();
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
                    lookup_results.into_iter().map(Into::into).collect();
                metrics.record_batch(any_found);
                Ok(Response::new(BatchReputationResponse { results }))
            }
            Err(ref e) => Err(lookup_error_to_status(e)),
        }
    }
}
