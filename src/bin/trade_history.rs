// trade_history.rs - CLI tool for querying trade history from SQLite database
//
// Usage:
//   cargo run --bin trade_history                     # Show recent trades
//   cargo run --bin trade_history -- --db test.db     # Use custom database
//   cargo run --bin trade_history -- --limit 100      # Show more trades

use anyhow::Result;
use clap::Parser;
use pm_whale_follower::persistence::TradeStore;

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

    // Display trades in requested format
    match args.format.to_lowercase().as_str() {
        "csv" => print_csv(&trades),
        "json" => print_json(&trades)?,
        _ => print_table(&trades),
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

/// Print trades in a formatted table
fn print_table(trades: &[pm_whale_follower::persistence::TradeRecord]) {
    println!("\n=== TRADE HISTORY ===\n");

    if trades.is_empty() {
        println!("No trades found.");
        return;
    }

    // Print header
    println!(
        "{:<20} {:<8} {:<15} {:>12} {:>10} {:>10} {:<10}",
        "Timestamp", "Side", "Token", "Whale $", "Our $", "Fill %", "Status"
    );
    println!("{}", "-".repeat(100));

    // Print each trade
    for trade in trades {
        let timestamp = format_timestamp(trade.timestamp_ms);
        let token = truncate_token_id(&trade.token_id);
        let whale_usd = format!("${:.2}", trade.whale_usd);
        let our_usd = trade.our_usd
            .map(|v| format!("${:.2}", v))
            .unwrap_or_else(|| "N/A".to_string());
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

    println!("\nTotal trades: {}", trades.len());

    // Print summary statistics
    print_summary(trades);
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
        print_table(&trades); // Should not panic
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
            },
        ];
        print_table(&trades); // Should not panic
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
            },
        ];
        print_table(&trades); // Should not panic
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
        }
    }
}
