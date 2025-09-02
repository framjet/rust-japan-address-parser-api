use axum::{
    extract::{Query, Request},
    http::StatusCode,
    middleware::{self, Next},
    response::Json,
    routing::get,
    Router,
};
use japanese_address_parser::parser::{ParseResult, Parser};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::net::TcpListener;
use tokio::signal;
use tokio::time::timeout;
use tower::ServiceBuilder;
use tower_http::{
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{self, EnvFilter};

// Configuration constants
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_REQUEST_SIZE: usize = 1024 * 1024; // 1MB
const MAX_ADDRESS_LENGTH: usize = 500;

// Global metrics
static TOTAL_REQUESTS: AtomicU64 = AtomicU64::new(0);
static SUCCESSFUL_PARSES: AtomicU64 = AtomicU64::new(0);
static FAILED_PARSES: AtomicU64 = AtomicU64::new(0);
static GET_REQUESTS: AtomicU64 = AtomicU64::new(0);
static POST_REQUESTS: AtomicU64 = AtomicU64::new(0);
static TIMEOUT_ERRORS: AtomicU64 = AtomicU64::new(0);
static VALIDATION_ERRORS: AtomicU64 = AtomicU64::new(0);

// Performance metrics
static PARSE_TIME_TOTAL_MS: AtomicU64 = AtomicU64::new(0);
static MIN_PARSE_TIME_MS: AtomicU64 = AtomicU64::new(u64::MAX);
static MAX_PARSE_TIME_MS: AtomicU64 = AtomicU64::new(0);

// Histogram buckets for response time distribution
static PARSE_TIME_BUCKETS: Mutex<[u64; 8]> = Mutex::new([0; 8]); // <1ms, <5ms, <10ms, <25ms, <50ms, <100ms, <500ms, >=500ms

static START_TIME: std::sync::OnceLock<SystemTime> = std::sync::OnceLock::new();

fn update_parse_time_metrics(duration_ms: u64) {
    PARSE_TIME_TOTAL_MS.fetch_add(duration_ms, Ordering::Relaxed);

    // Update min time
    let mut current_min = MIN_PARSE_TIME_MS.load(Ordering::Relaxed);
    while current_min > duration_ms {
        match MIN_PARSE_TIME_MS.compare_exchange_weak(
            current_min,
            duration_ms,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(x) => current_min = x,
        }
    }

    // Update max time
    let mut current_max = MAX_PARSE_TIME_MS.load(Ordering::Relaxed);
    while current_max < duration_ms {
        match MAX_PARSE_TIME_MS.compare_exchange_weak(
            current_max,
            duration_ms,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(x) => current_max = x,
        }
    }

    // Update histogram buckets
    if let Ok(mut buckets) = PARSE_TIME_BUCKETS.lock() {
        let bucket_index = match duration_ms {
            0..=0 => 0,     // <1ms
            1..=4 => 1,     // <5ms
            5..=9 => 2,     // <10ms
            10..=24 => 3,   // <25ms
            25..=49 => 4,   // <50ms
            50..=99 => 5,   // <100ms
            100..=499 => 6, // <500ms
            _ => 7,         // >=500ms
        };
        buckets[bucket_index] += 1;
    }
}

#[derive(Debug, Deserialize)]
struct ParseRequest {
    address: String,
}

#[derive(Debug, Serialize)]
struct ParseResponse {
    success: bool,
    result: Option<ParsedAddress>,
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    processing_time_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ParsedAddress {
    prefecture: Option<String>,
    city: Option<String>,
    town: Option<String>,
    rest: Option<String>,
}

impl From<ParseResult> for ParsedAddress {
    fn from(result: ParseResult) -> Self {
        Self {
            prefecture: Some(result.address.prefecture),
            city: Some(result.address.city),
            town: Some(result.address.town),
            rest: Some(result.address.rest),
        }
    }
}

#[derive(Clone)]
struct AppState {
    parser: Arc<Parser>,
    request_timeout: Duration,
}

impl AppState {
    fn new() -> Self {
        START_TIME.set(SystemTime::now()).ok();
        info!("Initializing Japanese address parser");

        let timeout_secs = std::env::var("REQUEST_TIMEOUT_SECS")
            .unwrap_or_else(|_| DEFAULT_REQUEST_TIMEOUT_SECS.to_string())
            .parse::<u64>()
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT_SECS);

        Self {
            parser: Arc::new(Parser::default()),
            request_timeout: Duration::from_secs(timeout_secs),
        }
    }
}

fn validate_address(address: &str) -> Result<(), String> {
    if address.trim().is_empty() {
        return Err("Address cannot be empty".to_string());
    }

    if address.len() > MAX_ADDRESS_LENGTH {
        return Err(format!(
            "Address too long (max {} characters)",
            MAX_ADDRESS_LENGTH
        ));
    }

    // Basic character validation - ensure it contains some Japanese characters or ASCII
    if !address
        .chars()
        .any(|c| c.is_ascii() || (c as u32 >= 0x3000 && c as u32 <= 0x9FFF))
    {
        return Err("Invalid address format".to_string());
    }

    Ok(())
}

async fn request_logging_middleware(request: Request, next: Next) -> axum::response::Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let start = Instant::now();

    debug!(
        event = "request_started",
        method = %method,
        uri = %uri,
        "Processing request"
    );

    let response = next.run(request).await;
    let duration = start.elapsed();

    debug!(
        event = "request_completed",
        method = %method,
        uri = %uri,
        status = response.status().as_u16(),
        duration_ms = duration.as_millis() as u64,
        "Request completed"
    );

    response
}

