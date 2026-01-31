/// HTTP API for exporting trade data
/// Provides REST endpoints for accessing positions, trades, and statistics
///
/// This module is optional - only starts if API_ENABLED=true in settings

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use crate::config::reloadable::ReloadableTraders;
use crate::persistence::{TradeStore, TradeRecord};

/// API server configuration
#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8080,
        }
    }
}

/// Shared state for API handlers
#[derive(Clone)]
struct AppState {
    db_path: Option<String>,
    start_time: Instant,
    /// Optional reloadable traders config for the /reload endpoint
    traders: Option<ReloadableTraders>,
}

/// Health check response
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct HealthResponse {
    status: String,
    uptime_seconds: u64,
}

/// Position response (matches Position from TradeStore)
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct PositionResponse {
    token_id: String,
    net_shares: f64,
    avg_entry_price: Option<f64>,
    trade_count: i32,
}

/// Trade response (simplified from TradeRecord)
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TradeResponse {
    timestamp_ms: i64,
    block_number: u64,
    tx_hash: String,
    trader_address: String,
    token_id: String,
    side: String,
    whale_shares: f64,
    whale_price: f64,
    whale_usd: f64,
    our_shares: Option<f64>,
    our_price: Option<f64>,
    our_usd: Option<f64>,
    fill_pct: Option<f64>,
    status: String,
    latency_ms: Option<i64>,
    is_live: Option<bool>,
    aggregation_count: Option<u32>,
    aggregation_window_ms: Option<u64>,
}

/// Stats response
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct StatsResponse {
    total_orders: u32,
    aggregated_orders: u32,
    total_trades_combined: u32,
    avg_trades_per_aggregation: f64,
    total_positions: usize,
}

/// Query parameters for /trades endpoint
#[derive(Debug, Deserialize)]
struct TradesQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    since: Option<i64>,
}

fn default_limit() -> usize {
    50
}

/// Health check endpoint
/// Returns bot status and uptime
async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();

    let response = HealthResponse {
        status: "ok".to_string(),
        uptime_seconds: uptime,
    };

    Json(response)
}

/// Positions endpoint
/// Returns current positions from TradeStore
async fn positions_handler(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let db_path = match &state.db_path {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "Database not available"})),
            )
                .into_response();
        }
    };

    // Create a TradeStore connection for this request
    let store = match TradeStore::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to connect to database: {}", e)})),
            )
                .into_response();
        }
    };

    match store.get_positions() {
        Ok(positions) => {
            let response: Vec<PositionResponse> = positions
                .into_iter()
                .map(|p| PositionResponse {
                    token_id: p.token_id,
                    net_shares: p.net_shares,
                    avg_entry_price: p.avg_entry_price,
                    trade_count: p.trade_count,
                })
                .collect();
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to get positions: {}", e)})),
        )
            .into_response(),
    }
}

/// Trades endpoint
/// Returns trade history with optional filters
async fn trades_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TradesQuery>,
) -> axum::response::Response {
    let db_path = match &state.db_path {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "Database not available"})),
            )
                .into_response();
        }
    };

    // Create a TradeStore connection for this request
    let store = match TradeStore::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to connect to database: {}", e)})),
            )
                .into_response();
        }
    };

    match store.get_recent_trades(params.limit) {
        Ok(trades) => {
            // Filter by timestamp if since parameter is provided
            let filtered: Vec<TradeRecord> = if let Some(since_ms) = params.since {
                trades
                    .into_iter()
                    .filter(|t| t.timestamp_ms >= since_ms)
                    .collect()
            } else {
                trades
            };

            let response: Vec<TradeResponse> = filtered
                .into_iter()
                .map(|t| TradeResponse {
                    timestamp_ms: t.timestamp_ms,
                    block_number: t.block_number,
                    tx_hash: t.tx_hash,
                    trader_address: t.trader_address,
                    token_id: t.token_id,
                    side: t.side,
                    whale_shares: t.whale_shares,
                    whale_price: t.whale_price,
                    whale_usd: t.whale_usd,
                    our_shares: t.our_shares,
                    our_price: t.our_price,
                    our_usd: t.our_usd,
                    fill_pct: t.fill_pct,
                    status: t.status,
                    latency_ms: t.latency_ms,
                    is_live: t.is_live,
                    aggregation_count: t.aggregation_count,
                    aggregation_window_ms: t.aggregation_window_ms,
                })
                .collect();
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to get trades: {}", e)})),
        )
            .into_response(),
    }
}

