/// PM Whale Follower - Main entry point
/// Monitors blockchain for whale trades and executes copy trades

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use dotenvy::dotenv;
use alloy::primitives::U256;
use futures::{SinkExt, StreamExt};
use rand::Rng;
use pm_whale_follower::{ApiCreds, OrderArgs, RustClobClient, PreparedCreds, OrderResponse};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use pm_whale_follower::risk_guard::{RiskGuard, RiskGuardConfig, SafetyDecision, TradeSide, calc_liquidity_depth};
use pm_whale_follower::settings::*;
use pm_whale_follower::market_cache;
use pm_whale_follower::tennis_markets;
use pm_whale_follower::soccer_markets;
use pm_whale_follower::persistence::{TradeStore, TradeRecord};
use pm_whale_follower::portfolio::{PortfolioTracker, PortfolioConfig};
use pm_whale_follower::config::traders::TradersConfig;
use pm_whale_follower::config::reloadable::ReloadableTraders;
use pm_whale_follower::trader_state::{TraderManager, TradeStatus};
use pm_whale_follower::aggregator::{TradeAggregator, AggregationConfig};
use pm_whale_follower::api::{ApiConfig, start_api_server_with_reload};
use pm_whale_follower::models::*;
use std::sync::Arc;

const GAMMA_API_BASE: &str = "https://gamma-api.polymarket.com";

// ============================================================================
// Thread-local buffers 
// ============================================================================

thread_local! {
    static CSV_BUF: RefCell<String> = RefCell::new(String::with_capacity(512));
    static SANITIZE_BUF: RefCell<String> = RefCell::new(String::with_capacity(128));
    static TOKEN_ID_CACHE: RefCell<HashMap<[u8; 32], Arc<str>>> = RefCell::new(HashMap::with_capacity(256));
}

// ============================================================================
// Order Engine 
// ============================================================================

#[derive(Clone)]
struct OrderEngine {
    tx: mpsc::Sender<WorkItem>,
    #[allow(dead_code)]
    resubmit_tx: mpsc::UnboundedSender<ResubmitRequest>,
    enable_trading: bool,
}

impl OrderEngine {
    async fn submit(&self, evt: ParsedEvent, is_live: Option<bool>) -> String {
        if !self.enable_trading {
            return "SKIPPED_DISABLED".into();
        }

        let (resp_tx, resp_rx) = oneshot::channel();
        if let Err(e) = self.tx.try_send(WorkItem { event: evt, respond_to: resp_tx, is_live }) {
            return format!("QUEUE_ERR: {e}");
        }

        match tokio::time::timeout(ORDER_REPLY_TIMEOUT, resp_rx).await {
            Ok(Ok(msg)) => msg,
            Ok(Err(_)) => "WORKER_DROPPED".into(),
            Err(_) => "WORKER_TIMEOUT".into(),
        }
    }
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    ensure_csv()?;

    // Initialize market data caches
    market_cache::init_caches();

    // Start background cache refresh task
    let _cache_refresh_handle = market_cache::spawn_cache_refresh_task();

    let cfg = Config::from_env()?;

    // Create reloadable traders config for hot-reload support
    let reloadable_traders = ReloadableTraders::new(cfg.traders.clone());
    let mut config_change_rx = reloadable_traders.subscribe();

    // Initialize trader state manager
    let trader_manager = Arc::new(Mutex::new(TraderManager::new(&cfg.traders)));
    println!("Trader state manager initialized for {} traders", cfg.traders.len());

    // Initialize trade aggregator (if enabled)
    let aggregator = if cfg.agg_enabled {
        let agg_config = AggregationConfig {
            window_duration: Duration::from_millis(cfg.agg_window_ms),
            min_trades: 2,
            max_pending_usd: 500.0,
            bypass_threshold: cfg.agg_bypass_shares,
        };
        let agg = Arc::new(Mutex::new(TradeAggregator::new(agg_config)));
        println!(
            "Trade aggregation enabled: {}ms window, bypass threshold: {} shares",
            cfg.agg_window_ms, cfg.agg_bypass_shares
        );
        Some(agg)
    } else {
        println!("Trade aggregation disabled");
        None
    };

    // Initialize trade persistence channel (if enabled)
    // Uses a dedicated background thread to handle SQLite operations
    let (trade_tx, stats_persist_path) = if cfg.db_enabled {
        let db_path = cfg.db_path.clone();
        let (tx, rx) = mpsc::unbounded_channel::<TradeRecord>();

        // Spawn a background thread for persistence (SQLite is not Send)
        std::thread::spawn(move || {
            persistence_worker(rx, &db_path);
        });

        println!("Trade persistence enabled: {}", cfg.db_path);
        (Some(tx), Some(cfg.db_path.clone()))
    } else {
        println!("Trade persistence disabled");
        (None, None)
    };

    // Start HTTP API server (if enabled)
    if cfg.api_enabled {
        let api_config = ApiConfig {
            enabled: cfg.api_enabled,
            port: cfg.api_port,
        };
        let api_db_path = stats_persist_path.clone();

        match start_api_server_with_reload(api_config, api_db_path, Some(reloadable_traders.clone())).await {
            Ok(_handle) => {
                println!("HTTP API server started on http://127.0.0.1:{}", cfg.api_port);
                println!("  - GET /health - Health check");
                println!("  - GET /positions - Current positions");
                println!("  - GET /trades?limit=N&since=TS - Trade history");
                println!("  - GET /stats - Aggregation statistics");
                println!("  - POST /reload - Reload trader configuration");
            }
            Err(e) => {
                eprintln!("Warning: Failed to start API server: {}", e);
            }
        }
    }

    let (client, creds) = build_worker_state(
        cfg.private_key.clone(),
        cfg.funder_address.clone(),
        ".clob_market_cache.json",
        ".clob_creds.json",
    ).await?;

    let prepared_creds = PreparedCreds::from_api_creds(&creds)?;
    let risk_config = cfg.risk_guard_config();

    // Initialize portfolio tracker for dynamic bet sizing (if configured)
    let portfolio_tracker = cfg.max_bet_portfolio_percent.map(|percent| {
        let portfolio_config = PortfolioConfig {
            wallet_address: cfg.wallet_address.clone(),
            cache_duration_secs: cfg.portfolio_cache_secs,
            max_bet_portfolio_percent: Some(percent),
        };
        let tracker = PortfolioTracker::new(portfolio_config);
        println!(
            "Portfolio-based bet limit enabled: {:.1}% of portfolio, cache: {}s",
            percent * 100.0, cfg.portfolio_cache_secs
        );
        Arc::new(tracker)
    });

    let (order_tx, order_rx) = mpsc::channel(1024);
    let (resubmit_tx, resubmit_rx) = mpsc::unbounded_channel::<ResubmitRequest>();

    let client_arc = Arc::new(client);
    let creds_arc = Arc::new(prepared_creds.clone());

    start_order_worker(order_rx, client_arc.clone(), prepared_creds, cfg.enable_trading, cfg.mock_trading, risk_config, resubmit_tx.clone(), stats_persist_path.clone(), portfolio_tracker);

    tokio::spawn(resubmit_worker(resubmit_rx, client_arc, creds_arc));

    let order_engine = OrderEngine {
        tx: order_tx,
        resubmit_tx,
        enable_trading: cfg.enable_trading,
    };

    println!(
        "🚀 Starting trader. Trading: {}, Mock: {}",
        cfg.enable_trading, cfg.mock_trading
    );

    // Spawn background flush task for aggregator (if enabled)
    if let Some(ref agg) = aggregator {
        let agg_clone = Arc::clone(agg);
        let order_engine_clone = order_engine.clone();
        let trade_tx_clone = trade_tx.clone();
        let trader_manager_clone = Arc::clone(&trader_manager);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                interval.tick().await;

                // Flush expired aggregation windows
                let expired = {
                    let mut agg_lock = agg_clone.lock().await;
                    agg_lock.flush_expired()
                };

                // Execute each aggregated trade
                for aggregated in expired {
                    let count = aggregated.trade_count;
                    let token_id = aggregated.token_id.clone();
                    println!(
                        "[AGG] Window flush: {} trades combined into 1 order ({:.2} shares @ {:.4} avg)",
                        count, aggregated.total_shares, aggregated.avg_price
                    );

                    // Get market liveness from cache (or None if not cached)
                    let is_live = market_cache::get_is_live(&token_id);

                    // Convert aggregated trade to event and execute
                    let evt = aggregated.to_parsed_event();
                    let status = order_engine_clone.submit(evt.clone(), is_live).await;
                    println!("[AGG] Flush result: {}", status);

                    // Record the aggregated trade result to CSV and DB
                    record_aggregated_trade(
                        &evt,
                        &status,
                        is_live,
                        &trade_tx_clone,
                        &trader_manager_clone,
                        count,
                    ).await;
                }
            }
        });
    }

    // Spawn SIGHUP handler for configuration reload (Unix only)
    #[cfg(unix)]
    {
        let reloadable_traders_sighup = reloadable_traders.clone();
        tokio::spawn(async move {
            let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
                .expect("Failed to register SIGHUP handler");
            loop {
                sighup.recv().await;
                println!("\n🔄 Received SIGHUP, reloading trader configuration...");
                match reloadable_traders_sighup.reload().await {
                    Ok(true) => {
                        println!("✅ Configuration reloaded. WebSocket will reconnect with new traders.");
                    }
                    Ok(false) => {
                        println!("ℹ️ Configuration unchanged.");
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to reload configuration: {}", e);
                    }
                }
            }
        });
        println!("SIGHUP handler registered (kill -HUP {} to reload config)", std::process::id());
    }

    // Spawn signal handler for graceful shutdown
    // Note: When the channel is dropped, the persistence worker will flush and exit
    let aggregator_shutdown = aggregator.clone();
    let order_engine_shutdown = order_engine.clone();
    let trade_tx_shutdown = trade_tx.clone();
    let trader_manager_shutdown = Arc::clone(&trader_manager);
    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            println!("\nReceived shutdown signal, shutting down...");

            // Flush any pending aggregations before shutdown
            if let Some(agg) = aggregator_shutdown {
                let pending_aggregations = {
                    let mut agg_lock = agg.lock().await;
                    agg_lock.flush_all()
                };

                if !pending_aggregations.is_empty() {
                    println!(
                        "[AGG] Shutdown: flushing {} pending aggregations",
                        pending_aggregations.len()
                    );

                    // Execute pending aggregations before exit
                    for aggregated in pending_aggregations {
                        let token_id = aggregated.token_id.clone();
                        let count = aggregated.trade_count;
                        println!(
                            "[AGG] Shutdown flush: {} trades -> {:.2} shares @ {:.4} avg",
                            count, aggregated.total_shares, aggregated.avg_price
                        );

                        // Get market liveness from cache
                        let is_live = market_cache::get_is_live(&token_id);

                        // Execute the aggregated trade
                        let evt = aggregated.to_parsed_event();
                        let status = order_engine_shutdown.submit(evt.clone(), is_live).await;
                        println!("[AGG] Shutdown result: {}", status);

                        // Record the trade result to CSV and DB
                        record_aggregated_trade(
                            &evt,
                            &status,
                            is_live,
                            &trade_tx_shutdown,
                            &trader_manager_shutdown,
                            count,
                        ).await;
                    }
                }
            }

            std::process::exit(0);
        }
    });

    loop {
        // Check if config changed before connecting
        let current_gen = reloadable_traders.generation();

        if let Err(e) = run_ws_loop(&cfg, &reloadable_traders, &order_engine, trade_tx.clone(), Arc::clone(&trader_manager), stats_persist_path.clone(), aggregator.clone(), &mut config_change_rx).await {
            // Check if error was due to config reload
            if reloadable_traders.generation() != current_gen {
                println!("🔄 Config changed, reconnecting with new traders...");
            } else {
                eprintln!("⚠️ WS error: {e}. Reconnecting...");
            }
            tokio::time::sleep(WS_RECONNECT_DELAY).await;
        }
    }
}