async fn parse_address(
    Query(params): Query<HashMap<String, String>>,
    state: axum::extract::State<AppState>,
) -> Result<Json<ParseResponse>, StatusCode> {
    let start_time = Instant::now();
    TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);
    GET_REQUESTS.fetch_add(1, Ordering::Relaxed);

    let address = match params.get("address") {
        Some(addr) => addr.trim(),
        None => {
            FAILED_PARSES.fetch_add(1, Ordering::Relaxed);
            VALIDATION_ERRORS.fetch_add(1, Ordering::Relaxed);
            warn!(
                event = "parse_request_failed",
                reason = "missing_address_parameter",
                method = "GET"
            );
            return Ok(Json(ParseResponse {
                success: false,
                result: None,
                error: Some("Missing 'address' parameter".to_string()),
                processing_time_ms: Some(start_time.elapsed().as_millis() as u64),
            }));
        }
    };

    // Validate address
    if let Err(validation_error) = validate_address(address) {
        FAILED_PARSES.fetch_add(1, Ordering::Relaxed);
        VALIDATION_ERRORS.fetch_add(1, Ordering::Relaxed);
        warn!(
            event = "parse_request_failed",
            reason = "validation_failed",
            method = "GET",
            error = validation_error
        );
        return Ok(Json(ParseResponse {
            success: false,
            result: None,
            error: Some(validation_error),
            processing_time_ms: Some(start_time.elapsed().as_millis() as u64),
        }));
    }

    info!(
        event = "parse_request_started",
        method = "GET",
        address_length = address.len(),
        "Processing address parsing request"
    );

    let parse_start = Instant::now();
    let parse_result = timeout(state.request_timeout, state.parser.parse(address)).await;

    let parsed_result = match parse_result {
        Ok(result) => result,
        Err(_) => {
            FAILED_PARSES.fetch_add(1, Ordering::Relaxed);
            TIMEOUT_ERRORS.fetch_add(1, Ordering::Relaxed);
            error!(
                event = "parse_request_timeout",
                method = "GET",
                address_length = address.len(),
                timeout_secs = state.request_timeout.as_secs(),
                "Request timed out"
            );
            return Ok(Json(ParseResponse {
                success: false,
                result: None,
                error: Some("Request timeout".to_string()),
                processing_time_ms: Some(start_time.elapsed().as_millis() as u64),
            }));
        }
    };

    let parse_duration = parse_start.elapsed();
    let total_duration = start_time.elapsed();

    let parse_time_ms = parse_duration.as_millis() as u64;
    let total_time_ms = total_duration.as_millis() as u64;

    update_parse_time_metrics(parse_time_ms);

    info!(
        event = "parse_request_completed",
        method = "GET",
        success = true,
        address_length = address.len(),
        parse_time_ms = parse_time_ms,
        total_time_ms = total_time_ms,
        "Successfully parsed address"
    );

    SUCCESSFUL_PARSES.fetch_add(1, Ordering::Relaxed);
    Ok(Json(ParseResponse {
        success: true,
        result: Some(parsed_result.into()),
        error: None,
        processing_time_ms: Some(total_time_ms),
    }))
}