/// Stats endpoint
/// Returns aggregation stats and overall statistics
async fn stats_handler(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let db_path = match &state.db_path {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "Database not available"})),
            )
                .into_response();
        }
    };

    // Create a TradeStore connection for this request
    let store = match TradeStore::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to connect to database: {}", e)})),
            )
                .into_response();
        }
    };

    match store.get_aggregation_stats() {
        Ok(agg_stats) => {
            // Also get positions count
            let positions_count = store.get_positions().map(|p| p.len()).unwrap_or(0);

            let response = StatsResponse {
                total_orders: agg_stats.total_orders,
                aggregated_orders: agg_stats.aggregated_orders,
                total_trades_combined: agg_stats.total_trades_combined,
                avg_trades_per_aggregation: agg_stats.avg_trades_per_aggregation,
                total_positions: positions_count,
            };
            Json(response).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to get stats: {}", e)})),
        )
            .into_response(),
    }
}

/// Reload response
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ReloadResponse {
    success: bool,
    changed: bool,
    message: String,
}

/// Reload endpoint
/// Reloads trader configuration from traders.json or environment variables
async fn reload_handler(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let traders = match &state.traders {
        Some(t) => t,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReloadResponse {
                    success: false,
                    changed: false,
                    message: "Reload not available (traders config not set)".to_string(),
                }),
            )
                .into_response();
        }
    };

    match traders.reload().await {
        Ok(changed) => {
            let message = if changed {
                "Configuration reloaded successfully. WebSocket will reconnect with new traders."
            } else {
                "Configuration unchanged."
            };
            Json(ReloadResponse {
                success: true,
                changed,
                message: message.to_string(),
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ReloadResponse {
                success: false,
                changed: false,
                message: format!("Failed to reload configuration: {}", e),
            }),
        )
            .into_response(),
    }
}

/// Creates the API router with all endpoints
fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/positions", get(positions_handler))
        .route("/trades", get(trades_handler))
        .route("/stats", get(stats_handler))
        .route("/reload", post(reload_handler))
        .with_state(state)
}

/// Starts the HTTP API server
/// Returns a JoinHandle that can be awaited for graceful shutdown
pub async fn start_api_server(
    config: ApiConfig,
    db_path: Option<String>,
) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error + Send + Sync>> {
    start_api_server_with_reload(config, db_path, None).await
}