// ============================================================================
// Worker Setup
// ============================================================================

async fn build_worker_state(
    private_key: String,
    funder: Option<String>,
    cache_path: &str,
    creds_path: &str,
) -> Result<(RustClobClient, ApiCreds)> {
    let cache_path = cache_path.to_string();
    let creds_path = creds_path.to_string();
    let host = CLOB_API_BASE.to_string();

    tokio::task::spawn_blocking(move || -> Result<(RustClobClient, ApiCreds)> {
        let mut client = RustClobClient::new(&host, 137, &private_key, funder.as_deref())?
            .with_cache_path(&cache_path);
        let _ = client.load_cache();

        let _ = client.prewarm_connections();

        let creds: ApiCreds = if Path::new(&creds_path).exists() {
            let data = std::fs::read_to_string(&creds_path)?;
            serde_json::from_str(&data)?
        } else {
            let derived = client.derive_api_key(0)?;
            std::fs::write(&creds_path, serde_json::to_string_pretty(&derived)?)?;
            derived
        };

        Ok((client, creds))
    }).await?
}

fn start_order_worker(
    rx: mpsc::Receiver<WorkItem>,
    client: Arc<RustClobClient>,
    creds: PreparedCreds,
    enable_trading: bool,
    mock_trading: bool,
    risk_config: RiskGuardConfig,
    resubmit_tx: mpsc::UnboundedSender<ResubmitRequest>,
    db_path: Option<String>,
    portfolio_tracker: Option<Arc<PortfolioTracker>>,
) {
    std::thread::spawn(move || {
        let mut guard = RiskGuard::new(risk_config);
        order_worker(rx, client, creds, enable_trading, mock_trading, &mut guard, resubmit_tx, db_path.as_deref(), portfolio_tracker);
    });
}

/// Background worker for trade persistence
/// Runs on a dedicated thread to avoid Send/Sync issues with rusqlite
fn persistence_worker(rx: mpsc::UnboundedReceiver<TradeRecord>, db_path: &str) {
    // Create TradeStore on this thread (SQLite connection is not Send)
    let store = match TradeStore::new(db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to initialize TradeStore in persistence worker: {}", e);
            return;
        }
    };

    // Create a runtime for the receiver
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime for persistence worker");

    rt.block_on(async {
        let mut rx = rx;
        while let Some(record) = rx.recv().await {
            store.record_trade(record);
        }

        // Channel closed - flush remaining trades
        match store.flush() {
            Ok(count) if count > 0 => println!("Flushed {} trades to database on shutdown", count),
            Ok(_) => {}
            Err(e) => eprintln!("Warning: Failed to flush trade store on shutdown: {}", e),
        }
    });
}

fn order_worker(
    mut rx: mpsc::Receiver<WorkItem>,
    client: Arc<RustClobClient>,
    creds: PreparedCreds,
    enable_trading: bool,
    mock_trading: bool,
    guard: &mut RiskGuard,
    resubmit_tx: mpsc::UnboundedSender<ResubmitRequest>,
    db_path: Option<&str>,
    portfolio_tracker: Option<Arc<PortfolioTracker>>,
) {
    let mut client_mut = (*client).clone();
    while let Some(work) = rx.blocking_recv() {
        let status = process_order(&work.event.order, work.event.trader_min_shares, &mut client_mut, &creds, enable_trading, mock_trading, guard, &resubmit_tx, work.is_live, db_path, portfolio_tracker.as_ref());
        let _ = work.respond_to.send(status);
    }
}

// ============================================================================
// Order Processing
// ============================================================================

fn process_order(
    info: &OrderInfo,
    trader_min_shares: f64,
    client: &mut RustClobClient,
    creds: &PreparedCreds,
    enable_trading: bool,
    mock_trading: bool,
    guard: &mut RiskGuard,
    resubmit_tx: &mpsc::UnboundedSender<ResubmitRequest>,
    is_live: Option<bool>,
    db_path: Option<&str>,
    portfolio_tracker: Option<&Arc<PortfolioTracker>>,
) -> String {
    if !enable_trading { return "SKIPPED_DISABLED".into(); }
    if mock_trading { return "MOCK_ONLY".into(); }

    let side_is_buy = info.order_type.starts_with("BUY");
    let whale_shares = info.shares;
    let whale_price = info.price_per_share;

    // For SELL orders, check if we have shares to sell
    if !side_is_buy {
        if let Some(path) = db_path {
            match TradeStore::new(path) {
                Ok(store) => {
                    match store.get_positions() {
                        Ok(positions) => {
                            // Check if we have this token with positive shares
                            let has_position = positions.iter()
                                .any(|p| p.token_id == info.clob_token_id.as_ref() && p.net_shares > 0.0);
                            if !has_position {
                                return "SKIPPED_NO_POSITION".into();
                            }
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to check positions for SELL: {}", e);
                            // Continue anyway - let the exchange reject if no position
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to open DB for position check: {}", e);
                    // Continue anyway - let the exchange reject if no position
                }
            }
        }
        // If no db_path, we can't check positions - let the exchange handle it
    }

    // Skip small trades using per-trader threshold from traders.json
    // Falls back to global MIN_WHALE_SHARES_TO_COPY if trader_min_shares is 0
    let min_threshold = if trader_min_shares > 0.0 { trader_min_shares } else { MIN_WHALE_SHARES_TO_COPY };
    if whale_shares < min_threshold {
        return format!("SKIPPED_SMALL (<{:.0} shares)", min_threshold);
    }

    // Risk guard safety check
    let eval = guard.check_fast(&info.clob_token_id, whale_shares);
    match eval.decision {
        SafetyDecision::Block => return format!("RISK_BLOCKED:{}", eval.reason.as_str()),
        SafetyDecision::FetchBook => {
            let side = if side_is_buy { TradeSide::Buy } else { TradeSide::Sell };
            match fetch_book_depth_blocking(client, &info.clob_token_id, side, whale_price) {
                Ok(depth) => {
                    let final_eval = guard.check_with_book(&info.clob_token_id, eval.consecutive_large, depth);
                    if final_eval.decision == SafetyDecision::Block {
                        return format!("RISK_BLOCKED:{}", final_eval.reason.as_str());
                    }
                }
                Err(e) => {
                    guard.trip(&info.clob_token_id);
                    return format!("RISK_BOOK_FAIL:{e}");
                }
            }
        }
        SafetyDecision::Allow => {}
    }

    let (buffer, order_action, size_multiplier) = get_tier_params(whale_shares, side_is_buy, &info.clob_token_id);

    // Polymarket valid price range: 0.01 to 0.99 (tick size 0.01)
    let limit_price = if side_is_buy {
        (whale_price + buffer).min(0.99)
    } else {
        (whale_price - buffer).max(0.01)
    };

    // Calculate max bet in shares based on portfolio value (if configured)
    let max_bet_shares = portfolio_tracker
        .and_then(|tracker| tracker.get_max_bet_shares(limit_price));

    let (my_shares, size_type) = calculate_safe_size(whale_shares, limit_price, size_multiplier, max_bet_shares);
    if my_shares == 0.0 {
        return format!("SKIPPED_PROBABILITY ({})", size_type);
    }

    // Calculate expiration for GTD orders (SELL orders always use GTD)
    // FAK orders don't need expiration (use None)
    let expiration = if order_action == "GTD" {
        use std::time::{SystemTime, UNIX_EPOCH};
        let expiry_secs = get_gtd_expiry_secs(is_live.unwrap_or(false));
        let expiry_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() + expiry_secs;
        Some(expiry_timestamp.to_string())
    } else {
        None // FAK orders don't use expiration
    };

    let args = OrderArgs {
        token_id: info.clob_token_id.to_string(),
        price: limit_price,
        size: (my_shares * 100.0).floor() / 100.0,
        side: if side_is_buy { "BUY".into() } else { "SELL".into() },
        fee_rate_bps: None,
        nonce: Some(0),
        expiration,
        taker: None,
        order_type: Some(order_action.to_string()),
    };

    match client.create_order(args).and_then(|signed| {
        let body = signed.post_body(&creds.api_key, order_action);
        client.post_order_fast(body, creds)
    }) {
        Ok(resp) => {
            let status = resp.status();
            let body_text = resp.text().unwrap_or_default();

            // Verbose logging for response body
            if std::env::var("VERBOSE_ORDER_LOG").is_ok() {
                eprintln!("\n📥 Response Body:");
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_text) {
                    eprintln!("{}", serde_json::to_string_pretty(&json).unwrap_or(body_text.clone()));
                } else {
                    eprintln!("{}", body_text);
                }
                eprintln!();
            }

            let order_resp: Option<OrderResponse> = if status.is_success() {
                serde_json::from_str(&body_text).ok()
            } else {
                None
            };

            let mut underfill_msg: Option<String> = None;
            if let Some(ref resp) = order_resp {
                if side_is_buy && order_action == "FAK" {
                    let filled_shares: f64 = resp.taking_amount.parse().unwrap_or(0.0);
                    let requested_shares = (my_shares * 100.0).floor() / 100.0;

                    if filled_shares < requested_shares && filled_shares > 0.0 {
                        let remaining_shares = requested_shares - filled_shares;

                        let min_threshold = MIN_SHARE_COUNT.max(MIN_CASH_VALUE / limit_price);
                        if remaining_shares >= min_threshold {
                            let resubmit_buffer = get_resubmit_max_buffer(whale_shares);
                            let max_price = (limit_price + resubmit_buffer).min(0.99);
                            let req = ResubmitRequest {
                                token_id: info.clob_token_id.to_string(),
                                whale_price,
                                failed_price: limit_price,  // Start at same price (already filled some)
                                size: (remaining_shares * 100.0).floor() / 100.0,
                                whale_shares,
                                side_is_buy: true,
                                attempt: 1,
                                max_price,
                                cumulative_filled: filled_shares,
                                original_size: requested_shares,
                                is_live: is_live.unwrap_or(false),
                            };
                            let _ = resubmit_tx.send(req);
                            underfill_msg = Some(format!(
                                " | \x1b[33mUNDERFILL: {:.2}/{:.2} filled, resubmit {:.2}\x1b[0m",
                                filled_shares, my_shares, remaining_shares
                            ));
                        }
                    }
                }
            }

            if status.as_u16() == 400 && body_text.contains("FAK") && side_is_buy {
                let resubmit_buffer = get_resubmit_max_buffer(whale_shares);
                let max_price = (limit_price + resubmit_buffer).min(0.99);
                let rounded_size = (my_shares * 100.0).floor() / 100.0;
                let req = ResubmitRequest {
                    token_id: info.clob_token_id.to_string(),
                    whale_price,
                    failed_price: limit_price,
                    size: rounded_size,
                    whale_shares,
                    side_is_buy: true,
                    attempt: 1,
                    max_price,
                    cumulative_filled: 0.0,
                    original_size: rounded_size,
                    is_live: is_live.unwrap_or(false),
                };
                let _ = resubmit_tx.send(req);
            }

            // Extract filled shares and actual fill price for display (reuse parsed response)
            let (filled_shares, actual_fill_price) = order_resp.as_ref()
                .and_then(|r| {
                    let taking: f64 = r.taking_amount.parse().ok()?;
                    let making: f64 = r.making_amount.parse().ok()?;
                    if taking > 0.0 { Some((taking, making / taking)) } else { None }
                })
                .unwrap_or_else(|| {
                    if status.is_success() { (my_shares, limit_price) } else { (0.0, limit_price) }
                });

            // Format with color-coded fill percentage
            let pink = "\x1b[38;5;199m";
            let reset = "\x1b[0m";
            let fill_color = get_fill_color(filled_shares, my_shares);
            let whale_color = get_whale_size_color(whale_shares);
            let status_str = if status.is_success() { "200 OK" } else { "FAILED" };
            let mut base = format!(
                "{} [{}] | {}{:.2}/{:.2}{} filled @ {}{:.2}{} | {}whale {:.1}{} @ {:.2}",
                status_str, size_type, fill_color, filled_shares, my_shares, reset, pink, actual_fill_price, reset, whale_color, whale_shares, reset, whale_price
            );
            if let Some(msg) = underfill_msg {
                base.push_str(&msg);
            }
            if !status.is_success() {
                base.push_str(&format!(" | {}", body_text));
            }
            base
        }
        Err(e) => {
            let chain: Vec<_> = e.chain().map(|c| c.to_string()).collect();
            format!("EXEC_FAIL: {} | chain: {}", e, chain.join(" -> "))
        }
    }
}

fn calculate_safe_size(whale_shares: f64, price: f64, size_multiplier: f64, max_bet_shares: Option<f64>) -> (f64, SizeType) {
    let target_scaled = whale_shares * SCALING_RATIO * size_multiplier;
    let safe_price = price.max(0.0001);
    let required_floor = (MIN_CASH_VALUE / safe_price).max(MIN_SHARE_COUNT);

    // Apply portfolio-based cap if configured
    let target_capped = match max_bet_shares {
        Some(max) if max > 0.0 && target_scaled > max => max,
        _ => target_scaled,
    };

    if target_capped >= required_floor {
        // If we capped the size, indicate it in the size type
        if max_bet_shares.is_some() && target_scaled > target_capped {
            return (target_capped, SizeType::Capped);
        }
        return (target_capped, SizeType::Scaled);
    }

    if !USE_PROBABILISTIC_SIZING {
        return (required_floor, SizeType::Scaled);
    }

    let probability = target_capped / required_floor;
    let pct = (probability * 100.0) as u8;
    if rand::thread_rng().r#gen::<f64>() < probability {
        (required_floor, SizeType::ProbHit(pct))
    } else {
        (0.0, SizeType::ProbSkip(pct))
    }
}