async fn parse_address_post(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(payload): Json<ParseRequest>,
) -> Result<Json<ParseResponse>, StatusCode> {
    let start_time = Instant::now();
    TOTAL_REQUESTS.fetch_add(1, Ordering::Relaxed);
    POST_REQUESTS.fetch_add(1, Ordering::Relaxed);

    let address = payload.address.trim();

    // Validate address
    if let Err(validation_error) = validate_address(address) {
        FAILED_PARSES.fetch_add(1, Ordering::Relaxed);
        VALIDATION_ERRORS.fetch_add(1, Ordering::Relaxed);
        warn!(
            event = "parse_request_failed",
            reason = "validation_failed",
            method = "POST",
            error = validation_error
        );
        return Ok(Json(ParseResponse {
            success: false,
            result: None,
            error: Some(validation_error),
            processing_time_ms: Some(start_time.elapsed().as_millis() as u64),
        }));
    }

    info!(
        event = "parse_request_started",
        method = "POST",
        address_length = address.len(),
        "Processing address parsing request"
    );

    let parse_start = Instant::now();
    let parse_result = timeout(state.request_timeout, state.parser.parse(address)).await;

    let parsed_result = match parse_result {
        Ok(result) => result,
        Err(_) => {
            FAILED_PARSES.fetch_add(1, Ordering::Relaxed);
            TIMEOUT_ERRORS.fetch_add(1, Ordering::Relaxed);
            error!(
                event = "parse_request_timeout",
                method = "POST",
                address_length = address.len(),
                timeout_secs = state.request_timeout.as_secs(),
                "Request timed out"
            );
            return Ok(Json(ParseResponse {
                success: false,
                result: None,
                error: Some("Request timeout".to_string()),
                processing_time_ms: Some(start_time.elapsed().as_millis() as u64),
            }));
        }
    };

    let parse_duration = parse_start.elapsed();
    let total_duration = start_time.elapsed();

    let parse_time_ms = parse_duration.as_millis() as u64;
    let total_time_ms = total_duration.as_millis() as u64;

    update_parse_time_metrics(parse_time_ms);

    info!(
        event = "parse_request_completed",
        method = "POST",
        success = true,
        address_length = address.len(),
        parse_time_ms = parse_time_ms,
        total_time_ms = total_time_ms,
        "Successfully parsed address"
    );

    SUCCESSFUL_PARSES.fetch_add(1, Ordering::Relaxed);
    Ok(Json(ParseResponse {
        success: true,
        result: Some(parsed_result.into()),
        error: None,
        processing_time_ms: Some(total_time_ms),
    }))
}

