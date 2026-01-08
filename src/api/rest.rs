use std::sync::Arc;

use actix_web::{get, post, web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};

use super::preserialized::{batch_size_error, health_response};
use super::LookupMetrics;
use crate::db::Database;
use crate::ip::{lookup_ip, lookup_ips_batch, lookup_range, lookup_ranges_batch, LookupError};
use crate::metrics;

const MAX_BATCH_SIZE: usize = 1000;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

impl From<LookupError> for ErrorResponse {
    fn from(err: LookupError) -> Self {
        Self {
            error: err.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct RangeQuery {
    cidr: String,
}

#[derive(Deserialize)]
struct BatchIPRequest {
    ips: Vec<String>,
}

#[derive(Deserialize)]
struct BatchRangeRequest {
    cidrs: Vec<String>,
}

#[get("/health")]
pub async fn health_check(state: web::Data<AppState>) -> impl Responder {
    health_response(state.db.is_healthy())
}

#[get("/metrics")]
pub async fn metrics_endpoint() -> impl Responder {
    let body = metrics::gather_metrics();
    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4; charset=utf-8")
        .body(body)
}

#[get("/v1/ip/{ip}")]
pub async fn get_ip(state: web::Data<AppState>, path: web::Path<String>) -> impl Responder {
    let metrics = LookupMetrics::start_rest();
    let ip_str = path.into_inner();

    match lookup_ip(&state.db, &ip_str) {
        Ok(result) => {
            metrics.record(&result);
            HttpResponse::Ok().json(result)
        }
        Err(e) => HttpResponse::BadRequest().json(ErrorResponse::from(e)),
    }
}

#[get("/v1/range")]
pub async fn get_range(
    state: web::Data<AppState>,
    query: web::Query<RangeQuery>,
) -> impl Responder {
    let metrics = LookupMetrics::start_rest();

    match lookup_range(&state.db, &query.cidr) {
        Ok(result) => {
            metrics.record(&result);
            HttpResponse::Ok().json(result)
        }
        Err(e) => HttpResponse::BadRequest().json(ErrorResponse::from(e)),
    }
}

#[post("/v1/ip/batch")]
pub async fn batch_get_ip(
    state: web::Data<AppState>,
    body: web::Json<BatchIPRequest>,
) -> HttpResponse {
    if body.ips.len() > MAX_BATCH_SIZE {
        return batch_size_error().into();
    }

    let metrics = LookupMetrics::start_rest();
    let ip_strs: Vec<&str> = body.ips.iter().map(String::as_str).collect();

    match lookup_ips_batch(&state.db, &ip_strs) {
        Ok(results) => {
            let any_found = results.iter().any(|r| r.found);
            metrics.record_batch(any_found);
            HttpResponse::Ok().json(results)
        }
        Err(e) => HttpResponse::BadRequest().json(ErrorResponse::from(e)),
    }
}

#[post("/v1/range/batch")]
pub async fn batch_get_range(
    state: web::Data<AppState>,
    body: web::Json<BatchRangeRequest>,
) -> HttpResponse {
    if body.cidrs.len() > MAX_BATCH_SIZE {
        return batch_size_error().into();
    }

    let metrics = LookupMetrics::start_rest();
    let cidr_strs: Vec<&str> = body.cidrs.iter().map(String::as_str).collect();

    match lookup_ranges_batch(&state.db, &cidr_strs) {
        Ok(results) => {
            let any_found = results.iter().any(|r| r.found);
            metrics.record_batch(any_found);
            HttpResponse::Ok().json(results)
        }
        Err(e) => HttpResponse::BadRequest().json(ErrorResponse::from(e)),
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(health_check)
        .service(metrics_endpoint)
        .service(get_ip)
        .service(get_range)
        .service(batch_get_ip)
        .service(batch_get_range);
}