/// Get ANSI color code based on fill percentage
fn get_fill_color(filled: f64, requested: f64) -> &'static str {
    if requested <= 0.0 { return "\x1b[31m"; }  // Red if no request
    let pct = (filled / requested) * 100.0;
    if pct < 50.0 { "\x1b[31m" }                // Red
    else if pct < 75.0 { "\x1b[38;5;208m" }     // Orange
    else if pct < 90.0 { "\x1b[33m" }           // Yellow
    else { "\x1b[32m" }                          // Green
}

/// Get ANSI color code based on whale share count (gradient from small to large)
fn get_whale_size_color(shares: f64) -> &'static str {
    if shares < 500.0 { "\x1b[90m" }              // Gray (very small)
    else if shares < 1000.0 { "\x1b[36m" }        // Cyan (small)
    else if shares < 2000.0 { "\x1b[34m" }        // Blue (medium-small)
    else if shares < 5000.0 { "\x1b[32m" }        // Green (medium)
    else if shares < 8000.0 { "\x1b[33m" }        // Yellow (medium-large)
    else if shares < 15000.0 { "\x1b[38;5;208m" } // Orange (large)
    else { "\x1b[35m" }                           // Magenta (huge)
}

fn fetch_book_depth_blocking(
    client: &RustClobClient,
    token_id: &str,
    side: TradeSide,
    threshold: f64,
) -> Result<f64, &'static str> {
    let url = format!("{}/book?token_id={}", CLOB_API_BASE, token_id);
    let resp = client.http_client()
        .get(&url)
        .timeout(Duration::from_millis(500))
        .send()
        .map_err(|_| "NETWORK")?;
    
    if !resp.status().is_success() { return Err("HTTP_ERROR"); }
    
    let book: Value = resp.json().map_err(|_| "PARSE")?;
    let key = if side == TradeSide::Buy { "asks" } else { "bids" };

    // Stack array instead of Vec - avoids heap allocation for max 10 items
    let mut levels: [(f64, f64); 10] = [(0.0, 0.0); 10];
    let mut count = 0;
    if let Some(arr) = book[key].as_array() {
        for lvl in arr.iter().take(10) {
            if let (Some(p), Some(s)) = (
                lvl["price"].as_str().and_then(|s| s.parse().ok()),
                lvl["size"].as_str().and_then(|s| s.parse().ok()),
            ) {
                levels[count] = (p, s);
                count += 1;
            }
        }
    }

    Ok(calc_liquidity_depth(side, &levels[..count], threshold))
}

// ============================================================================
// WebSocket Loop
// ============================================================================

/// Build WebSocket subscription message for monitoring trader events
/// Returns JSON-RPC subscription message as string
fn build_subscription_message(topic_filter: Vec<String>) -> String {
    let topics_array: Value = if topic_filter.is_empty() {
        // No filter - should not happen in practice
        serde_json::json!([[ORDERS_FILLED_EVENT_SIGNATURE], Value::Null, Value::Null])
    } else if topic_filter.len() > 10 {
        // Too many traders - use null filter and do client-side filtering
        serde_json::json!([[ORDERS_FILLED_EVENT_SIGNATURE], Value::Null, Value::Null])
    } else {
        // Normal case: filter by specific trader topics
        serde_json::json!([[ORDERS_FILLED_EVENT_SIGNATURE], Value::Null, topic_filter])
    };

    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "eth_subscribe",
        "params": ["logs", {
            "address": MONITORED_ADDRESSES,
            "topics": topics_array
        }]
    }).to_string()
}