async fn health() -> Json<serde_json::Value> {
    info!(event = "health_check", status = "healthy");

    let uptime_seconds = START_TIME
        .get()
        .and_then(|start| SystemTime::now().duration_since(*start).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Json(json!({
        "status": "healthy",
        "service": "japanese-address-parser-api",
        "version": env!("CARGO_PKG_VERSION"),
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "uptime_seconds": uptime_seconds
    }))
}

async fn metrics() -> (StatusCode, String) {
    let total = TOTAL_REQUESTS.load(Ordering::Relaxed);
    let successful = SUCCESSFUL_PARSES.load(Ordering::Relaxed);
    let failed = FAILED_PARSES.load(Ordering::Relaxed);
    let get_requests = GET_REQUESTS.load(Ordering::Relaxed);
    let post_requests = POST_REQUESTS.load(Ordering::Relaxed);
    let timeout_errors = TIMEOUT_ERRORS.load(Ordering::Relaxed);
    let validation_errors = VALIDATION_ERRORS.load(Ordering::Relaxed);

    let parse_time_total = PARSE_TIME_TOTAL_MS.load(Ordering::Relaxed);
    let min_parse_time = MIN_PARSE_TIME_MS.load(Ordering::Relaxed);
    let max_parse_time = MAX_PARSE_TIME_MS.load(Ordering::Relaxed);

    let avg_parse_time = if successful > 0 {
        parse_time_total as f64 / successful as f64
    } else {
        0.0
    };

    let success_rate = if total > 0 {
        (successful as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let uptime_seconds = START_TIME
        .get()
        .and_then(|start| SystemTime::now().duration_since(*start).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let buckets = *PARSE_TIME_BUCKETS.lock().unwrap();

    info!(
        event = "metrics_requested",
        total_requests = total,
        successful_parses = successful,
        failed_parses = failed,
        success_rate = success_rate,
        avg_parse_time_ms = avg_parse_time
    );

    let prometheus_metrics = format!(
        "# HELP japanese_address_parser_requests_total Total number of address parsing requests\n\
         # TYPE japanese_address_parser_requests_total counter\n\
         japanese_address_parser_requests_total {}\n\
         \n\
         # HELP japanese_address_parser_requests_by_method_total Total requests by HTTP method\n\
         # TYPE japanese_address_parser_requests_by_method_total counter\n\
         japanese_address_parser_requests_by_method_total{{method=\"GET\"}} {}\n\
         japanese_address_parser_requests_by_method_total{{method=\"POST\"}} {}\n\
         \n\
         # HELP japanese_address_parser_requests_successful_total Total number of successful address parsing requests\n\
         # TYPE japanese_address_parser_requests_successful_total counter\n\
         japanese_address_parser_requests_successful_total {}\n\
         \n\
         # HELP japanese_address_parser_requests_failed_total Total number of failed address parsing requests\n\
         # TYPE japanese_address_parser_requests_failed_total counter\n\
         japanese_address_parser_requests_failed_total {}\n\
         \n\
         # HELP japanese_address_parser_timeout_errors_total Total number of timeout errors\n\
         # TYPE japanese_address_parser_timeout_errors_total counter\n\
         japanese_address_parser_timeout_errors_total {}\n\
         \n\
         # HELP japanese_address_parser_validation_errors_total Total number of validation errors\n\
         # TYPE japanese_address_parser_validation_errors_total counter\n\
         japanese_address_parser_validation_errors_total {}\n\
         \n\
         # HELP japanese_address_parser_success_rate_percent Success rate of address parsing requests as percentage\n\
         # TYPE japanese_address_parser_success_rate_percent gauge\n\
         japanese_address_parser_success_rate_percent {:.2}\n\
         \n\
         # HELP japanese_address_parser_parse_duration_seconds_total Total time spent parsing addresses in seconds\n\
         # TYPE japanese_address_parser_parse_duration_seconds_total counter\n\
         japanese_address_parser_parse_duration_seconds_total {:.3}\n\
         \n\
         # HELP japanese_address_parser_parse_duration_seconds Average parsing duration in seconds\n\
         # TYPE japanese_address_parser_parse_duration_seconds gauge\n\
         japanese_address_parser_parse_duration_seconds{{stat=\"avg\"}} {:.6}\n\
         japanese_address_parser_parse_duration_seconds{{stat=\"min\"}} {:.6}\n\
         japanese_address_parser_parse_duration_seconds{{stat=\"max\"}} {:.6}\n\
         \n\
         # HELP japanese_address_parser_parse_duration_histogram Parse duration distribution\n\
         # TYPE japanese_address_parser_parse_duration_histogram histogram\n\
         japanese_address_parser_parse_duration_histogram_bucket{{le=\"0.001\"}} {}\n\
         japanese_address_parser_parse_duration_histogram_bucket{{le=\"0.005\"}} {}\n\
         japanese_address_parser_parse_duration_histogram_bucket{{le=\"0.010\"}} {}\n\
         japanese_address_parser_parse_duration_histogram_bucket{{le=\"0.025\"}} {}\n\
         japanese_address_parser_parse_duration_histogram_bucket{{le=\"0.050\"}} {}\n\
         japanese_address_parser_parse_duration_histogram_bucket{{le=\"0.100\"}} {}\n\
         japanese_address_parser_parse_duration_histogram_bucket{{le=\"0.500\"}} {}\n\
         japanese_address_parser_parse_duration_histogram_bucket{{le=\"+Inf\"}} {}\n\
         \n\
         # HELP japanese_address_parser_uptime_seconds Service uptime in seconds\n\
         # TYPE japanese_address_parser_uptime_seconds gauge\n\
         japanese_address_parser_uptime_seconds {}\n",
        total,
        get_requests,
        post_requests,
        successful,
        failed,
        timeout_errors,
        validation_errors,
        success_rate,
        parse_time_total as f64 / 1000.0, // Convert to seconds
        avg_parse_time / 1000.0,          // Convert to seconds
        if min_parse_time == u64::MAX { 0.0 } else { min_parse_time as f64 / 1000.0 },
        max_parse_time as f64 / 1000.0,
        buckets[0],
        buckets[0] + buckets[1],
        buckets[0] + buckets[1] + buckets[2],
        buckets[0] + buckets[1] + buckets[2] + buckets[3],
        buckets[0] + buckets[1] + buckets[2] + buckets[3] + buckets[4],
        buckets[0] + buckets[1] + buckets[2] + buckets[3] + buckets[4] + buckets[5],
        buckets[0] + buckets[1] + buckets[2] + buckets[3] + buckets[4] + buckets[5] + buckets[6],
        total,
        uptime_seconds
    );

    (StatusCode::OK, prometheus_metrics)
}

fn create_app() -> Router {
    let state = AppState::new();

    let max_request_size = std::env::var("MAX_REQUEST_SIZE")
        .unwrap_or_else(|_| DEFAULT_MAX_REQUEST_SIZE.to_string())
        .parse::<usize>()
        .unwrap_or(DEFAULT_MAX_REQUEST_SIZE);

    Router::new()
        .route("/parse", get(parse_address).post(parse_address_post))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(middleware::from_fn(request_logging_middleware))
                .layer(TraceLayer::new_for_http())
                .layer(RequestBodyLimitLayer::new(max_request_size))
                .layer(
                    CorsLayer::new()
                        .allow_origin(Any)
                        .allow_methods(Any)
                        .allow_headers(Any),
                ),
        )
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C signal, initiating graceful shutdown...");
        },
        _ = terminate => {
            info!("Received SIGTERM signal, initiating graceful shutdown...");
        },
    }

    info!("Shutdown signal received, cleaning up...");
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .json()
        .with_target(false)
        .with_current_span(false)
        .with_span_list(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let app = create_app();

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse::<u16>()
        .map_err(|e| {
            error!(event = "invalid_port", error = %e, "Failed to parse PORT environment variable");
            e
        })?;

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let addr = format!("{}:{}", host, port);

    info!(
        event = "server_starting",
        host = %host,
        port = port,
        addr = %addr,
        version = env!("CARGO_PKG_VERSION"),
        "Starting Japanese Address Parser API"
    );

    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        error!(event = "bind_failed", addr = %addr, error = %e, "Failed to bind to address");
        e
    })?;

    info!(
        event = "server_started",
        addr = %addr,
        endpoints = ?["/parse", "/health", "/metrics"],
        "Server running successfully"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| {
            error!(event = "server_error", error = %e, "Server encountered an error");
            e
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_parse_get_missing_address() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/parse")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_parse_get_valid_address() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/parse?address=東京都渋谷区神宮前1-1-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_parse_post_valid_address() {
        let app = create_app();

        let body = serde_json::json!({
            "address": "東京都渋谷区神宮前1-1-1"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/parse")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_parse_post_empty_address() {
        let app = create_app();

        let body = serde_json::json!({
            "address": ""
        });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/parse")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn test_validate_address() {
        // Valid addresses
        assert!(validate_address("東京都渋谷区神宮前1-1-1").is_ok());
        assert!(validate_address("Tokyo").is_ok());
        assert!(validate_address("123 Main St").is_ok());

        // Invalid addresses
        assert!(validate_address("").is_err());
        assert!(validate_address("   ").is_err());
        assert!(validate_address(&"a".repeat(501)).is_err());
    }
}
