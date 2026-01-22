// trade_history.rs - CLI tool for querying trade history from SQLite database
//
// Usage:
//   cargo run --bin trade_history                     # Show recent trades
//   cargo run --bin trade_history -- --db test.db     # Use custom database
//   cargo run --bin trade_history -- --limit 100      # Show more trades
//   cargo run --bin trade_history -- --refresh        # Enrich with live market data

use anyhow::Result;
use clap::Parser;
use pm_whale_follower::persistence::TradeStore;
use std::collections::HashMap;

#[derive(Parser)]
#[command(name = "trade_history")]
#[command(about = "Query trade history from database")]
struct Args {
    /// Database path
    #[arg(long, default_value = "trades.db")]
    db: String,

    /// Maximum number of trades to show
    #[arg(long, default_value = "50")]
    limit: usize,

    /// Filter by trader address
    #[arg(long)]
    trader: Option<String>,

    /// Filter by token ID
    #[arg(long)]
    token: Option<String>,

    /// Filter by status (SUCCESS, FAILED, PARTIAL, SKIPPED)
    #[arg(long)]
    status: Option<String>,

    /// Show trades since timestamp (Unix seconds)
    #[arg(long)]
    since: Option<i64>,

    /// Output format: table, csv, json
    #[arg(long, default_value = "table")]
    format: String,

    /// Refresh trade data with live market information
    #[arg(long)]
    refresh: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Open database read-only
    let store = TradeStore::new(&args.db)?;

    // Fetch trades
    let mut trades = store.get_recent_trades(args.limit)?;

    // Apply filters
    trades = apply_filters(
        trades,
        args.trader.as_deref(),
        args.token.as_deref(),
        args.status.as_deref(),
        args.since,
    );

    // Conditionally collect enriched data if refresh flag is set
    // This runs blocking HTTP requests, so we need to run it in a blocking context
    let enriched_data = if args.refresh {
        let trades_clone = trades.clone();
        tokio::task::spawn_blocking(move || {
            maybe_collect_enriched_data(true, &trades_clone)
        })
        .await?
    } else {
        None
    };

    // Display trades in requested format
    match args.format.to_lowercase().as_str() {
        "csv" => print_csv(&trades),
        "json" => print_json(&trades)?,
        _ => print_table(&trades, enriched_data.as_ref()),
    }

    Ok(())
}