async fn run_ws_loop(
    cfg: &Config,
    reloadable_traders: &ReloadableTraders,
    order_engine: &OrderEngine,
    trade_tx: Option<mpsc::UnboundedSender<TradeRecord>>,
    trader_manager: Arc<Mutex<TraderManager>>,
    stats_persist_path: Option<String>,
    aggregator: Option<Arc<Mutex<TradeAggregator>>>,
    config_change_rx: &mut tokio::sync::watch::Receiver<u64>,
) -> Result<()> {
    let (mut ws, _) = connect_async(&cfg.wss_url).await?;

    // Build topic filter from traders config
    let traders_config = reloadable_traders.read().await;
    let topic_filter = traders_config.build_topic_filter();
    let sub = build_subscription_message(topic_filter.clone());

    // Log trader monitoring info with topic details for debugging
    let trader_count = traders_config.iter().filter(|t| t.enabled).count();
    if topic_filter.len() > 10 {
        println!("🔌 Connected. Subscribing to {} traders (client-side filtering)...", trader_count);
    } else {
        println!("🔌 Connected. Subscribing to {} trader(s)...", trader_count);
    }

    // Debug: Print topic filter being used
    for (i, topic) in topic_filter.iter().enumerate() {
        println!("   📍 Trader {}: {}", i + 1, topic);
    }
    drop(traders_config); // Release the read lock

    ws.send(Message::Text(sub)).await?;

    let http_client = reqwest::Client::builder().no_proxy().build()?;
    let mut subscription_confirmed = false;
    let mut last_heartbeat = std::time::Instant::now();
    let heartbeat_interval = Duration::from_secs(60);

    // Get a snapshot of traders config for parsing events in this loop iteration
    // When config changes, we'll exit and reconnect with the new config
    let traders_snapshot = reloadable_traders.read().await.clone();

    loop {
        // Check for config changes - if changed, exit loop to reconnect
        if config_change_rx.has_changed().unwrap_or(false) {
            let _ = config_change_rx.borrow_and_update(); // Clear the changed flag
            return Err(anyhow!("Config changed, reconnecting"));
        }

        let msg = tokio::time::timeout(WS_PING_TIMEOUT, ws.next()).await
            .map_err(|_| anyhow!("WS timeout"))?
            .ok_or_else(|| anyhow!("WS closed"))??;

        match msg {
            Message::Text(text) => {
                // Check for subscription confirmation (first message after subscribing)
                if !subscription_confirmed {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        if v.get("id").and_then(|i| i.as_i64()) == Some(1) && v.get("result").is_some() {
                            subscription_confirmed = true;
                            println!("✅ Subscription confirmed. Listening for whale trades...");
                        }
                    }
                }

                if let Some(evt) = parse_event(text, Some(&traders_snapshot)) {
                    let engine = order_engine.clone();
                    let client = http_client.clone();
                    let tx = trade_tx.clone();
                    let tm = Arc::clone(&trader_manager);
                    let agg = aggregator.clone();
                    tokio::spawn(async move { handle_event(evt, &engine, &client, tx, tm, agg).await });
                }
            }
            Message::Binary(bin) => {
                if let Ok(text) = String::from_utf8(bin) {
                    if let Some(evt) = parse_event(text, Some(&traders_snapshot)) {
                        let engine = order_engine.clone();
                        let client = http_client.clone();
                        let tx = trade_tx.clone();
                        let tm = Arc::clone(&trader_manager);
                        let agg = aggregator.clone();
                        tokio::spawn(async move { handle_event(evt, &engine, &client, tx, tm, agg).await });
                    }
                }
            }
            Message::Ping(d) => { ws.send(Message::Pong(d)).await?; }
            Message::Close(f) => return Err(anyhow!("WS closed: {:?}", f)),
            _ => {}
        }

        // Periodic heartbeat to show bot is alive and check daily reset
        if last_heartbeat.elapsed() >= heartbeat_interval {
            // Check daily reset
            {
                let mut manager = trader_manager.lock().await;
                manager.check_daily_reset();
            }

            // Log summary stats
            let stats = {
                let manager = trader_manager.lock().await;
                manager.get_summary_stats()
            };

            println!(
                "💓 Heartbeat: {} traders | {} trades today | {}/{}/{} (success/partial/failed) | ${:.2} total copied",
                stats.total_traders,
                stats.total_trades,
                stats.total_successful,
                stats.total_partial,
                stats.total_failed,
                stats.total_copied_usd
            );

            // Persist trader stats to database (if enabled)
            if let Some(ref db_path) = stats_persist_path {
                let db_path = db_path.clone();
                let tm = Arc::clone(&trader_manager);
                tokio::task::spawn_blocking(move || {
                    if let Ok(store) = TradeStore::new(&db_path) {
                        let manager = tokio::runtime::Handle::current().block_on(tm.lock());
                        if let Err(e) = manager.persist_to_db(&store) {
                            eprintln!("Warning: Failed to persist trader stats: {}", e);
                        }
                    }
                });
            }

            last_heartbeat = std::time::Instant::now();
        }
    }
}

async fn handle_event(
    evt: ParsedEvent,
    order_engine: &OrderEngine,
    http_client: &reqwest::Client,
    trade_tx: Option<mpsc::UnboundedSender<TradeRecord>>,
    trader_manager: Arc<Mutex<TraderManager>>,
    aggregator: Option<Arc<Mutex<TradeAggregator>>>,
) {
    // Check live status from cache, fallback to API lookup
    let is_live = match market_cache::get_is_live(&evt.order.clob_token_id) {
        Some(v) => Some(v),
        None => fetch_is_live(&evt.order.clob_token_id, http_client).await,
    };

    // Aggregation logic (if enabled)
    let status = if let Some(agg) = aggregator {
        let side = if evt.order.order_type.starts_with("BUY") { "BUY" } else { "SELL" };
        let shares = evt.order.shares;
        let price = evt.order.price_per_share;
        let token_id = evt.order.clob_token_id.to_string();
        let trader = evt.trader_address.clone();

        // Add trade to aggregator and check if we should execute immediately
        let aggregation_result = {
            let mut agg_lock = agg.lock().await;
            agg_lock.add_trade(token_id.clone(), side.to_string(), shares, price, trader)
        };

        match aggregation_result {
            Some(aggregated) => {
                // Execute aggregated trade immediately (bypass or threshold reached)
                if aggregated.trade_count == 1 {
                    // Bypass: large trade executed immediately
                    println!("[AGG] Bypass: {:.2} shares executed immediately", aggregated.total_shares);
                } else {
                    // Aggregated: multiple trades combined
                    println!(
                        "[AGG] Aggregated: {} trades -> {:.2} shares @ {:.4} avg",
                        aggregated.trade_count, aggregated.total_shares, aggregated.avg_price
                    );
                }
                // Execute the aggregated trade with combined shares and avg price
                let agg_evt = aggregated.to_parsed_event();
                order_engine.submit(agg_evt, is_live).await
            }
            None => {
                // Trade added to pending window
                println!("[AGG] Pending: trade added to aggregation window");
                "AGG_PENDING".to_string()
            }
        }
    } else {
        // Aggregation disabled - execute immediately
        order_engine.submit(evt.clone(), is_live).await
    };

    tokio::time::sleep(Duration::from_secs_f32(2.8)).await;

    // Fetch order book for post-trade logging
    let bests = fetch_best_book(&evt.order.clob_token_id, &evt.order.order_type, http_client).await;
    let ((bp, bs), (sp, ss)) = bests.unwrap_or_else(|| (("N/A".into(), "N/A".into()), ("N/A".into(), "N/A".into())));
    let is_live_bool = is_live.unwrap_or(false);

    // Highlight best price in bright pink
    let pink = "\x1b[38;5;199m";
    let reset = "\x1b[0m";
    let colored_bp = format!("{}{}{}", pink, bp, reset);

    let live_display = if is_live_bool {
        format!("\x1b[34mlive: true\x1b[0m")
    } else {
        "live: false".to_string()
    };

    // Tennis market indicator (green)
    let tennis_display = if tennis_markets::get_tennis_token_buffer(&evt.order.clob_token_id) > 0.0 {
        "\x1b[32m(TENNIS)\x1b[0m "
    } else {
        ""
    };

    // Soccer market indicator (cyan)
    let soccer_display = if soccer_markets::get_soccer_token_buffer(&evt.order.clob_token_id) > 0.0 {
        "\x1b[36m(SOCCER)\x1b[0m "
    } else {
        ""
    };

    println!(
        "⚡ [B:{}] {}{}{} | ${:.0} | {} | best: {} @ {} | 2nd: {} @ {} | {}",
        evt.block_number, tennis_display, soccer_display, evt.order.order_type, evt.order.usd_value, status, colored_bp, bs, sp, ss, live_display
    );

    // Parse status to determine trade outcome and record in trader manager
    let (our_shares_opt, our_price_opt, our_usd_opt, fill_pct_opt, trade_status_str) = parse_status_for_db(&status);

    // Determine TradeStatus enum from status string
    let trade_status = if trade_status_str == "SUCCESS" {
        // Check fill percentage to distinguish full success from partial
        if let Some(fill_pct) = fill_pct_opt {
            if fill_pct >= 90.0 {
                TradeStatus::Success
            } else {
                TradeStatus::Partial
            }
        } else {
            TradeStatus::Success
        }
    } else if trade_status_str.starts_with("SKIPPED") {
        TradeStatus::Skipped
    } else {
        TradeStatus::Failed
    };

    // Record trade in trader manager (with USD amount from our execution)
    let usd_amount = our_usd_opt.unwrap_or(0.0);
    {
        let mut manager = trader_manager.lock().await;
        manager.record_trade(&evt.trader_address, usd_amount, trade_status);
    }

    // Record trade to database if persistence is enabled
    if let Some(tx) = trade_tx {
        let record = TradeRecord {
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            block_number: evt.block_number,
            tx_hash: evt.tx_hash.clone(),
            trader_address: evt.trader_address.clone(),
            token_id: evt.order.clob_token_id.to_string(),
            side: if evt.order.order_type.starts_with("BUY") { "BUY".to_string() } else { "SELL".to_string() },
            whale_shares: evt.order.shares,
            whale_price: evt.order.price_per_share,
            whale_usd: evt.order.usd_value,
            our_shares: our_shares_opt,
            our_price: our_price_opt,
            our_usd: our_usd_opt,
            fill_pct: fill_pct_opt,
            status: trade_status_str,
            latency_ms: None, // Could be added with timing instrumentation
            is_live,
            aggregation_count: None, // TODO: Set from aggregator when Phase 3 Step 3.2 integration complete
            aggregation_window_ms: None, // TODO: Set from aggregator when Phase 3 Step 3.2 integration complete
        };

        // Send to persistence worker (non-blocking)
        let _ = tx.send(record);
    }

    let ts: DateTime<Utc> = Utc::now();
    let row = CSV_BUF.with(|buf| {
        SANITIZE_BUF.with(|sbuf| {
            let mut b = buf.borrow_mut();
            let mut sb = sbuf.borrow_mut();
            sanitize_csv(&status, &mut sb);
            b.clear();
            let _ = write!(b,
                "{},{},{},{:.2},{:.6},{:.4},{},{},{},{},{},{},{},{}",
                ts.format("%Y-%m-%d %H:%M:%S%.3f"),
                evt.block_number, evt.order.clob_token_id, evt.order.usd_value,
                evt.order.shares, evt.order.price_per_share, evt.order.order_type,
                sb, bp, bs, sp, ss, evt.tx_hash, is_live_bool
            );
            b.clone()
        })
    });
    let _ = tokio::task::spawn_blocking(move || append_csv_row(row)).await;
}

