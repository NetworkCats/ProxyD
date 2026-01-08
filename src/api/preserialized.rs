use actix_web::body::BoxBody;
use actix_web::http::header::ContentType;
use actix_web::http::StatusCode;
use actix_web::{HttpResponse, Responder};
use once_cell::sync::Lazy;

pub struct PreserializedJson {
    body: &'static [u8],
    status: StatusCode,
}

impl PreserializedJson {
    pub const fn new(body: &'static [u8], status: StatusCode) -> Self {
        Self { body, status }
    }

    pub const fn ok(body: &'static [u8]) -> Self {
        Self::new(body, StatusCode::OK)
    }

    pub const fn service_unavailable(body: &'static [u8]) -> Self {
        Self::new(body, StatusCode::SERVICE_UNAVAILABLE)
    }

    pub const fn bad_request(body: &'static [u8]) -> Self {
        Self::new(body, StatusCode::BAD_REQUEST)
    }
}

impl Responder for PreserializedJson {
    type Body = BoxBody;

    fn respond_to(self, _req: &actix_web::HttpRequest) -> HttpResponse<Self::Body> {
        self.into_response()
    }
}

impl PreserializedJson {
    fn into_response(self) -> HttpResponse<BoxBody> {
        HttpResponse::build(self.status)
            .content_type(ContentType::json())
            .body(self.body.to_vec())
    }
}

impl From<PreserializedJson> for HttpResponse {
    fn from(value: PreserializedJson) -> Self {
        value.into_response()
    }
}

pub static HEALTH_OK: Lazy<&'static [u8]> = Lazy::new(|| {
    Box::leak(
        serde_json::json!({
            "status": "healthy",
            "database_healthy": true
        })
        .to_string()
        .into_bytes()
        .into_boxed_slice(),
    )
});

pub static HEALTH_UNAVAILABLE: Lazy<&'static [u8]> = Lazy::new(|| {
    Box::leak(
        serde_json::json!({
            "status": "unhealthy",
            "database_healthy": false
        })
        .to_string()
        .into_bytes()
        .into_boxed_slice(),
    )
});

pub static BATCH_SIZE_ERROR: Lazy<&'static [u8]> = Lazy::new(|| {
    Box::leak(
        serde_json::json!({
            "error": "Batch size exceeds maximum of 1000"
        })
        .to_string()
        .into_bytes()
        .into_boxed_slice(),
    )
});

pub fn health_response(db_healthy: bool) -> PreserializedJson {
    if db_healthy {
        PreserializedJson::ok(*HEALTH_OK)
    } else {
        PreserializedJson::service_unavailable(*HEALTH_UNAVAILABLE)
    }
}

pub fn batch_size_error() -> PreserializedJson {
    PreserializedJson::bad_request(*BATCH_SIZE_ERROR)
}