/// Starts the HTTP API server with optional reload support
/// Returns a JoinHandle that can be awaited for graceful shutdown
pub async fn start_api_server_with_reload(
    config: ApiConfig,
    db_path: Option<String>,
    traders: Option<ReloadableTraders>,
) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error + Send + Sync>> {
    if !config.enabled {
        return Err("API is disabled".into());
    }

    let state = Arc::new(AppState {
        db_path,
        start_time: Instant::now(),
        traders,
    });

    let app = create_router(state);
    let addr = format!("127.0.0.1:{}", config.port);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_config_default() {
        let config = ApiConfig::default();
        assert!(!config.enabled, "API should be disabled by default");
        assert_eq!(config.port, 8080, "Default port should be 8080");
    }

    #[test]
    fn test_health_response_structure() {
        let response = HealthResponse {
            status: "ok".to_string(),
            uptime_seconds: 123,
        };

        assert_eq!(response.status, "ok");
        assert_eq!(response.uptime_seconds, 123);
    }

    #[test]
    fn test_health_response_serialization() {
        let response = HealthResponse {
            status: "ok".to_string(),
            uptime_seconds: 123,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"uptime_seconds\":123"));
    }

    #[test]
    fn test_health_response_deserialization() {
        let json = r#"{"status":"ok","uptime_seconds":456}"#;
        let response: HealthResponse = serde_json::from_str(json).unwrap();

        assert_eq!(response.status, "ok");
        assert_eq!(response.uptime_seconds, 456);
    }

    #[tokio::test]
    async fn test_api_disabled_by_default() {
        let config = ApiConfig::default();
        let result = start_api_server(config, None).await;

        assert!(result.is_err(), "API server should fail to start when disabled");
        assert_eq!(result.unwrap_err().to_string(), "API is disabled");
    }

    #[tokio::test]
    async fn test_health_endpoint_returns_valid_json() {
        // Create a test server
        let config = ApiConfig {
            enabled: true,
            port: 18080, // Use a different port for testing
        };

        let handle = start_api_server(config.clone(), None).await.unwrap();

        // Give the server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Make a request to the health endpoint
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/health", config.port))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let health: HealthResponse = response.json().await.unwrap();
        assert_eq!(health.status, "ok");
        assert!(health.uptime_seconds >= 0);

        // Cleanup: abort the server task
        handle.abort();
    }

    #[tokio::test]
    async fn test_health_endpoint_uptime_increases() {
        let config = ApiConfig {
            enabled: true,
            port: 18081,
        };

        let handle = start_api_server(config.clone(), None).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

        // First request
        let response1 = client
            .get(format!("http://127.0.0.1:{}/health", config.port))
            .send()
            .await
            .unwrap();
        let health1: HealthResponse = response1.json().await.unwrap();

        // Wait a bit
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Second request
        let response2 = client
            .get(format!("http://127.0.0.1:{}/health", config.port))
            .send()
            .await
            .unwrap();
        let health2: HealthResponse = response2.json().await.unwrap();

        assert!(
            health2.uptime_seconds > health1.uptime_seconds,
            "Uptime should increase between requests"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn test_api_binds_to_localhost_only() {
        let config = ApiConfig {
            enabled: true,
            port: 18082,
        };

        let handle = start_api_server(config.clone(), None).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Should work on localhost
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/health", config.port))
            .send()
            .await;

        assert!(response.is_ok(), "Should be accessible on localhost");

        handle.abort();
    }

    // Helper to create a test database with sample data
    fn create_test_db_with_data() -> (tempfile::TempDir, String) {
        use crate::persistence::TradeRecord;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let store = TradeStore::new(&db_path).unwrap();

        // Insert some test trades
        for i in 0..3 {
            let record = TradeRecord {
                timestamp_ms: 1706000000000 + i,
                block_number: 12345678,
                tx_hash: format!("0xtx{}", i),
                trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                token_id: format!("token{}", i % 2), // Two different tokens
                side: if i % 2 == 0 { "BUY" } else { "SELL" }.to_string(),
                whale_shares: 100.0,
                whale_price: 0.50,
                whale_usd: 50.0,
                our_shares: Some(10.0),
                our_price: Some(0.51),
                our_usd: Some(5.1),
                fill_pct: Some(100.0),
                status: "SUCCESS".to_string(),
                latency_ms: Some(85),
                is_live: Some(false),
                aggregation_count: if i == 2 { Some(2) } else { None },
                aggregation_window_ms: if i == 2 { Some(500) } else { None },
            };
            store.insert_trade(&record).unwrap();
        }

        (temp_dir, db_path.to_string_lossy().to_string())
    }

    #[tokio::test]
    async fn test_positions_endpoint_returns_positions() {
        let (_temp_dir, db_path) = create_test_db_with_data();

        let config = ApiConfig {
            enabled: true,
            port: 18083,
        };

        let handle = start_api_server(config.clone(), Some(db_path)).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/positions", config.port))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let positions: Vec<PositionResponse> = response.json().await.unwrap();
        assert!(!positions.is_empty(), "Should have at least one position");

        handle.abort();
    }

    #[tokio::test]
    async fn test_positions_endpoint_without_database() {
        let config = ApiConfig {
            enabled: true,
            port: 18084,
        };

        let handle = start_api_server(config.clone(), None).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/positions", config.port))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 503); // SERVICE_UNAVAILABLE

        handle.abort();
    }

    #[tokio::test]
    async fn test_trades_endpoint_returns_trades() {
        let (_temp_dir, db_path) = create_test_db_with_data();

        let config = ApiConfig {
            enabled: true,
            port: 18085,
        };

        let handle = start_api_server(config.clone(), Some(db_path)).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/trades", config.port))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let trades: Vec<TradeResponse> = response.json().await.unwrap();
        assert_eq!(trades.len(), 3, "Should return 3 trades");

        handle.abort();
    }

    #[tokio::test]
    async fn test_trades_endpoint_with_limit() {
        let (_temp_dir, db_path) = create_test_db_with_data();

        let config = ApiConfig {
            enabled: true,
            port: 18086,
        };

        let handle = start_api_server(config.clone(), Some(db_path)).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/trades?limit=2", config.port))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let trades: Vec<TradeResponse> = response.json().await.unwrap();
        assert_eq!(trades.len(), 2, "Should return 2 trades (limited)");

        handle.abort();
    }

    #[tokio::test]
    async fn test_trades_endpoint_with_since_filter() {
        let (_temp_dir, db_path) = create_test_db_with_data();

        let config = ApiConfig {
            enabled: true,
            port: 18087,
        };

        let handle = start_api_server(config.clone(), Some(db_path)).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        // Filter trades since timestamp 1706000000001 (should return 2 trades)
        let response = client
            .get(format!(
                "http://127.0.0.1:{}/trades?since=1706000000001",
                config.port
            ))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let trades: Vec<TradeResponse> = response.json().await.unwrap();
        assert_eq!(trades.len(), 2, "Should return 2 trades after filter");

        handle.abort();
    }

    #[tokio::test]
    async fn test_stats_endpoint_returns_stats() {
        let (_temp_dir, db_path) = create_test_db_with_data();

        let config = ApiConfig {
            enabled: true,
            port: 18088,
        };

        let handle = start_api_server(config.clone(), Some(db_path)).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/stats", config.port))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let stats: StatsResponse = response.json().await.unwrap();
        assert_eq!(stats.total_orders, 3);
        assert_eq!(stats.aggregated_orders, 1); // One trade has aggregation_count = 2
        assert!(stats.total_positions > 0);

        handle.abort();
    }

    #[tokio::test]
    async fn test_stats_endpoint_without_database() {
        let config = ApiConfig {
            enabled: true,
            port: 18089,
        };

        let handle = start_api_server(config.clone(), None).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/stats", config.port))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 503); // SERVICE_UNAVAILABLE

        handle.abort();
    }
}