/// Parse the status string to extract execution details for database storage
/// Returns (our_shares, our_price, our_usd, fill_pct, status_category)
fn parse_status_for_db(status: &str) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>, String) {
    // Status contains ANSI color codes - strip them for parsing
    let clean_status = strip_ansi_codes(status);

    // Try to parse "200 OK" successful trades: "200 OK [type] | filled/requested filled @ price | whale ..."
    if clean_status.starts_with("200 OK") {
        // Pattern: "200 OK [SCALED] | 5.00/5.00 filled @ 0.45 | whale 500.0 @ 0.44"
        if let Some((filled, requested, price)) = parse_fill_details(&clean_status) {
            let fill_pct = if requested > 0.0 { (filled / requested) * 100.0 } else { 0.0 };
            let our_usd = filled * price;
            return (Some(filled), Some(price), Some(our_usd), Some(fill_pct), "SUCCESS".to_string());
        }
        return (None, None, None, None, "SUCCESS".to_string());
    }

    // Check for various failure/skip statuses
    if clean_status.starts_with("SKIPPED") {
        return (None, None, None, None, clean_status.split_whitespace().next().unwrap_or("SKIPPED").to_string());
    }
    if clean_status.starts_with("RISK_BLOCKED") {
        return (None, None, None, None, "RISK_BLOCKED".to_string());
    }
    if clean_status.starts_with("EXEC_FAIL") || clean_status.starts_with("FAILED") {
        return (None, None, None, None, "FAILED".to_string());
    }
    if clean_status.starts_with("MOCK") {
        return (None, None, None, None, "MOCK".to_string());
    }
    if clean_status.contains("QUEUE_ERR") || clean_status.contains("WORKER") {
        return (None, None, None, None, "ERROR".to_string());
    }

    // Default: unknown status
    (None, None, None, None, clean_status.chars().take(20).collect())
}

/// Strip ANSI escape codes from a string
fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse fill details from status string
/// Returns (filled_shares, requested_shares, price)
fn parse_fill_details(status: &str) -> Option<(f64, f64, f64)> {
    // Pattern: "... | 5.00/5.00 filled @ 0.45 | ..."
    // Find the fill segment
    let parts: Vec<&str> = status.split('|').collect();
    for part in parts {
        let trimmed = part.trim();
        // Look for "X.XX/Y.YY filled @ Z.ZZ"
        if trimmed.contains("filled @") {
            // Split on "filled @"
            let fill_parts: Vec<&str> = trimmed.split("filled @").collect();
            if fill_parts.len() == 2 {
                // First part has "X.XX/Y.YY", second has "Z.ZZ"
                let ratio_part = fill_parts[0].trim();
                let price_str = fill_parts[1].trim();

                // Parse ratio: "5.00/5.00" or just the numbers
                if let Some(slash_idx) = ratio_part.rfind('/') {
                    let filled_str = ratio_part[..slash_idx].split_whitespace().last()?;
                    let requested_str = &ratio_part[slash_idx + 1..];

                    let filled: f64 = filled_str.parse().ok()?;
                    let requested: f64 = requested_str.parse().ok()?;
                    let price: f64 = price_str.split_whitespace().next()?.parse().ok()?;

                    return Some((filled, requested, price));
                }
            }
        }
    }
    None
}

/// Record an aggregated trade result to CSV and database
/// This is called by the background aggregation flush task after executing trades
async fn record_aggregated_trade(
    evt: &ParsedEvent,
    status: &str,
    is_live: Option<bool>,
    trade_tx: &Option<mpsc::UnboundedSender<TradeRecord>>,
    trader_manager: &Arc<Mutex<TraderManager>>,
    aggregation_count: usize,
) {
    // Parse status to extract execution details
    let (our_shares_opt, our_price_opt, our_usd_opt, fill_pct_opt, trade_status_str) = parse_status_for_db(status);

    // Determine TradeStatus enum from status string
    let trade_status = if trade_status_str == "SUCCESS" {
        if let Some(fill_pct) = fill_pct_opt {
            if fill_pct >= 90.0 {
                TradeStatus::Success
            } else {
                TradeStatus::Partial
            }
        } else {
            TradeStatus::Success
        }
    } else if trade_status_str.starts_with("SKIPPED") {
        TradeStatus::Skipped
    } else {
        TradeStatus::Failed
    };

    // Record trade in trader manager
    let usd_amount = our_usd_opt.unwrap_or(0.0);
    {
        let mut manager = trader_manager.lock().await;
        manager.record_trade(&evt.trader_address, usd_amount, trade_status);
    }

    // Record trade to database if persistence is enabled
    if let Some(tx) = trade_tx {
        let record = TradeRecord {
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            block_number: evt.block_number,
            tx_hash: evt.tx_hash.clone(),
            trader_address: evt.trader_address.clone(),
            token_id: evt.order.clob_token_id.to_string(),
            side: if evt.order.order_type.starts_with("BUY") { "BUY".to_string() } else { "SELL".to_string() },
            whale_shares: evt.order.shares,
            whale_price: evt.order.price_per_share,
            whale_usd: evt.order.usd_value,
            our_shares: our_shares_opt,
            our_price: our_price_opt,
            our_usd: our_usd_opt,
            fill_pct: fill_pct_opt,
            status: trade_status_str.clone(),
            latency_ms: None,
            is_live,
            aggregation_count: Some(aggregation_count as u32),
            aggregation_window_ms: None,
        };

        // Send to persistence worker (non-blocking)
        let _ = tx.send(record);
    }

    // Write to CSV
    let ts: DateTime<Utc> = Utc::now();
    let is_live_bool = is_live.unwrap_or(false);

    // Format status for CSV (include aggregation info)
    let csv_status = if aggregation_count > 1 {
        format!("AGG_FLUSH({}) {}", aggregation_count, status)
    } else {
        format!("AGG_BYPASS {}", status)
    };

    let row = CSV_BUF.with(|buf| {
        SANITIZE_BUF.with(|sbuf| {
            let mut b = buf.borrow_mut();
            let mut sb = sbuf.borrow_mut();
            sanitize_csv(&csv_status, &mut sb);
            b.clear();
            let _ = write!(b,
                "{},{},{},{:.2},{:.6},{:.4},{},{},N/A,N/A,N/A,N/A,{},{}",
                ts.format("%Y-%m-%d %H:%M:%S%.3f"),
                evt.block_number, evt.order.clob_token_id, evt.order.usd_value,
                evt.order.shares, evt.order.price_per_share, evt.order.order_type,
                sb, evt.tx_hash, is_live_bool
            );
            b.clone()
        })
    });
    let _ = tokio::task::spawn_blocking(move || append_csv_row(row)).await;
}

// ============================================================================
// Resubmitter Worker (handles FAK failures with price escalation)
// ============================================================================

async fn resubmit_worker(
    mut rx: mpsc::UnboundedReceiver<ResubmitRequest>,
    client: Arc<RustClobClient>,
    creds: Arc<PreparedCreds>,
) {
    println!("🔄 Resubmitter worker started");

    while let Some(req) = rx.recv().await {
        let max_attempts = get_max_resubmit_attempts(req.whale_shares);
        let is_last_attempt = req.attempt >= max_attempts;

        // Calculate increment: chase only if should_increment_price returns true
        let increment = if should_increment_price(req.whale_shares, req.attempt) {
            RESUBMIT_PRICE_INCREMENT
        } else {
            0.0  // Flat retry
        };
        let new_price = if req.side_is_buy {
            (req.failed_price + increment).min(0.99)
        } else {
            (req.failed_price - increment).max(0.01)
        };

        // Check if we've exceeded max buffer (skip check for GTD - last attempt always goes through)
        if !is_last_attempt && req.side_is_buy && new_price > req.max_price {
            let fill_pct = if req.original_size > 0.0 { (req.cumulative_filled / req.original_size) * 100.0 } else { 0.0 };
            println!(
                "🔄 Resubmit ABORT: attempt {} price {:.2} > max {:.2} | filled {:.2}/{:.2} ({:.0}%)",
                req.attempt, new_price, req.max_price, req.cumulative_filled, req.original_size, fill_pct
            );
            continue;
        }

        let client_clone = Arc::clone(&client);
        let creds_clone = Arc::clone(&creds);
        let token_id = req.token_id.clone();
        let size = req.size;
        let attempt = req.attempt;
        let whale_price = req.whale_price;
        let max_price = req.max_price;
        let is_live = req.is_live;

        // Submit order: FAK for early attempts, GTD with expiry for last attempt
        let result = tokio::task::spawn_blocking(move || {
            submit_resubmit_order_sync(&client_clone, &creds_clone, &token_id, new_price, size, is_live, is_last_attempt, max_price)
        }).await;

        match result {
            Ok(Ok((true, _, filled_this_attempt))) => {
                if is_last_attempt {
                    // GTD order placed on book - we don't know fill amount yet
                    println!(
                        "\x1b[32m🔄 Resubmit GTD SUBMITTED: attempt {} @ ≤{:.2} | size {:.2} | prior filled {:.2}/{:.2}\x1b[0m",
                        attempt, max_price, size, req.cumulative_filled, req.original_size
                    );
                } else {
                    // FAK order - check if partial fill
                    let total_filled = req.cumulative_filled + filled_this_attempt;
                    let fill_pct = if req.original_size > 0.0 { (total_filled / req.original_size) * 100.0 } else { 0.0 };
                    let remaining = size - filled_this_attempt;

                    // If partial fill, continue with remaining size
                    if remaining > 1.0 && filled_this_attempt > 0.0 {
                        println!(
                            "\x1b[33m🔄 Resubmit PARTIAL: attempt {} @ {:.2} | filled {:.2}/{:.2} ({:.0}%) | remaining {:.2}\x1b[0m",
                            attempt, new_price, total_filled, req.original_size, fill_pct, remaining
                        );
                        let next_req = ResubmitRequest {
                            token_id: req.token_id,
                            whale_price,
                            failed_price: new_price,
                            size: remaining,
                            whale_shares: req.whale_shares,
                            side_is_buy: req.side_is_buy,
                            attempt: attempt + 1,
                            max_price,
                            cumulative_filled: total_filled,
                            original_size: req.original_size,
                            is_live: req.is_live,
                        };
                        let _ = process_resubmit_chain(&client, &creds, next_req).await;
                    } else {
                        println!(
                            "\x1b[32m🔄 Resubmit SUCCESS: attempt {} @ {:.2} | filled {:.2}/{:.2} ({:.0}%)\x1b[0m",
                            attempt, new_price, total_filled, req.original_size, fill_pct
                        );
                    }
                }
            }
            Ok(Ok((false, body, filled_this_attempt))) => {
                if attempt < max_attempts {
                    // Re-queue with updated price
                    let next_req = ResubmitRequest {
                        token_id: req.token_id,
                        whale_price,
                        failed_price: new_price,
                        size: req.size,
                        whale_shares: req.whale_shares,
                        side_is_buy: req.side_is_buy,
                        attempt: attempt + 1,
                        max_price,
                        cumulative_filled: req.cumulative_filled + filled_this_attempt,
                        original_size: req.original_size,
                        is_live: req.is_live,
                    };
                    let next_increment = if should_increment_price(req.whale_shares, attempt + 1) {
                        RESUBMIT_PRICE_INCREMENT
                    } else {
                        0.0
                    };
                    println!(
                        "🔄 Resubmit attempt {} failed (FAK), retrying @ {:.2} (max: {})",
                        attempt, new_price + next_increment, max_attempts
                    );
                    if req.whale_shares < 1000.0 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    let _ = process_resubmit_chain(
                        &client,
                        &creds,
                        next_req,
                    ).await;
                } else {
                    let total_filled = req.cumulative_filled + filled_this_attempt;
                    let fill_pct = if req.original_size > 0.0 { (total_filled / req.original_size) * 100.0 } else { 0.0 };
                    let error_msg = if DEBUG_FULL_ERRORS { body.clone() } else { body.chars().take(80).collect::<String>() };
                    println!(
                        "🔄 Resubmit FAILED: attempt {} @ {:.2} | filled {:.2}/{:.2} ({:.0}%) | {}",
                        attempt, new_price, total_filled, req.original_size, fill_pct, error_msg
                    );
                }
            }
            Ok(Err(e)) => {
                let fill_pct = if req.original_size > 0.0 { (req.cumulative_filled / req.original_size) * 100.0 } else { 0.0 };
                println!(
                    "🔄 Resubmit ERROR: attempt {} | filled {:.2}/{:.2} ({:.0}%) | {}",
                    attempt, req.cumulative_filled, req.original_size, fill_pct, e
                );
            }
            Err(e) => {
                let fill_pct = if req.original_size > 0.0 { (req.cumulative_filled / req.original_size) * 100.0 } else { 0.0 };
                println!(
                    "🔄 Resubmit TASK ERROR: filled {:.2}/{:.2} ({:.0}%) | {}",
                    req.cumulative_filled, req.original_size, fill_pct, e
                );
            }
        }
    }
}