/// Apply filters to trades
fn apply_filters(
    trades: Vec<pm_whale_follower::persistence::TradeRecord>,
    trader: Option<&str>,
    token: Option<&str>,
    status: Option<&str>,
    since: Option<i64>,
) -> Vec<pm_whale_follower::persistence::TradeRecord> {
    trades
        .into_iter()
        .filter(|t| {
            // Filter by trader address
            if let Some(trader_filter) = trader {
                if !t.trader_address.contains(trader_filter) {
                    return false;
                }
            }
            // Filter by token ID
            if let Some(token_filter) = token {
                if !t.token_id.contains(token_filter) {
                    return false;
                }
            }
            // Filter by status
            if let Some(status_filter) = status {
                if !t.status.eq_ignore_ascii_case(status_filter) {
                    return false;
                }
            }
            // Filter by timestamp
            if let Some(since_ts) = since {
                if t.timestamp_ms < since_ts * 1000 {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Enriched data for a token from live market information
#[derive(Debug, Clone)]
struct EnrichedData {
    market_title: Option<String>,
    outcome: Option<String>,
    current_bid: Option<f64>,
    current_ask: Option<f64>,
}

/// Extract unique token IDs from a list of trades
fn extract_unique_token_ids(trades: &[pm_whale_follower::persistence::TradeRecord]) -> Vec<String> {
    use std::collections::HashSet;

    trades
        .iter()
        .map(|t| t.token_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

/// Conditionally collect enriched data if refresh flag is set
fn maybe_collect_enriched_data(
    refresh: bool,
    trades: &[pm_whale_follower::persistence::TradeRecord],
) -> Option<HashMap<String, EnrichedData>> {
    if !refresh {
        return None;
    }

    let token_ids = extract_unique_token_ids(trades);
    Some(collect_enriched_data(&token_ids))
}

/// Collect enriched data for a list of token IDs
/// Fetches market info and current prices, handling errors gracefully
fn collect_enriched_data(token_ids: &[String]) -> HashMap<String, EnrichedData> {
    use pm_whale_follower::market_info::MarketInfo;
    use pm_whale_follower::prices::PriceCache;

    let mut enriched = HashMap::new();

    if token_ids.is_empty() {
        return enriched;
    }

    let market_info = MarketInfo::new();
    let mut price_cache = PriceCache::new(30);

    for token_id in token_ids {
        // Fetch market info
        let (market_title, outcome) = match market_info.fetch(token_id) {
            Ok(Some(metadata)) => (Some(metadata.title), Some(metadata.outcome)),
            _ => (None, None),
        };

        // Fetch current prices
        let (current_bid, current_ask) = match price_cache.get_or_fetch_price(token_id) {
            Ok(price_info) => (Some(price_info.bid_price), Some(price_info.ask_price)),
            Err(_) => (None, None),
        };

        enriched.insert(
            token_id.clone(),
            EnrichedData {
                market_title,
                outcome,
                current_bid,
                current_ask,
            },
        );
    }

    enriched
}

/// Print trades in a formatted table
fn print_table(
    trades: &[pm_whale_follower::persistence::TradeRecord],
    enriched_data: Option<&HashMap<String, EnrichedData>>,
) {
    println!("\n=== TRADE HISTORY ===\n");

    if trades.is_empty() {
        println!("No trades found.");
        return;
    }

    // Choose header based on whether we have enriched data
    if enriched_data.is_some() {
        println!(
            "{:<20} {:<8} {:<40} {:>12} {:>10} {:>10} {:>10} {:<10}",
            "Timestamp", "Side", "Market", "Whale $", "Our $", "Current", "P&L %", "Status"
        );
        println!("{}", "-".repeat(130));
    } else {
        println!(
            "{:<20} {:<8} {:<15} {:>12} {:>10} {:>10} {:<10}",
            "Timestamp", "Side", "Token", "Whale $", "Our $", "Fill %", "Status"
        );
        println!("{}", "-".repeat(100));
    }

    // Print each trade
    for trade in trades {
        let timestamp = format_timestamp(trade.timestamp_ms);
        let whale_usd = format!("${:.2}", trade.whale_usd);
        let our_usd = trade.our_usd
            .map(|v| format!("${:.2}", v))
            .unwrap_or_else(|| "N/A".to_string());

        if let Some(enriched_map) = enriched_data {
            // Enhanced display with market info and current prices
            let market_display = if let Some(enriched) = enriched_map.get(&trade.token_id) {
                format_market_display(enriched)
            } else {
                truncate_token_id(&trade.token_id)
            };

            let current_price = if let Some(enriched) = enriched_map.get(&trade.token_id) {
                format_current_price(&trade.side, enriched)
            } else {
                "N/A".to_string()
            };

            let pnl = if let Some(enriched) = enriched_map.get(&trade.token_id) {
                calculate_pnl(&trade.side, trade.our_price, enriched)
            } else {
                "N/A".to_string()
            };

            println!(
                "{:<20} {:<8} {:<40} {:>12} {:>10} {:>10} {:>10} {:<10}",
                timestamp,
                trade.side,
                market_display,
                whale_usd,
                our_usd,
                current_price,
                pnl,
                trade.status
            );
        } else {
            // Original display format
            let token = truncate_token_id(&trade.token_id);
            let fill_pct = trade.fill_pct
                .map(|v| format!("{:.1}%", v))
                .unwrap_or_else(|| "N/A".to_string());

            println!(
                "{:<20} {:<8} {:<15} {:>12} {:>10} {:>10} {:<10}",
                timestamp,
                trade.side,
                token,
                whale_usd,
                our_usd,
                fill_pct,
                trade.status
            );
        }
    }

    println!("\nTotal trades: {}", trades.len());

    // Print summary statistics
    print_summary(trades);
}

/// Format market display from enriched data
fn format_market_display(enriched: &EnrichedData) -> String {
    match (&enriched.market_title, &enriched.outcome) {
        (Some(title), Some(outcome)) => {
            let combined = format!("{} - {}", title, outcome);
            if combined.len() > 40 {
                format!("{}...", &combined[..37])
            } else {
                combined
            }
        }
        (Some(title), None) => {
            if title.len() > 40 {
                format!("{}...", &title[..37])
            } else {
                title.clone()
            }
        }
        _ => "N/A".to_string(),
    }
}

/// Format current price based on trade side
fn format_current_price(side: &str, enriched: &EnrichedData) -> String {
    match side {
        "BUY" => enriched.current_bid.map(|p| format!("${:.3}", p)).unwrap_or_else(|| "N/A".to_string()),
        "SELL" => "closed".to_string(),
        _ => "N/A".to_string(),
    }
}

/// Calculate P&L percentage
fn calculate_pnl(side: &str, our_price: Option<f64>, enriched: &EnrichedData) -> String {
    if side != "BUY" {
        return "closed".to_string();
    }

    match (our_price, enriched.current_bid) {
        (Some(entry_price), Some(current_bid)) if entry_price > 0.0 => {
            let pnl_pct = ((current_bid - entry_price) / entry_price) * 100.0;
            format!("{:+.1}%", pnl_pct)
        }
        _ => "N/A".to_string(),
    }
}

/// Format Unix timestamp (milliseconds) to human-readable string
fn format_timestamp(timestamp_ms: i64) -> String {
    use chrono::{DateTime, Utc};
    let dt = DateTime::<Utc>::from_timestamp(timestamp_ms / 1000, 0)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Truncate token ID for display (show first 12 chars + ...)
fn truncate_token_id(token_id: &str) -> String {
    if token_id.len() > 15 {
        format!("{}...", &token_id[..12])
    } else {
        token_id.to_string()
    }
}

/// Print trades in CSV format
fn print_csv(trades: &[pm_whale_follower::persistence::TradeRecord]) {
    // Print CSV header
    println!("timestamp,side,token_id,trader_address,whale_shares,whale_price,whale_usd,our_shares,our_price,our_usd,fill_pct,status,latency_ms,tx_hash");

    // Print each trade
    for trade in trades {
        let timestamp = format_timestamp(trade.timestamp_ms);
        let our_shares = trade.our_shares.map(|v| v.to_string()).unwrap_or_default();
        let our_price = trade.our_price.map(|v| v.to_string()).unwrap_or_default();
        let our_usd = trade.our_usd.map(|v| v.to_string()).unwrap_or_default();
        let fill_pct = trade.fill_pct.map(|v| v.to_string()).unwrap_or_default();
        let latency_ms = trade.latency_ms.map(|v| v.to_string()).unwrap_or_default();

        println!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            timestamp,
            trade.side,
            trade.token_id,
            trade.trader_address,
            trade.whale_shares,
            trade.whale_price,
            trade.whale_usd,
            our_shares,
            our_price,
            our_usd,
            fill_pct,
            trade.status,
            latency_ms,
            trade.tx_hash
        );
    }
}

/// Print trades in JSON format
fn print_json(trades: &[pm_whale_follower::persistence::TradeRecord]) -> Result<()> {
    use serde_json::json;

    let json_trades: Vec<_> = trades
        .iter()
        .map(|t| {
            json!({
                "timestamp": format_timestamp(t.timestamp_ms),
                "timestamp_ms": t.timestamp_ms,
                "block_number": t.block_number,
                "tx_hash": t.tx_hash,
                "trader_address": t.trader_address,
                "token_id": t.token_id,
                "side": t.side,
                "whale_shares": t.whale_shares,
                "whale_price": t.whale_price,
                "whale_usd": t.whale_usd,
                "our_shares": t.our_shares,
                "our_price": t.our_price,
                "our_usd": t.our_usd,
                "fill_pct": t.fill_pct,
                "status": t.status,
                "latency_ms": t.latency_ms,
                "is_live": t.is_live,
            })
        })
        .collect();

    println!("{}", serde_json::to_string_pretty(&json_trades)?);
    Ok(())
}

/// Print summary statistics
fn print_summary(trades: &[pm_whale_follower::persistence::TradeRecord]) {
    if trades.is_empty() {
        return;
    }

    println!("\n=== SUMMARY STATISTICS ===");

    // Count by status
    let success_count = trades.iter().filter(|t| t.status == "SUCCESS").count();
    let failed_count = trades.iter().filter(|t| t.status == "FAILED").count();
    let partial_count = trades.iter().filter(|t| t.status == "PARTIAL").count();
    let skipped_count = trades.iter().filter(|t| t.status == "SKIPPED").count();

    println!("\nStatus breakdown:");
    println!("  SUCCESS: {} ({:.1}%)", success_count, (success_count as f64 / trades.len() as f64) * 100.0);
    if failed_count > 0 {
        println!("  FAILED:  {} ({:.1}%)", failed_count, (failed_count as f64 / trades.len() as f64) * 100.0);
    }
    if partial_count > 0 {
        println!("  PARTIAL: {} ({:.1}%)", partial_count, (partial_count as f64 / trades.len() as f64) * 100.0);
    }
    if skipped_count > 0 {
        println!("  SKIPPED: {} ({:.1}%)", skipped_count, (skipped_count as f64 / trades.len() as f64) * 100.0);
    }

    // Count by side
    let buy_count = trades.iter().filter(|t| t.side == "BUY").count();
    let sell_count = trades.iter().filter(|t| t.side == "SELL").count();

    println!("\nSide breakdown:");
    println!("  BUY:  {}", buy_count);
    println!("  SELL: {}", sell_count);

    // Calculate USD totals (only for successful trades)
    let successful_trades: Vec<_> = trades.iter().filter(|t| t.our_usd.is_some()).collect();
    if !successful_trades.is_empty() {
        let total_whale_usd: f64 = successful_trades.iter().map(|t| t.whale_usd).sum();
        let total_our_usd: f64 = successful_trades.iter().filter_map(|t| t.our_usd).sum();

        println!("\nVolume:");
        println!("  Whale total: ${:.2}", total_whale_usd);
        println!("  Our total:   ${:.2}", total_our_usd);
        println!("  Scaling:     {:.2}%", (total_our_usd / total_whale_usd) * 100.0);
    }

    // Calculate average latency
    let latencies: Vec<_> = trades.iter().filter_map(|t| t.latency_ms).collect();
    if !latencies.is_empty() {
        let avg_latency = latencies.iter().sum::<i64>() as f64 / latencies.len() as f64;
        let min_latency = latencies.iter().min().unwrap();
        let max_latency = latencies.iter().max().unwrap();

        println!("\nLatency (ms):");
        println!("  Average: {:.1}", avg_latency);
        println!("  Min:     {}", min_latency);
        println!("  Max:     {}", max_latency);
    }

    // Calculate average fill percentage
    let fill_pcts: Vec<_> = trades.iter().filter_map(|t| t.fill_pct).collect();
    if !fill_pcts.is_empty() {
        let avg_fill = fill_pcts.iter().sum::<f64>() / fill_pcts.len() as f64;
        println!("\nAverage fill: {:.1}%", avg_fill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_whale_follower::persistence::TradeRecord;

    #[test]
    fn test_print_table_empty() {
        // This test verifies that print_table handles empty trades gracefully
        let trades = vec![];
        print_table(&trades, None); // Should not panic
    }

    #[test]
    fn test_print_table_with_trades() {
        // This test verifies that print_table displays trades without panicking
        let trades = vec![
            TradeRecord {
                timestamp_ms: 1704067200000, // 2024-01-01 00:00:00
                block_number: 12345678,
                tx_hash: "0xabc123".to_string(),
                trader_address: "0xdef456".to_string(),
                token_id: "token123".to_string(),
                side: "BUY".to_string(),
                whale_shares: 1000.0,
                whale_price: 0.45,
                whale_usd: 450.0,
                our_shares: Some(20.0),
                our_price: Some(0.46),
                our_usd: Some(9.2),
                fill_pct: Some(100.0),
                status: "SUCCESS".to_string(),
                latency_ms: Some(50),
                is_live: Some(true),
                aggregation_count: None,
                aggregation_window_ms: None,
            },
            TradeRecord {
                timestamp_ms: 1704067260000, // 2024-01-01 00:01:00
                block_number: 12345679,
                tx_hash: "0xghi789".to_string(),
                trader_address: "0xdef456".to_string(),
                token_id: "token456".to_string(),
                side: "SELL".to_string(),
                whale_shares: 500.0,
                whale_price: 0.55,
                whale_usd: 275.0,
                our_shares: Some(10.0),
                our_price: Some(0.54),
                our_usd: Some(5.4),
                fill_pct: Some(100.0),
                status: "SUCCESS".to_string(),
                latency_ms: Some(75),
                is_live: Some(true),
                aggregation_count: None,
                aggregation_window_ms: None,
            },
        ];
        print_table(&trades, None); // Should not panic
    }

    #[test]
    fn test_print_table_with_failed_trade() {
        // Test display of failed trade with None values
        let trades = vec![
            TradeRecord {
                timestamp_ms: 1704067200000,
                block_number: 12345678,
                tx_hash: "0xabc123".to_string(),
                trader_address: "0xdef456".to_string(),
                token_id: "token123".to_string(),
                side: "BUY".to_string(),
                whale_shares: 1000.0,
                whale_price: 0.45,
                whale_usd: 450.0,
                our_shares: None,
                our_price: None,
                our_usd: None,
                fill_pct: None,
                status: "FAILED".to_string(),
                latency_ms: Some(50),
                is_live: Some(true),
                aggregation_count: None,
                aggregation_window_ms: None,
            },
        ];
        print_table(&trades, None); // Should not panic
    }

    #[test]
    fn test_format_timestamp() {
        // Test timestamp formatting
        let timestamp_ms = 1704067200000; // 2024-01-01 00:00:00 UTC
        let formatted = format_timestamp(timestamp_ms);
        assert_eq!(formatted, "2024-01-01 00:00:00");
    }

    #[test]
    fn test_truncate_token_id_short() {
        let short_id = "token123";
        assert_eq!(truncate_token_id(short_id), "token123");
    }

    #[test]
    fn test_truncate_token_id_long() {
        let long_id = "token1234567890abcdefghijklmnop";
        let result = truncate_token_id(long_id);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 15); // 12 chars + "..."
        assert_eq!(result, "token1234567...");
    }

    #[test]
    fn test_truncate_token_id_exact_boundary() {
        let boundary_id = "token1234567890"; // 15 chars exactly
        assert_eq!(truncate_token_id(boundary_id), boundary_id);
    }

    #[test]
    fn test_apply_filters_no_filters() {
        // Test that no filters returns all trades
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader2", "token2", "FAILED", 1704067260000),
        ];
        let filtered = apply_filters(trades.clone(), None, None, None, None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_apply_filters_by_trader() {
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader2", "token2", "FAILED", 1704067260000),
            create_test_trade("0xtrader1", "token3", "SUCCESS", 1704067320000),
        ];
        let filtered = apply_filters(trades, Some("trader1"), None, None, None);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.trader_address.contains("trader1")));
    }

    #[test]
    fn test_apply_filters_by_token() {
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader2", "token2", "FAILED", 1704067260000),
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067320000),
        ];
        let filtered = apply_filters(trades, None, Some("token1"), None, None);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.token_id.contains("token1")));
    }

    #[test]
    fn test_apply_filters_by_status() {
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader2", "token2", "FAILED", 1704067260000),
            create_test_trade("0xtrader1", "token3", "SUCCESS", 1704067320000),
        ];
        let filtered = apply_filters(trades, None, None, Some("SUCCESS"), None);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.status == "SUCCESS"));
    }

    #[test]
    fn test_apply_filters_by_since() {
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000), // 2024-01-01 00:00:00
            create_test_trade("0xtrader2", "token2", "FAILED", 1704067260000),  // 2024-01-01 00:01:00
            create_test_trade("0xtrader1", "token3", "SUCCESS", 1704067320000), // 2024-01-01 00:02:00
        ];
        // Filter to show only trades after 2024-01-01 00:01:00 (1704067260 seconds)
        let filtered = apply_filters(trades, None, None, None, Some(1704067260));
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_apply_filters_combined() {
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader2", "token2", "FAILED", 1704067260000),
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067320000),
            create_test_trade("0xtrader1", "token2", "SUCCESS", 1704067380000),
        ];
        // Filter by trader1 AND token1
        let filtered = apply_filters(trades, Some("trader1"), Some("token1"), None, None);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|t| t.trader_address.contains("trader1") && t.token_id.contains("token1")));
    }

    #[test]
    fn test_print_csv_empty() {
        // Verify CSV output handles empty trades gracefully
        let trades = vec![];
        print_csv(&trades); // Should not panic
    }

    #[test]
    fn test_print_csv_with_trades() {
        // Verify CSV output handles trades without panicking
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
        ];
        print_csv(&trades); // Should not panic
    }

    #[test]
    fn test_print_json_empty() {
        // Verify JSON output handles empty trades gracefully
        let trades = vec![];
        assert!(print_json(&trades).is_ok());
    }

    #[test]
    fn test_print_json_with_trades() {
        // Verify JSON output handles trades without panicking
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
        ];
        assert!(print_json(&trades).is_ok());
    }

    #[test]
    fn test_print_summary_empty() {
        // Verify summary handles empty trades gracefully
        let trades = vec![];
        print_summary(&trades); // Should not panic
    }

    #[test]
    fn test_print_summary_with_trades() {
        // Verify summary displays statistics without panicking
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader2", "token2", "FAILED", 1704067260000),
            create_test_trade("0xtrader1", "token3", "SUCCESS", 1704067320000),
        ];
        print_summary(&trades); // Should not panic
    }

    #[test]
    fn test_print_summary_mixed_sides() {
        // Verify summary handles mixed BUY/SELL trades
        let mut trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
        ];
        trades[0].side = "SELL".to_string();
        trades.push(create_test_trade("0xtrader2", "token2", "SUCCESS", 1704067260000));
        print_summary(&trades); // Should not panic
    }

    #[test]
    fn test_args_refresh_flag_default_is_false() {
        // Test that refresh flag defaults to false
        let args = Args::try_parse_from(vec!["trade_history"]).unwrap();
        assert!(!args.refresh);
    }

    #[test]
    fn test_args_refresh_flag_can_be_enabled() {
        // Test that refresh flag can be explicitly set to true
        let args = Args::try_parse_from(vec!["trade_history", "--refresh"]).unwrap();
        assert!(args.refresh);
    }

    #[test]
    fn test_args_refresh_flag_with_other_flags() {
        // Test that refresh flag works alongside other flags
        let args = Args::try_parse_from(vec![
            "trade_history",
            "--refresh",
            "--limit",
            "100",
            "--db",
            "test.db",
        ])
        .unwrap();
        assert!(args.refresh);
        assert_eq!(args.limit, 100);
        assert_eq!(args.db, "test.db");
    }

    #[test]
    fn test_extract_unique_token_ids_empty() {
        // Test that extracting unique token IDs from empty trades returns empty set
        let trades = vec![];
        let unique_tokens = extract_unique_token_ids(&trades);
        assert_eq!(unique_tokens.len(), 0);
    }

    #[test]
    fn test_extract_unique_token_ids_single_trade() {
        // Test that extracting unique token IDs from a single trade returns one token
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
        ];
        let unique_tokens = extract_unique_token_ids(&trades);
        assert_eq!(unique_tokens.len(), 1);
        assert!(unique_tokens.contains(&"token1".to_string()));
    }

    #[test]
    fn test_extract_unique_token_ids_multiple_trades_same_token() {
        // Test that extracting unique token IDs from multiple trades with same token returns one token
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067260000),
            create_test_trade("0xtrader2", "token1", "FAILED", 1704067320000),
        ];
        let unique_tokens = extract_unique_token_ids(&trades);
        assert_eq!(unique_tokens.len(), 1);
        assert!(unique_tokens.contains(&"token1".to_string()));
    }

    #[test]
    fn test_extract_unique_token_ids_multiple_trades_different_tokens() {
        // Test that extracting unique token IDs from multiple trades with different tokens returns all unique tokens
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader2", "token2", "SUCCESS", 1704067260000),
            create_test_trade("0xtrader1", "token1", "FAILED", 1704067320000),
            create_test_trade("0xtrader3", "token3", "SUCCESS", 1704067380000),
        ];
        let unique_tokens = extract_unique_token_ids(&trades);
        assert_eq!(unique_tokens.len(), 3);
        assert!(unique_tokens.contains(&"token1".to_string()));
        assert!(unique_tokens.contains(&"token2".to_string()));
        assert!(unique_tokens.contains(&"token3".to_string()));
    }

    #[test]
    fn test_enriched_data_struct_creation() {
        // Test that EnrichedData struct can be created with all Option fields
        let enriched = EnrichedData {
            market_title: Some("Will it rain?".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: Some(0.45),
            current_ask: Some(0.46),
        };

        assert_eq!(enriched.market_title, Some("Will it rain?".to_string()));
        assert_eq!(enriched.outcome, Some("Yes".to_string()));
        assert_eq!(enriched.current_bid, Some(0.45));
        assert_eq!(enriched.current_ask, Some(0.46));
    }

    #[test]
    fn test_enriched_data_with_none_values() {
        // Test that EnrichedData can handle None values (for failed fetches)
        let enriched = EnrichedData {
            market_title: None,
            outcome: None,
            current_bid: None,
            current_ask: None,
        };

        assert!(enriched.market_title.is_none());
        assert!(enriched.outcome.is_none());
        assert!(enriched.current_bid.is_none());
        assert!(enriched.current_ask.is_none());
    }

    #[test]
    fn test_collect_enriched_data_empty_tokens() {
        // Test that collecting enriched data with empty token list returns empty HashMap
        let tokens: Vec<String> = vec![];
        let enriched_data = collect_enriched_data(&tokens);
        assert_eq!(enriched_data.len(), 0);
    }

    #[test]
    fn test_collect_enriched_data_with_invalid_tokens() {
        // Test that collecting enriched data with invalid tokens returns HashMap with None values
        // This test uses an invalid host to simulate API failures
        let tokens = vec!["invalid_token_123".to_string()];
        let enriched_data = collect_enriched_data(&tokens);

        // Should have entry for the token
        assert_eq!(enriched_data.len(), 1);
        assert!(enriched_data.contains_key("invalid_token_123"));

        // All fields should be None because fetch failed
        let data = &enriched_data["invalid_token_123"];
        assert!(data.market_title.is_none());
        assert!(data.outcome.is_none());
        assert!(data.current_bid.is_none());
        assert!(data.current_ask.is_none());
    }

    #[test]
    fn test_maybe_collect_enriched_data_when_refresh_true() {
        // Test that enriched data is collected when refresh is true
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
            create_test_trade("0xtrader2", "token2", "SUCCESS", 1704067260000),
        ];

        let enriched = maybe_collect_enriched_data(true, &trades);
        assert!(enriched.is_some());

        let data = enriched.unwrap();
        // Should have entries for both tokens
        assert!(data.contains_key("token1"));
        assert!(data.contains_key("token2"));
    }

    #[test]
    fn test_maybe_collect_enriched_data_when_refresh_false() {
        // Test that enriched data is NOT collected when refresh is false
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
        ];

        let enriched = maybe_collect_enriched_data(false, &trades);
        assert!(enriched.is_none());
    }

    #[test]
    fn test_maybe_collect_enriched_data_with_empty_trades() {
        // Test that empty trades return empty HashMap when refresh is true
        let trades = vec![];

        let enriched = maybe_collect_enriched_data(true, &trades);
        assert!(enriched.is_some());
        assert_eq!(enriched.unwrap().len(), 0);
    }

    #[test]
    fn test_print_table_with_enriched_data_none() {
        // Test that print_table works with None enriched data (existing behavior)
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
        ];
        print_table(&trades, None); // Should not panic
    }

    #[test]
    fn test_print_table_with_enriched_data_some_empty() {
        // Test that print_table works with Some but empty enriched data
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
        ];
        let enriched = HashMap::new();
        print_table(&trades, Some(&enriched)); // Should not panic
    }

    #[test]
    fn test_print_table_with_enriched_data_populated() {
        // Test that print_table works with populated enriched data
        let trades = vec![
            create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000),
        ];
        let mut enriched = HashMap::new();
        enriched.insert(
            "token1".to_string(),
            EnrichedData {
                market_title: Some("Will it rain?".to_string()),
                outcome: Some("Yes".to_string()),
                current_bid: Some(0.50),
                current_ask: Some(0.51),
            },
        );
        print_table(&trades, Some(&enriched)); // Should not panic
    }

    #[test]
    fn test_format_market_display_with_title_and_outcome() {
        let enriched = EnrichedData {
            market_title: Some("Will it rain?".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: Some(0.50),
            current_ask: Some(0.51),
        };
        let display = format_market_display(&enriched);
        assert_eq!(display, "Will it rain? - Yes");
    }

    #[test]
    fn test_format_market_display_truncates_long_text() {
        let enriched = EnrichedData {
            market_title: Some("This is a very long market title that should be truncated".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: Some(0.50),
            current_ask: Some(0.51),
        };
        let display = format_market_display(&enriched);
        assert!(display.len() <= 40);
        assert!(display.ends_with("..."));
    }

    #[test]
    fn test_format_market_display_with_none_values() {
        let enriched = EnrichedData {
            market_title: None,
            outcome: None,
            current_bid: Some(0.50),
            current_ask: Some(0.51),
        };
        let display = format_market_display(&enriched);
        assert_eq!(display, "N/A");
    }

    #[test]
    fn test_format_current_price_for_buy() {
        let enriched = EnrichedData {
            market_title: Some("Market".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: Some(0.456),
            current_ask: Some(0.457),
        };
        let price = format_current_price("BUY", &enriched);
        assert_eq!(price, "$0.456");
    }

    #[test]
    fn test_format_current_price_for_sell() {
        let enriched = EnrichedData {
            market_title: Some("Market".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: Some(0.456),
            current_ask: Some(0.457),
        };
        let price = format_current_price("SELL", &enriched);
        assert_eq!(price, "closed");
    }

    #[test]
    fn test_format_current_price_with_none() {
        let enriched = EnrichedData {
            market_title: Some("Market".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: None,
            current_ask: None,
        };
        let price = format_current_price("BUY", &enriched);
        assert_eq!(price, "N/A");
    }

    #[test]
    fn test_calculate_pnl_for_buy_positive() {
        let enriched = EnrichedData {
            market_title: Some("Market".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: Some(0.50),
            current_ask: Some(0.51),
        };
        let pnl = calculate_pnl("BUY", Some(0.46), &enriched);
        // (0.50 - 0.46) / 0.46 * 100 = 8.7%
        assert!(pnl.starts_with("+8."));
        assert!(pnl.ends_with("%"));
    }

    #[test]
    fn test_calculate_pnl_for_buy_negative() {
        let enriched = EnrichedData {
            market_title: Some("Market".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: Some(0.40),
            current_ask: Some(0.41),
        };
        let pnl = calculate_pnl("BUY", Some(0.50), &enriched);
        // (0.40 - 0.50) / 0.50 * 100 = -20.0%
        assert_eq!(pnl, "-20.0%");
    }

    #[test]
    fn test_calculate_pnl_for_sell() {
        let enriched = EnrichedData {
            market_title: Some("Market".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: Some(0.50),
            current_ask: Some(0.51),
        };
        let pnl = calculate_pnl("SELL", Some(0.46), &enriched);
        assert_eq!(pnl, "closed");
    }

    #[test]
    fn test_calculate_pnl_with_none_values() {
        let enriched = EnrichedData {
            market_title: Some("Market".to_string()),
            outcome: Some("Yes".to_string()),
            current_bid: None,
            current_ask: None,
        };
        let pnl = calculate_pnl("BUY", None, &enriched);
        assert_eq!(pnl, "N/A");
    }

    #[test]
    fn test_end_to_end_refresh_flow_with_successful_trade() {
        // Integration test: verify entire refresh flow works for a successful BUY trade
        // Create a successful BUY trade
        let mut trade = create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000);
        trade.our_price = Some(0.45); // Entry price
        trade.our_usd = Some(45.0);
        let trades = vec![trade];

        // Create enriched data simulating successful API fetches
        let mut enriched = HashMap::new();
        enriched.insert(
            "token1".to_string(),
            EnrichedData {
                market_title: Some("Will it rain tomorrow?".to_string()),
                outcome: Some("Yes".to_string()),
                current_bid: Some(0.50), // Current price higher than entry
                current_ask: Some(0.51),
            },
        );

        // This should not panic and should display P&L
        print_table(&trades, Some(&enriched));

        // Verify P&L calculation is correct: (0.50 - 0.45) / 0.45 * 100 = +11.1%
        let pnl = calculate_pnl("BUY", Some(0.45), &enriched["token1"]);
        assert!(pnl.starts_with("+11."));
        assert!(pnl.ends_with("%"));
    }

    #[test]
    fn test_end_to_end_refresh_flow_with_sell_trade() {
        // Integration test: verify entire refresh flow works for a SELL trade
        let mut trade = create_test_trade("0xtrader1", "token1", "SUCCESS", 1704067200000);
        trade.side = "SELL".to_string();
        trade.our_price = Some(0.55);
        trade.our_usd = Some(55.0);
        let trades = vec![trade];

        // Create enriched data
        let mut enriched = HashMap::new();
        enriched.insert(
            "token1".to_string(),
            EnrichedData {
                market_title: Some("Test Market".to_string()),
                outcome: Some("No".to_string()),
                current_bid: Some(0.50),
                current_ask: Some(0.51),
            },
        );

        // This should not panic
        print_table(&trades, Some(&enriched));

        // Verify SELL shows "closed" for both current price and P&L
        let current_price = format_current_price("SELL", &enriched["token1"]);
        assert_eq!(current_price, "closed");

        let pnl = calculate_pnl("SELL", Some(0.55), &enriched["token1"]);
        assert_eq!(pnl, "closed");
    }

    // Helper function to create test trades
    fn create_test_trade(trader: &str, token: &str, status: &str, timestamp_ms: i64) -> TradeRecord {
        TradeRecord {
            timestamp_ms,
            block_number: 12345678,
            tx_hash: "0xabc123".to_string(),
            trader_address: trader.to_string(),
            token_id: token.to_string(),
            side: "BUY".to_string(),
            whale_shares: 1000.0,
            whale_price: 0.45,
            whale_usd: 450.0,
            our_shares: Some(20.0),
            our_price: Some(0.46),
            our_usd: Some(9.2),
            fill_pct: Some(100.0),
            status: status.to_string(),
            latency_ms: Some(50),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        }
    }
}