async fn process_resubmit_chain(
    client: &Arc<RustClobClient>,
    creds: &Arc<PreparedCreds>,
    mut req: ResubmitRequest,
) {
    let max_attempts = get_max_resubmit_attempts(req.whale_shares);

    while req.attempt <= max_attempts {
        let is_last_attempt = req.attempt >= max_attempts;

        // Calculate increment: chase only if should_increment_price returns true
        let increment = if should_increment_price(req.whale_shares, req.attempt) {
            RESUBMIT_PRICE_INCREMENT
        } else {
            0.0  // Flat retry
        };
        let new_price = if req.side_is_buy {
            (req.failed_price + increment).min(0.99)
        } else {
            (req.failed_price - increment).max(0.01)
        };

        // Check if we've exceeded max buffer (skip check for GTD - last attempt always goes through)
        if !is_last_attempt && req.side_is_buy && new_price > req.max_price {
            let fill_pct = if req.original_size > 0.0 { (req.cumulative_filled / req.original_size) * 100.0 } else { 0.0 };
            println!(
                "🔄 Resubmit chain ABORT: attempt {} price {:.2} > max {:.2} | filled {:.2}/{:.2} ({:.0}%)",
                req.attempt, new_price, req.max_price, req.cumulative_filled, req.original_size, fill_pct
            );
            return;
        }

        let client_clone = Arc::clone(&client);
        let creds_clone = Arc::clone(&creds);
        let token_id = req.token_id.clone();
        let size = req.size;
        let attempt = req.attempt;
        let is_live = req.is_live;
        let max_price = req.max_price;

        // Submit order: FAK for early attempts, GTD with expiry for last attempt
        let result = tokio::task::spawn_blocking(move || {
            submit_resubmit_order_sync(&client_clone, &creds_clone, &token_id, new_price, size, is_live, is_last_attempt, max_price)
        }).await;

        match result {
            Ok(Ok((true, _, filled_this_attempt))) => {
                if is_last_attempt {
                    // GTD order placed on book - we don't know fill amount yet
                    println!(
                        "\x1b[32m🔄 Resubmit chain GTD SUBMITTED: attempt {} @ ≤{:.2} | size {:.2} | prior filled {:.2}/{:.2}\x1b[0m",
                        attempt, req.max_price, req.size, req.cumulative_filled, req.original_size
                    );
                    return;
                } else {
                    // FAK order - check if partial fill
                    let total_filled = req.cumulative_filled + filled_this_attempt;
                    let fill_pct = if req.original_size > 0.0 { (total_filled / req.original_size) * 100.0 } else { 0.0 };
                    let remaining = req.size - filled_this_attempt;

                    // If partial fill, continue with remaining size
                    if remaining > 1.0 && filled_this_attempt > 0.0 {
                        println!(
                            "\x1b[33m🔄 Resubmit chain PARTIAL: attempt {} @ {:.2} | filled {:.2}/{:.2} ({:.0}%) | remaining {:.2}\x1b[0m",
                            attempt, new_price, total_filled, req.original_size, fill_pct, remaining
                        );
                        req.cumulative_filled = total_filled;
                        req.size = remaining;
                        req.failed_price = new_price;
                        req.attempt += 1;
                        continue;
                    } else {
                        println!(
                            "\x1b[32m🔄 Resubmit chain SUCCESS: attempt {} @ {:.2} | filled {:.2}/{:.2} ({:.0}%)\x1b[0m",
                            attempt, new_price, total_filled, req.original_size, fill_pct
                        );
                        return;
                    }
                }
            }
            Ok(Ok((false, body, filled_this_attempt))) if body.contains("FAK") && attempt < max_attempts => {
                // FAK failed (no liquidity), retry with next attempt
                let next_attempt = attempt + 1;
                let next_is_last = next_attempt >= max_attempts;
                let next_type = if next_is_last { "GTD" } else { "FAK" };
                println!(
                    "🔄 Resubmit chain: attempt {} FAK no match, trying {} @ {:.2} (attempt {})",
                    attempt, next_type, new_price, next_attempt
                );
                req.cumulative_filled += filled_this_attempt;
                req.failed_price = new_price;
                req.attempt = next_attempt;
                // Small trades get 50ms delay to let orderbook refresh
                if req.whale_shares < 1000.0 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                continue;
            }
            Ok(Ok((false, body, filled_this_attempt))) => {
                let total_filled = req.cumulative_filled + filled_this_attempt;
                let fill_pct = if req.original_size > 0.0 { (total_filled / req.original_size) * 100.0 } else { 0.0 };
                let fill_color = get_fill_color(total_filled, req.original_size);
                let reset = "\x1b[0m";
                let error_msg = if DEBUG_FULL_ERRORS { body.clone() } else { body.chars().take(80).collect::<String>() };
                println!(
                    "🔄 Resubmit chain FAILED: attempt {}/{} @ {:.2} | {}filled {:.2}/{:.2} ({:.0}%){} | {}",
                    attempt, max_attempts, new_price, fill_color, total_filled, req.original_size, fill_pct, reset, error_msg
                );
                return;
            }
            Ok(Err(e)) => {
                let fill_pct = if req.original_size > 0.0 { (req.cumulative_filled / req.original_size) * 100.0 } else { 0.0 };
                let fill_color = get_fill_color(req.cumulative_filled, req.original_size);
                let reset = "\x1b[0m";
                println!(
                    "🔄 Resubmit chain ERROR: attempt {} | {}filled {:.2}/{:.2} ({:.0}%){} | {}",
                    attempt, fill_color, req.cumulative_filled, req.original_size, fill_pct, reset, e
                );
                return;
            }
            Err(e) => {
                let fill_pct = if req.original_size > 0.0 { (req.cumulative_filled / req.original_size) * 100.0 } else { 0.0 };
                let fill_color = get_fill_color(req.cumulative_filled, req.original_size);
                let reset = "\x1b[0m";
                println!(
                    "🔄 Resubmit chain TASK ERROR: {}filled {:.2}/{:.2} ({:.0}%){} | {}",
                    fill_color, req.cumulative_filled, req.original_size, fill_pct, reset, e
                );
                return;
            }
        }
    }
}

/// Returns (success, body_text, filled_shares)
fn submit_resubmit_order_sync(
    client: &RustClobClient,
    creds: &PreparedCreds,
    token_id: &str,
    price: f64,
    size: f64,
    is_live: bool,
    is_last_attempt: bool,
    max_price: f64,
) -> anyhow::Result<(bool, String, f64)> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut client = client.clone();

    // Only use GTD with expiry on the LAST attempt; earlier attempts use FAK
    let (expiration, order_type, final_price) = if is_last_attempt {
        let expiry_secs = get_gtd_expiry_secs(is_live);
        let expiry_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() + expiry_secs;

        // For GTD, try to cross the spread by using min(max_price, best_ask)
        let gtd_price = fetch_best_ask_sync(token_id)
            .map(|best_ask| {
                let crossed_price = best_ask.min(max_price);
                if crossed_price > price {
                    println!(
                        "🔄 GTD crossing spread: {:.2} -> {:.2} (best_ask={:.2}, max={:.2})",
                        price, crossed_price, best_ask, max_price
                    );
                }
                crossed_price
            })
            .unwrap_or(price); // Fall back to original price if book fetch fails

        (Some(expiry_timestamp.to_string()), "GTD", gtd_price)
    } else {
        (None, "FAK", price)
    };

    // Round to micro-units (6 decimals) then back to avoid floating-point truncation issues
    // e.g., 40.80 stored as 40.7999999... would truncate to 40799999 instead of 40800000
    let price_micro = (final_price * 1_000_000.0).round() as i64;
    let size_micro = (size * 1_000_000.0).round() as i64;
    let rounded_price = price_micro as f64 / 1_000_000.0;
    let rounded_size = size_micro as f64 / 1_000_000.0;

    let args = OrderArgs {
        token_id: token_id.to_string(),
        price: rounded_price,
        size: rounded_size,
        side: "BUY".into(),
        fee_rate_bps: None,
        nonce: Some(0),
        expiration,
        taker: None,
        order_type: Some(order_type.to_string()),
    };

    let signed = client.create_order(args)?;
    let body = signed.post_body(&creds.api_key, order_type);
    let resp = client.post_order_fast(body, creds)?;

    let status = resp.status();
    let body_text = resp.text().unwrap_or_default();

    // Parse filled amount from successful responses
    // GTD orders return taking_amount=0 since they're placed on book, not immediately filled
    // For GTD, return 0 - caller handles GTD success messaging separately
    let filled_shares = if status.is_success() && order_type == "FAK" {
        serde_json::from_str::<OrderResponse>(&body_text)
            .ok()
            .and_then(|r| r.taking_amount.parse::<f64>().ok())
            .unwrap_or(0.0)
    } else {
        0.0
    };

    Ok((status.is_success(), body_text, filled_shares))
}

/// Fetch the best ask price from the order book (blocking/sync version)
/// Returns None if the book fetch fails or no asks are available
fn fetch_best_ask_sync(token_id: &str) -> Option<f64> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;

    let url = format!("{}/book?token_id={}", CLOB_API_BASE, token_id);
    let resp = client.get(&url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let val: Value = resp.json().ok()?;
    let asks = val.get("asks")?.as_array()?;

    // Find the best (lowest) ask price
    asks.iter()
        .filter_map(|entry| {
            entry.get("price")?.as_str()?.parse::<f64>().ok()
        })
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

async fn fetch_is_live(token_id: &str, client: &reqwest::Client) -> Option<bool> {
    // Fetch market info to get slug
    let market_url = format!("{}/markets?clob_token_ids={}", GAMMA_API_BASE, token_id);
    let resp = client.get(&market_url).timeout(Duration::from_secs(2)).send().await.ok()?;
    let val: Value = resp.json().await.ok()?;
    let slug = val.get(0)?.get("slug")?.as_str()?.to_string();

    // Fetch live status from events API
    let event_url = format!("{}/events/slug/{}", GAMMA_API_BASE, slug);
    let resp = client.get(&event_url).timeout(Duration::from_secs(2)).send().await.ok()?;
    let val: Value = resp.json().await.ok()?;

    Some(val["live"].as_bool().unwrap_or(false))
}

async fn fetch_best_book(token_id: &str, order_type: &str, client: &reqwest::Client) -> Option<((String, String), (String, String))> {
    let url = format!("{}/book?token_id={}", CLOB_API_BASE, token_id);
    let resp = client.get(&url).timeout(BOOK_REQ_TIMEOUT).send().await.ok()?;
    if !resp.status().is_success() { return None; }
    
    let val: Value = resp.json().await.ok()?;
    let key = if order_type.starts_with("BUY") { "asks" } else { "bids" };
    let entries = val.get(key)?.as_array()?;

    let is_buy = order_type.starts_with("BUY");
    
    let (best, second): (Option<(&Value, f64)>, Option<(&Value, f64)>) = 
        entries.iter().fold((None, None), |(best, second), entry| {
            let price: f64 = match entry.get("price").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) {
                Some(p) => p,
                None => return (best, second),
            };
            
            let better = |candidate: f64, current: f64| {
                if is_buy { candidate < current } else { candidate > current }
            };
            
            match best {
                Some((_, bp)) if better(price, bp) => (Some((entry, price)), best),
                Some((_, _bp)) => {
                    let new_second = match second {
                        Some((_, sp)) if better(price, sp) => Some((entry, price)),
                        None => Some((entry, price)),
                        _ => second,
                    };
                    (best, new_second)
                }
                None => (Some((entry, price)), second),
            }
        });

    let b = best?.0;
    let best_price = b.get("price")?.to_string();
    let best_size = b.get("size")?.to_string();
    
    let (second_price, second_size) = second
        .and_then(|(e, _)| {
            let p = e.get("price")?.to_string();
            let s = e.get("size")?.to_string();
            Some((p, s))
        })
        .unwrap_or_else(|| ("N/A".into(), "N/A".into()));
    
    Some(((best_price, best_size), (second_price, second_size)))
}

// ============================================================================
// Event Parsing
// ============================================================================

fn parse_event(message: String, traders: Option<&TradersConfig>) -> Option<ParsedEvent> {
    let msg: WsMessage = serde_json::from_str(&message).ok()?;
    let result = msg.params?.result?;

    // just to double check!
    if result.topics.len() < 3 { return None; }

    // Extract trader address from topics[2]
    // Format: 0x000000000000000000000000{40-char-address}
    let trader_topic = result.topics.get(2)?;
    let trader_address = extract_address_from_topic(trader_topic)?;

    // Look up trader in config (if provided)
    // Returns (label, min_shares) tuple
    let (trader_label, trader_min_shares) = if let Some(traders_cfg) = traders {
        // Try to find trader by topic hex (case-insensitive for robustness)
        // WebSocket may return different case than our stored topics
        let topic_lower = trader_topic.to_lowercase();
        if let Some(trader_cfg) = traders_cfg.get_by_topic(&topic_lower) {
            if !trader_cfg.enabled {
                return None; // Skip disabled traders
            }
            (trader_cfg.label.clone(), trader_cfg.min_shares)
        } else {
            // Debug: Log when we receive an event but don't match a trader
            // This helps diagnose subscription/filtering issues
            if std::env::var("DEBUG_EVENTS").is_ok() {
                eprintln!("🔍 Event from unknown trader: {} (addr: {})", trader_topic, trader_address);
            }
            // Trader not in our config - skip
            return None;
        }
    } else {
        // No traders config provided (legacy mode or tests)
        // Fall back to checking TARGET_TOPIC_HEX
        let has_target = trader_topic.eq_ignore_ascii_case(TARGET_TOPIC_HEX.as_str());
        if !has_target { return None; }
        // Legacy mode uses global MIN_WHALE_SHARES_TO_COPY
        (String::new(), MIN_WHALE_SHARES_TO_COPY)
    };

    let hex_data = &result.data;
    if hex_data.len() < 2 + 64 * 4 { return None; }

    let (maker_id, maker_bytes) = parse_u256_hex_slice_with_bytes(hex_data, 2, 66)?;
    let (taker_id, taker_bytes) = parse_u256_hex_slice_with_bytes(hex_data, 66, 130)?;

    let (clob_id, token_bytes, maker_amt, taker_amt, base_type) =
        if maker_id.is_zero() && !taker_id.is_zero() {
            let m = parse_u256_hex_slice(hex_data, 130, 194)?;
            let t = parse_u256_hex_slice(hex_data, 194, 258)?;
            (taker_id, taker_bytes, m, t, "BUY")
        } else if taker_id.is_zero() && !maker_id.is_zero() {
            let m = parse_u256_hex_slice(hex_data, 130, 194)?;
            let t = parse_u256_hex_slice(hex_data, 194, 258)?;
            (maker_id, maker_bytes, m, t, "SELL")
        } else {
            return None;
        };

    let shares = if base_type == "BUY" { u256_to_f64(&taker_amt)? } else { u256_to_f64(&maker_amt)? } / 1e6;
    if shares <= 0.0 { return None; }
    
    let usd = if base_type == "BUY" { u256_to_f64(&maker_amt)? } else { u256_to_f64(&taker_amt)? } / 1e6;
    let price = usd / shares;
    
    let mut order_type = base_type.to_string();
    if result.topics[0].eq_ignore_ascii_case(ORDERS_FILLED_EVENT_SIGNATURE) {
        order_type.push_str("_FILL");
    }

    Some(ParsedEvent {
        block_number: result.block_number.as_deref()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or_default(),
        tx_hash: result.transaction_hash.unwrap_or_default(),
        trader_address,
        trader_label,
        trader_min_shares,
        order: OrderInfo {
            order_type,
            clob_token_id: u256_to_dec_cached(&token_bytes, &clob_id),
            usd_value: usd,
            shares,
            price_per_share: price,
        },
    })
}

/// Extract 40-character address from a topic hex string
/// Format: 0x000000000000000000000000{40-char-address}
/// Returns normalized lowercase address without 0x prefix
fn extract_address_from_topic(topic: &str) -> Option<String> {
    let topic_clean = topic.trim().strip_prefix("0x").unwrap_or(topic.trim());
    if topic_clean.len() != 64 {
        return None;
    }
    // Last 40 characters are the address (first 24 are padding zeros)
    let address = &topic_clean[24..64];
    Some(address.to_lowercase())
}

// ============================================================================
// Hex Parsing Helpers
// ============================================================================

#[inline]
fn parse_u256_hex_slice_with_bytes(full: &str, start: usize, end: usize) -> Option<(U256, [u8; 32])> {
    let slice = full.get(start..end)?;
    let clean = slice.strip_prefix("0x").unwrap_or(slice);
    if clean.len() > 64 { return None; }

    let mut hex_buf = [b'0'; 64];
    hex_buf[64 - clean.len()..].copy_from_slice(clean.as_bytes());

    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = hex_nibble(hex_buf[i * 2])?;
        let lo = hex_nibble(hex_buf[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some((U256::from_be_slice(&out), out))
}

#[inline]
fn parse_u256_hex_slice(full: &str, start: usize, end: usize) -> Option<U256> {
    parse_u256_hex_slice_with_bytes(full, start, end).map(|(v, _)| v)
}

fn u256_to_dec_cached(bytes: &[u8; 32], val: &U256) -> Arc<str> {
    TOKEN_ID_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(s) = cache.get(bytes) { return Arc::clone(s); }  // Cheap Arc clone
        let s: Arc<str> = val.to_string().into();
        cache.insert(*bytes, Arc::clone(&s));
        s
    })
}

fn u256_to_f64(v: &U256) -> Option<f64> {
    if v.bit_len() <= 64 { Some(v.as_limbs()[0] as f64) }
    else { v.to_string().parse().ok() }
}

// Hex nibble lookup table - 2-3x faster than branching
const HEX_NIBBLE_LUT: [u8; 256] = {
    let mut lut = [255u8; 256];
    let mut i = b'0';
    while i <= b'9' {
        lut[i as usize] = i - b'0';
        i += 1;
    }
    let mut i = b'a';
    while i <= b'f' {
        lut[i as usize] = i - b'a' + 10;
        i += 1;
    }
    let mut i = b'A';
    while i <= b'F' {
        lut[i as usize] = i - b'A' + 10;
        i += 1;
    }
    lut
};

#[inline(always)]
fn hex_nibble(b: u8) -> Option<u8> {
    let val = HEX_NIBBLE_LUT[b as usize];
    if val == 255 { None } else { Some(val) }
}

// ============================================================================
// CSV Helpers
// ============================================================================

fn ensure_csv() -> Result<()> {
    if !Path::new(CSV_FILE).exists() {
        let mut f = File::create(CSV_FILE)?;
        writeln!(f, "timestamp,block,clob_asset_id,usd_value,shares,price_per_share,direction,order_status,best_price,best_size,second_price,second_size,tx_hash,is_live")?;
    }
    Ok(())
}

fn append_csv_row(row: String) {
    if let Ok(mut f) = OpenOptions::new().append(true).create(true).open(CSV_FILE) {
        let _ = writeln!(f, "{}", row);
    }
}

#[inline]
fn sanitize_csv(value: &str, out: &mut String) {
    out.clear();
    if !value.bytes().any(|b| b == b',' || b == b'\n' || b == b'\r') {
        out.push_str(value);
        return;
    }
    out.reserve(value.len());
    for &b in value.as_bytes() {
        out.push(match b { b',' => ';', b'\n' | b'\r' => ' ', _ => b as char });
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test extracting trader address from topics[2]
    /// Topics[2] format: 0x000000000000000000000000{40-char-address}
    #[test]
    fn test_extract_trader_address_from_topic() {
        // Ensure env var is set (for legacy TARGET_TOPIC_HEX fallback)
        unsafe {
            std::env::set_var("TARGET_WHALE_ADDRESS", "def456def456789012345678901234567890def4");
        }

        // Use a consistent test address
        let trader_addr = "def456def456789012345678901234567890def4";
        let topic_hex = format!("0x000000000000000000000000{}", trader_addr);

        let message = serde_json::json!({
            "params": {
                "result": {
                    "topics": [
                        ORDERS_FILLED_EVENT_SIGNATURE,
                        "0x0000000000000000000000000000000000000000000000000000000000000000",
                        topic_hex
                    ],
                    "data": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000123456000000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000000007a120",
                    "blockNumber": "0x1234",
                    "transactionHash": "0xabcdef"
                }
            }
        }).to_string();

        let event = parse_event(message, None);
        assert!(event.is_some());
        let event = event.unwrap();

        // Trader address should be extracted and normalized (lowercase, no 0x)
        assert_eq!(event.trader_address, trader_addr);
    }

    /// Test that trader address is normalized to lowercase
    #[test]
    fn test_trader_address_normalized_to_lowercase() {
        // Ensure env var is set (for legacy TARGET_TOPIC_HEX fallback)
        unsafe {
            std::env::set_var("TARGET_WHALE_ADDRESS", "def456def456789012345678901234567890def4");
        }

        // Use same test address but uppercase
        let trader_addr_upper = "DEF456DEF456789012345678901234567890DEF4";
        let topic_hex = format!("0x000000000000000000000000{}", trader_addr_upper);

        let message = serde_json::json!({
            "params": {
                "result": {
                    "topics": [
                        ORDERS_FILLED_EVENT_SIGNATURE,
                        "0x0000000000000000000000000000000000000000000000000000000000000000",
                        topic_hex
                    ],
                    "data": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000123456000000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000000007a120",
                    "blockNumber": "0x1234",
                    "transactionHash": "0xabcdef"
                }
            }
        }).to_string();

        let event = parse_event(message, None);
        assert!(event.is_some());
        let event = event.unwrap();

        // Should be normalized to lowercase
        assert_eq!(event.trader_address, trader_addr_upper.to_lowercase());
    }

    /// Test that trader_label is empty (will be populated in later increment)
    #[test]
    fn test_trader_label_initially_empty() {
        // Ensure env var is set (for legacy TARGET_TOPIC_HEX fallback)
        unsafe {
            std::env::set_var("TARGET_WHALE_ADDRESS", "def456def456789012345678901234567890def4");
        }

        let trader_addr = "def456def456789012345678901234567890def4";
        let topic_hex = format!("0x000000000000000000000000{}", trader_addr);

        let message = serde_json::json!({
            "params": {
                "result": {
                    "topics": [
                        ORDERS_FILLED_EVENT_SIGNATURE,
                        "0x0000000000000000000000000000000000000000000000000000000000000000",
                        topic_hex
                    ],
                    "data": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000123456000000000000000000000000000000000000000000000000000000000000f4240000000000000000000000000000000000000000000000000000000000007a120",
                    "blockNumber": "0x1234",
                    "transactionHash": "0xabcdef"
                }
            }
        }).to_string();

        let event = parse_event(message, None);
        assert!(event.is_some());
        let event = event.unwrap();

        // Label should be empty for now
        assert_eq!(event.trader_label, "");
    }

    // Test extract_address_from_topic helper function
    #[test]
    fn test_extract_address_from_topic_valid() {
        let topic = "0x000000000000000000000000abc123def456789012345678901234567890abcd";
        let result = extract_address_from_topic(topic);
        assert_eq!(result, Some("abc123def456789012345678901234567890abcd".to_string()));
    }

    #[test]
    fn test_extract_address_from_topic_uppercase() {
        let topic = "0x000000000000000000000000ABC123DEF456789012345678901234567890ABCD";
        let result = extract_address_from_topic(topic);
        assert_eq!(result, Some("abc123def456789012345678901234567890abcd".to_string()));
    }

    #[test]
    fn test_extract_address_from_topic_no_prefix() {
        let topic = "000000000000000000000000def456def456789012345678901234567890def4";
        let result = extract_address_from_topic(topic);
        assert_eq!(result, Some("def456def456789012345678901234567890def4".to_string()));
    }

    #[test]
    fn test_extract_address_from_topic_invalid_length() {
        let topic = "0x0000000000000000000000abc123";  // Too short
        let result = extract_address_from_topic(topic);
        assert_eq!(result, None);
    }

    // Test subscription message building
    #[test]
    fn test_build_subscription_message_single_topic() {
        let topics = vec![
            "0x000000000000000000000000abc123def456789012345678901234567890abcd".to_string()
        ];

        let msg = build_subscription_message(topics);
        let parsed: Value = serde_json::from_str(&msg).unwrap();

        // Verify structure
        assert_eq!(parsed["method"], "eth_subscribe");
        assert_eq!(parsed["params"][0], "logs");

        // Verify topics array has trader filter
        let topics_array = &parsed["params"][1]["topics"];
        assert!(topics_array.is_array());
        assert_eq!(topics_array[2][0], "0x000000000000000000000000abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_build_subscription_message_multiple_topics() {
        let topics = vec![
            "0x000000000000000000000000abc123def456789012345678901234567890abcd".to_string(),
            "0x000000000000000000000000def456def456789012345678901234567890def4".to_string(),
        ];

        let msg = build_subscription_message(topics);
        let parsed: Value = serde_json::from_str(&msg).unwrap();

        // Verify topics array has both traders
        let topics_array = &parsed["params"][1]["topics"];
        assert_eq!(topics_array[2].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_build_subscription_message_many_topics_uses_null_filter() {
        // Create 15 topics (> 10 threshold)
        let topics: Vec<String> = (0..15)
            .map(|i| format!("0x{:064x}", i))
            .collect();

        let msg = build_subscription_message(topics);
        let parsed: Value = serde_json::from_str(&msg).unwrap();

        // Verify topics[2] is null (client-side filtering)
        let topics_array = &parsed["params"][1]["topics"];
        assert_eq!(topics_array[2], Value::Null);
    }

    // -------------------------------------------------------------------------
    // Portfolio-based bet size cap tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_calculate_safe_size_no_cap() {
        // Without a cap, should return scaled size
        // 10000 shares * 0.02 (SCALING_RATIO) * 1.0 (multiplier) = 200 shares
        let (shares, size_type) = calculate_safe_size(10000.0, 0.50, 1.0, None);
        assert!((shares - 200.0).abs() < 0.01);
        assert!(matches!(size_type, SizeType::Scaled));
    }

    #[test]
    fn test_calculate_safe_size_with_cap_not_exceeded() {
        // Cap is higher than calculated size, should return scaled size
        // 5000 shares * 0.02 * 1.0 = 100 shares, cap = 200 shares
        let (shares, size_type) = calculate_safe_size(5000.0, 0.50, 1.0, Some(200.0));
        assert!((shares - 100.0).abs() < 0.01);
        assert!(matches!(size_type, SizeType::Scaled)); // Not capped
    }

    #[test]
    fn test_calculate_safe_size_with_cap_exceeded() {
        // Cap is lower than calculated size, should return capped size
        // 10000 shares * 0.02 * 1.0 = 200 shares, cap = 50 shares
        let (shares, size_type) = calculate_safe_size(10000.0, 0.50, 1.0, Some(50.0));
        assert!((shares - 50.0).abs() < 0.01);
        assert!(matches!(size_type, SizeType::Capped));
    }

    #[test]
    fn test_calculate_safe_size_with_multiplier_and_cap() {
        // 8000 shares * 0.02 * 1.25 (large trade multiplier) = 200 shares
        // Cap = 100 shares, should cap
        let (shares, size_type) = calculate_safe_size(8000.0, 0.50, 1.25, Some(100.0));
        assert!((shares - 100.0).abs() < 0.01);
        assert!(matches!(size_type, SizeType::Capped));
    }

    #[test]
    fn test_calculate_safe_size_cap_at_exactly_scaled() {
        // Cap equals scaled size exactly, should NOT show as capped
        // 5000 shares * 0.02 * 1.0 = 100 shares, cap = 100 shares
        let (shares, size_type) = calculate_safe_size(5000.0, 0.50, 1.0, Some(100.0));
        assert!((shares - 100.0).abs() < 0.01);
        assert!(matches!(size_type, SizeType::Scaled)); // Not capped because size == cap
    }

    #[test]
    fn test_calculate_safe_size_cap_zero_disables() {
        // Cap of 0 should effectively disable capping (treated as no cap)
        let (shares, _size_type) = calculate_safe_size(10000.0, 0.50, 1.0, Some(0.0));
        // With cap=0, the condition `max > 0.0` fails, so no capping applied
        assert!((shares - 200.0).abs() < 0.01);
    }
}