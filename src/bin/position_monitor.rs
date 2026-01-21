// position_monitor.rs - CLI tool for monitoring current positions from trade database
//
// Usage:
//   cargo run --bin position_monitor                    # Monitor with live P&L and daily P&L change
//   cargo run --bin position_monitor -- --db test.db    # Use custom database
//   cargo run --bin position_monitor -- --no-prices     # Show positions without prices/P&L
//   cargo run --bin position_monitor -- --ttl 60        # Set price cache TTL to 60 seconds
//   cargo run --bin position_monitor -- --once          # Single snapshot and exit
//   cargo run --bin position_monitor -- --stats         # Show aggregation statistics
//   cargo run --bin position_monitor -- --json          # Output portfolio data in JSON format
//
// Features:
//   - Daily P&L tracking: Snapshots portfolio value at start of each day (UTC)
//   - Snapshot file: Stored as .portfolio_snapshot.json in same directory as database
//   - JSON output: Use --json flag for machine-readable output

use anyhow::Result;
use clap::Parser;
use pm_whale_follower::persistence::{TradeStore, Position, AggregationStats};
use pm_whale_follower::prices::{PriceCache, PriceInfo};
use serde::{Serialize, Deserialize};
use std::path::{Path, PathBuf};

/// Daily snapshot of portfolio state for tracking day-over-day changes
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DailySnapshot {
    date: String,              // ISO date "2026-01-20"
    portfolio_value: f64,
    cost_basis: f64,
    unrealized_pnl: f64,
    timestamp: String,         // ISO datetime when taken
}

/// JSON representation of a position for output
#[derive(Debug, Clone, Serialize)]
struct PositionJson {
    token_id: String,
    net_shares: f64,
    avg_entry_price: Option<f64>,
    current_price: Option<f64>,
    position_value: Option<f64>,
    unrealized_pnl: Option<f64>,
}

/// JSON representation of portfolio for output
#[derive(Debug, Clone, Serialize)]
struct PortfolioJson {
    timestamp: String,
    portfolio_value: f64,
    cost_basis: f64,
    unrealized_pnl: f64,
    daily_pnl_change: f64,
    snapshot_date: String,
    position_count: usize,
    positions: Vec<PositionJson>,
}

/// Position enriched with price information for P&L calculation
#[derive(Debug, Clone)]
struct PositionWithPrice {
    position: Position,
    price_info: Option<PriceInfo>,
}

/// Calculate the market value of a position
///
/// # Arguments
/// * `net_shares` - Net shares (positive for long, negative for short)
/// * `bid_price` - Current bid price (what you'd sell at)
/// * `ask_price` - Current ask price (what you'd buy at)
///
/// # Returns
/// * `f64` - Position value in USD
fn calculate_position_value(net_shares: f64, bid_price: f64, ask_price: f64) -> f64 {
    if net_shares > 0.0 {
        // LONG position: value is what we'd get selling at bid
        net_shares * bid_price
    } else if net_shares < 0.0 {
        // SHORT position: value is the obligation (cost to close at ask)
        net_shares.abs() * ask_price
    } else {
        // Zero shares
        0.0
    }
}

/// Calculate the cost basis of a position
///
/// # Arguments
/// * `net_shares` - Net shares (positive for long, negative for short)
/// * `avg_entry_price` - Average entry price
///
/// # Returns
/// * `Option<f64>` - Cost basis in USD, or None if avg_entry_price is None
fn calculate_cost_basis(net_shares: f64, avg_entry_price: Option<f64>) -> Option<f64> {
    avg_entry_price.map(|entry| net_shares.abs() * entry)
}

/// Calculate unrealized P&L for a position
///
/// # Arguments
/// * `net_shares` - Net shares (positive for long, negative for short)
/// * `avg_entry` - Average entry price
/// * `bid_price` - Current bid price (what you'd sell at)
/// * `ask_price` - Current ask price (what you'd buy at)
///
/// # Returns
/// * `Option<f64>` - Unrealized P&L in USD, or None if avg_entry is None
fn calculate_unrealized_pnl(
    net_shares: f64,
    avg_entry: Option<f64>,
    bid_price: f64,
    ask_price: f64,
) -> Option<f64> {
    avg_entry.map(|entry| {
        if net_shares > 0.0 {
            // LONG position: use bid price (what you'd sell at)
            (bid_price - entry) * net_shares
        } else {
            // SHORT position: use ask price (what you'd buy at to close)
            (entry - ask_price) * net_shares.abs()
        }
    })
}

/// Portfolio summary aggregating all positions
#[derive(Debug, Clone, PartialEq)]
struct PortfolioSummary {
    total_value: f64,
    cost_basis: f64,
    unrealized_pnl: f64,
    position_count: usize,
}

/// Calculate portfolio summary from positions with prices
///
/// # Arguments
/// * `positions` - Slice of positions with price information
///
/// # Returns
/// * `PortfolioSummary` - Aggregated portfolio metrics
fn calculate_portfolio_summary(positions: &[PositionWithPrice]) -> PortfolioSummary {
    let mut total_value = 0.0;
    let mut cost_basis = 0.0;
    let mut unrealized_pnl = 0.0;
    let mut position_count = 0;

    for pos_with_price in positions {
        let pos = &pos_with_price.position;

        if let Some(price_info) = &pos_with_price.price_info {
            // Calculate position value
            let value = calculate_position_value(
                pos.net_shares,
                price_info.bid_price,
                price_info.ask_price,
            );
            total_value += value;

            // Calculate cost basis
            if let Some(basis) = calculate_cost_basis(pos.net_shares, pos.avg_entry_price) {
                cost_basis += basis;
            }

            // Calculate P&L
            if let Some(pnl) = calculate_unrealized_pnl(
                pos.net_shares,
                pos.avg_entry_price,
                price_info.bid_price,
                price_info.ask_price,
            ) {
                unrealized_pnl += pnl;
            }

            position_count += 1;
        }
    }

    PortfolioSummary {
        total_value,
        cost_basis,
        unrealized_pnl,
        position_count,
    }
}

#[derive(Parser)]
#[command(name = "position_monitor")]
#[command(about = "Monitor current positions from trade database")]
struct Args {
    /// Database path
    #[arg(long, default_value = "trades.db")]
    db: String,

    /// Run once and exit
    #[arg(long)]
    once: bool,

    /// Show aggregation statistics
    #[arg(long)]
    stats: bool,

    /// Skip price fetching (show positions only)
    #[arg(long)]
    no_prices: bool,

    /// Cache TTL for prices in seconds
    #[arg(long, default_value = "30")]
    ttl: u64,

    /// Output in JSON format
    #[arg(long)]
    json: bool,
}

/// Get the path for the daily snapshot file
///
/// # Arguments
/// * `db_path` - Path to the database file
///
/// # Returns
/// * `PathBuf` - Path to the snapshot file (same directory as DB, named `.portfolio_snapshot.json`)
///
/// # Examples
/// * `trades.db` -> `.portfolio_snapshot.json`
/// * `/path/to/mydata.db` -> `/path/to/.portfolio_snapshot.json`
fn get_snapshot_path(db_path: &str) -> PathBuf {
    let db = Path::new(db_path);
    let parent = db.parent().unwrap_or_else(|| Path::new(""));
    parent.join(".portfolio_snapshot.json")
}

/// Get today's date in UTC as ISO date string
///
/// # Returns
/// * `String` - Today's date in "YYYY-MM-DD" format (UTC)
fn get_today_utc() -> String {
    use chrono::Utc;
    Utc::now().format("%Y-%m-%d").to_string()
}

/// Load daily snapshot from file
///
/// # Arguments
/// * `path` - Path to the snapshot file
///
/// # Returns
/// * `Option<DailySnapshot>` - The snapshot if file exists and is valid, None otherwise
fn load_daily_snapshot(path: &Path) -> Option<DailySnapshot> {
    use std::fs;

    let contents = fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Save daily snapshot to file
///
/// # Arguments
/// * `path` - Path to save the snapshot file
/// * `snapshot` - The snapshot to save
///
/// # Returns
/// * `Result<()>` - Ok if successful, error otherwise
fn save_daily_snapshot(path: &Path, snapshot: &DailySnapshot) -> Result<()> {
    use std::fs;

    let json = serde_json::to_string_pretty(snapshot)?;
    fs::write(path, json)?;
    Ok(())
}

/// Check and update daily snapshot
///
/// If snapshot exists and date matches today, return existing snapshot.
/// If snapshot doesn't exist or date is different, create new snapshot with current values and save.
///
/// # Arguments
/// * `path` - Path to the snapshot file
/// * `current_summary` - Current portfolio summary
///
/// # Returns
/// * `DailySnapshot` - Either the existing snapshot or newly created one
fn check_and_update_snapshot(path: &Path, current_summary: &PortfolioSummary) -> DailySnapshot {
    use chrono::Utc;

    let today = get_today_utc();

    // Try to load existing snapshot
    if let Some(existing) = load_daily_snapshot(path) {
        if existing.date == today {
            // Same day - return existing snapshot
            return existing;
        }
    }

    // Different day or no snapshot - create new one
    let new_snapshot = DailySnapshot {
        date: today,
        portfolio_value: current_summary.total_value,
        cost_basis: current_summary.cost_basis,
        unrealized_pnl: current_summary.unrealized_pnl,
        timestamp: Utc::now().to_rfc3339(),
    };

    // Save the new snapshot
    save_daily_snapshot(path, &new_snapshot).ok();

    new_snapshot
}

/// Calculate daily P&L change
///
/// # Arguments
/// * `current_pnl` - Current unrealized P&L
/// * `snapshot` - Daily snapshot from start of day
///
/// # Returns
/// * `f64` - Change in P&L since snapshot
fn calculate_daily_pnl_change(current_pnl: f64, snapshot: &DailySnapshot) -> f64 {
    current_pnl - snapshot.unrealized_pnl
}

/// Convert positions and summary to JSON output format
///
/// # Arguments
/// * `positions` - Slice of positions with price information
/// * `summary` - Portfolio summary
/// * `snapshot` - Daily snapshot for calculating daily P&L change
///
/// # Returns
/// * `PortfolioJson` - JSON-serializable portfolio data
fn to_portfolio_json(
    positions: &[PositionWithPrice],
    summary: &PortfolioSummary,
    snapshot: &DailySnapshot,
) -> PortfolioJson {
    use chrono::Utc;

    let daily_pnl_change = calculate_daily_pnl_change(summary.unrealized_pnl, snapshot);

    let position_jsons: Vec<PositionJson> = positions
        .iter()
        .map(|pos_with_price| {
            let pos = &pos_with_price.position;
            let (current_price, position_value, unrealized_pnl) = if let Some(price_info) = &pos_with_price.price_info {
                let current_price = if pos.net_shares > 0.0 {
                    price_info.bid_price
                } else {
                    price_info.ask_price
                };

                let value = calculate_position_value(
                    pos.net_shares,
                    price_info.bid_price,
                    price_info.ask_price,
                );

                let pnl = calculate_unrealized_pnl(
                    pos.net_shares,
                    pos.avg_entry_price,
                    price_info.bid_price,
                    price_info.ask_price,
                );

                (Some(current_price), Some(value), pnl)
            } else {
                (None, None, None)
            };

            PositionJson {
                token_id: pos.token_id.clone(),
                net_shares: pos.net_shares,
                avg_entry_price: pos.avg_entry_price,
                current_price,
                position_value,
                unrealized_pnl,
            }
        })
        .collect();

    PortfolioJson {
        timestamp: Utc::now().to_rfc3339(),
        portfolio_value: summary.total_value,
        cost_basis: summary.cost_basis,
        unrealized_pnl: summary.unrealized_pnl,
        daily_pnl_change,
        snapshot_date: snapshot.date.clone(),
        position_count: summary.position_count,
        positions: position_jsons,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Open database read-only
    let store = TradeStore::new(&args.db)?;

    if args.stats {
        // Display aggregation statistics
        let stats = store.get_aggregation_stats()?;
        print_aggregation_stats(&stats);
    } else {
        // Fetch positions
        let positions = store.get_positions()?;

        // Enrich positions with price data (unless --no-prices is set)
        let positions_with_prices = if args.no_prices {
            // No price fetching - create PositionWithPrice with None
            positions
                .into_iter()
                .map(|pos| PositionWithPrice {
                    position: pos,
                    price_info: None,
                })
                .collect()
        } else {
            // Fetch prices for all positions
            fetch_prices_for_positions(positions, args.ttl)
        };

        // Calculate portfolio summary
        let summary = calculate_portfolio_summary(&positions_with_prices);

        // Get or create daily snapshot
        let snapshot_path = get_snapshot_path(&args.db);
        let snapshot = check_and_update_snapshot(&snapshot_path, &summary);

        // Output based on format flag
        if args.json {
            // JSON output
            let portfolio_json = to_portfolio_json(&positions_with_prices, &summary, &snapshot);
            let json_output = serde_json::to_string_pretty(&portfolio_json)?;
            println!("{}", json_output);
        } else {
            // Table output
            print_table_with_pnl(&positions_with_prices, &snapshot);
        }
    }

    Ok(())
}

/// Fetch prices for all positions using batch API
fn fetch_prices_for_positions(positions: Vec<Position>, ttl_seconds: u64) -> Vec<PositionWithPrice> {
    // Create price cache
    let mut cache = PriceCache::new(ttl_seconds);

    // Extract token IDs
    let token_ids: Vec<&str> = positions.iter().map(|p| p.token_id.as_str()).collect();

    // Batch fetch prices with fallback
    let mut price_map = std::collections::HashMap::new();
    for token_id in &token_ids {
        if let Some(price) = cache.get_or_fetch_price_with_fallback(token_id) {
            price_map.insert(token_id.to_string(), price);
        }
    }

    // Enrich positions with prices
    positions
        .into_iter()
        .map(|pos| {
            let price_info = price_map.get(&pos.token_id).cloned();
            PositionWithPrice {
                position: pos,
                price_info,
            }
        })
        .collect()
}

/// Print positions with P&L in a formatted table
fn print_table_with_pnl(positions: &[PositionWithPrice], snapshot: &DailySnapshot) {
    println!("\n=== CURRENT POSITIONS ===\n");

    if positions.is_empty() {
        println!("No positions found.");
        return;
    }

    // Print header
    println!("{:<20} {:>12} {:>12} {:>12} {:>15}",
        "Token ID", "Shares", "Avg Entry", "Current", "Unrealized P&L");
    println!("{}", "-".repeat(75));

    let mut total_pnl = 0.0;
    let mut has_any_pnl = false;

    // Print each position
    for pos_with_price in positions {
        let pos = &pos_with_price.position;

        let avg_price_str = pos.avg_entry_price
            .map(|p| format!("{:.4}", p))
            .unwrap_or_else(|| "N/A".to_string());

        let (current_price_str, pnl_str) = if let Some(price_info) = &pos_with_price.price_info {
            // We have price data - calculate P&L
            let pnl = calculate_unrealized_pnl(
                pos.net_shares,
                pos.avg_entry_price,
                price_info.bid_price,
                price_info.ask_price,
            );

            let current_price = if pos.net_shares > 0.0 {
                price_info.bid_price
            } else {
                price_info.ask_price
            };

            let current_str = format!("{:.4}", current_price);

            let pnl_display = if let Some(pnl_value) = pnl {
                total_pnl += pnl_value;
                has_any_pnl = true;
                format!("{:+.2}", pnl_value)
            } else {
                "N/A".to_string()
            };

            (current_str, pnl_display)
        } else {
            // No price data available
            ("N/A".to_string(), "N/A".to_string())
        };

        println!("{:<20} {:>12.2} {:>12} {:>12} {:>15}",
            truncate_token_id(&pos.token_id),
            pos.net_shares,
            avg_price_str,
            current_price_str,
            pnl_str
        );
    }

    println!("\nTotal positions: {}", positions.len());

    if has_any_pnl {
        println!("Total Unrealized P&L: {:+.2}", total_pnl);
    }

    // Calculate and display portfolio summary
    let summary = calculate_portfolio_summary(positions);

    if summary.position_count > 0 {
        let daily_pnl_change = calculate_daily_pnl_change(summary.unrealized_pnl, snapshot);

        println!("\n=== PORTFOLIO SUMMARY ===");
        println!("Total Portfolio Value:  ${:.2}", summary.total_value);
        println!("Total Cost Basis:       ${:.2}", summary.cost_basis);
        println!("Total Unrealized P&L:   ${:+.2}", summary.unrealized_pnl);
        println!("Daily P&L Change:       ${:+.2} (since {})", daily_pnl_change, snapshot.date);
        println!("Positions with Prices:  {}", summary.position_count);
    }
}

/// Truncate token ID for display (show first 10 chars + ...)
fn truncate_token_id(token_id: &str) -> String {
    if token_id.len() > 17 {
        format!("{}...", &token_id[..14])
    } else {
        token_id.to_string()
    }
}

/// Print aggregation statistics in a formatted display
fn print_aggregation_stats(stats: &AggregationStats) {
    println!("\n=== AGGREGATION STATISTICS ===\n");

    if stats.total_orders == 0 {
        println!("No orders found in database.");
        return;
    }

    // Calculate percentage of aggregated orders
    let aggregation_pct = (stats.aggregated_orders as f64 / stats.total_orders as f64) * 100.0;

    // Calculate estimated fees saved
    let fees_saved = calculate_fees_saved(stats.total_trades_combined, stats.aggregated_orders);

    println!("Total orders:          {}", stats.total_orders);
    println!("Aggregated orders:     {} ({:.1}%)", stats.aggregated_orders, aggregation_pct);
    println!("Avg trades per agg:    {:.1}", stats.avg_trades_per_aggregation);
    println!("Estimated fees saved:  ${:.2}", fees_saved);
    println!();
}

/// Calculate estimated fees saved through aggregation
///
/// Assumes $0.02 per trade saved by aggregation.
/// Formula: (total_trades_combined - aggregated_orders) * $0.02
///
/// # Arguments
/// * `total_trades_combined` - Total individual trades combined through aggregation
/// * `aggregated_orders` - Number of aggregated orders executed
///
/// # Returns
/// * `f64` - Estimated fees saved in USD
fn calculate_fees_saved(total_trades_combined: u32, aggregated_orders: u32) -> f64 {
    if total_trades_combined < aggregated_orders {
        return 0.0;
    }
    let trades_saved = total_trades_combined - aggregated_orders;
    trades_saved as f64 * 0.02
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_whale_follower::persistence::Position;

    #[test]
    fn test_truncate_token_id_short() {
        let short_id = "12345";
        assert_eq!(truncate_token_id(short_id), "12345");
    }

    #[test]
    fn test_truncate_token_id_long() {
        let long_id = "1234567890abcdefghijklmnop";
        let result = truncate_token_id(long_id);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 17); // 14 chars + "..."
        assert_eq!(result, "1234567890abcd...");
    }

    #[test]
    fn test_truncate_token_id_exact_boundary() {
        let boundary_id = "1234567890abcdefg"; // 17 chars exactly
        assert_eq!(truncate_token_id(boundary_id), boundary_id);
    }

    #[test]
    fn test_print_aggregation_stats_with_data() {
        use pm_whale_follower::persistence::AggregationStats;

        let stats = AggregationStats {
            total_orders: 150,
            aggregated_orders: 45,
            total_trades_combined: 126,
            avg_trades_per_aggregation: 2.8,
        };

        print_aggregation_stats(&stats); // Should not panic
    }

    #[test]
    fn test_print_aggregation_stats_empty() {
        use pm_whale_follower::persistence::AggregationStats;

        let stats = AggregationStats {
            total_orders: 0,
            aggregated_orders: 0,
            total_trades_combined: 0,
            avg_trades_per_aggregation: 0.0,
        };

        print_aggregation_stats(&stats); // Should not panic
    }

    #[test]
    fn test_calculate_fees_saved() {
        // Test basic fee calculation
        let result = calculate_fees_saved(100, 30);
        assert!((result - 1.40).abs() < 0.001, "Expected 1.40, got {}", result);

        // Test no aggregation
        assert_eq!(calculate_fees_saved(50, 50), 0.0);

        // Test zero orders
        assert_eq!(calculate_fees_saved(0, 0), 0.0);
    }

    #[test]
    fn test_args_default_values() {
        // Test that default values are set correctly
        let args = Args::parse_from(&["position_monitor"]);
        assert_eq!(args.db, "trades.db");
        assert_eq!(args.once, false);
        assert_eq!(args.stats, false);
        assert_eq!(args.no_prices, false);
        assert_eq!(args.ttl, 30);
    }

    #[test]
    fn test_args_no_prices_flag() {
        let args = Args::parse_from(&["position_monitor", "--no-prices"]);
        assert_eq!(args.no_prices, true);
    }

    #[test]
    fn test_args_custom_ttl() {
        let args = Args::parse_from(&["position_monitor", "--ttl", "60"]);
        assert_eq!(args.ttl, 60);
    }

    #[test]
    fn test_args_combined_flags() {
        let args = Args::parse_from(&[
            "position_monitor",
            "--db", "custom.db",
            "--no-prices",
            "--ttl", "120",
            "--once"
        ]);
        assert_eq!(args.db, "custom.db");
        assert_eq!(args.no_prices, true);
        assert_eq!(args.ttl, 120);
        assert_eq!(args.once, true);
    }

    #[test]
    fn test_calculate_unrealized_pnl_long_profit() {
        // LONG position with profit
        // Bought 150 shares at $0.45, current bid is $0.52
        // P&L = (0.52 - 0.45) * 150 = $10.50
        let pnl = calculate_unrealized_pnl(150.0, Some(0.45), 0.52, 0.53);
        assert!(pnl.is_some());
        let pnl_value = pnl.unwrap();
        assert!((pnl_value - 10.50).abs() < 0.001, "Expected 10.50, got {}", pnl_value);
    }

    #[test]
    fn test_calculate_unrealized_pnl_long_loss() {
        // LONG position with loss
        // Bought 100 shares at $0.60, current bid is $0.50
        // P&L = (0.50 - 0.60) * 100 = -$10.00
        let pnl = calculate_unrealized_pnl(100.0, Some(0.60), 0.50, 0.51);
        assert!(pnl.is_some());
        let pnl_value = pnl.unwrap();
        assert!((pnl_value - (-10.00)).abs() < 0.001, "Expected -10.00, got {}", pnl_value);
    }

    #[test]
    fn test_calculate_unrealized_pnl_short_profit() {
        // SHORT position with profit
        // Sold 50 shares at $0.62, current ask is $0.58
        // P&L = (0.62 - 0.58) * 50 = $2.00
        let pnl = calculate_unrealized_pnl(-50.0, Some(0.62), 0.57, 0.58);
        assert!(pnl.is_some());
        let pnl_value = pnl.unwrap();
        assert!((pnl_value - 2.00).abs() < 0.001, "Expected 2.00, got {}", pnl_value);
    }

    #[test]
    fn test_calculate_unrealized_pnl_short_loss() {
        // SHORT position with loss
        // Sold 30 shares at $0.50, current ask is $0.60
        // P&L = (0.50 - 0.60) * 30 = -$3.00
        let pnl = calculate_unrealized_pnl(-30.0, Some(0.50), 0.59, 0.60);
        assert!(pnl.is_some());
        let pnl_value = pnl.unwrap();
        assert!((pnl_value - (-3.00)).abs() < 0.001, "Expected -3.00, got {}", pnl_value);
    }

    #[test]
    fn test_calculate_unrealized_pnl_no_avg_entry() {
        // Position with no average entry price should return None
        let pnl = calculate_unrealized_pnl(100.0, None, 0.50, 0.51);
        assert!(pnl.is_none());
    }

    #[test]
    fn test_calculate_unrealized_pnl_zero_shares() {
        // Zero shares should give zero P&L
        let pnl = calculate_unrealized_pnl(0.0, Some(0.50), 0.52, 0.53);
        assert!(pnl.is_some());
        let pnl_value = pnl.unwrap();
        assert!((pnl_value - 0.0).abs() < 0.001, "Expected 0.00, got {}", pnl_value);
    }

    #[test]
    fn test_print_table_with_pnl_empty() {
        // Should handle empty positions without panic
        let positions: Vec<PositionWithPrice> = vec![];
        let snapshot = DailySnapshot {
            date: "2026-01-20".to_string(),
            portfolio_value: 0.0,
            cost_basis: 0.0,
            unrealized_pnl: 0.0,
            timestamp: "2026-01-20T00:00:00Z".to_string(),
        };
        print_table_with_pnl(&positions, &snapshot); // Should not panic
    }

    #[test]
    fn test_print_table_with_pnl_with_prices() {
        use std::time::Instant;

        let positions = vec![
            PositionWithPrice {
                position: Position {
                    token_id: "token1".to_string(),
                    net_shares: 100.0,
                    avg_entry_price: Some(0.45),
                    trade_count: 5,
                },
                price_info: Some(PriceInfo {
                    bid_price: 0.52,
                    ask_price: 0.53,
                    timestamp: Instant::now(),
                }),
            }
        ];
        let snapshot = DailySnapshot {
            date: "2026-01-20".to_string(),
            portfolio_value: 50.0,
            cost_basis: 45.0,
            unrealized_pnl: 5.0,
            timestamp: "2026-01-20T00:00:00Z".to_string(),
        };
        print_table_with_pnl(&positions, &snapshot); // Should not panic
    }

    #[test]
    fn test_print_table_with_pnl_missing_prices() {
        let positions = vec![
            PositionWithPrice {
                position: Position {
                    token_id: "token_no_price".to_string(),
                    net_shares: 50.0,
                    avg_entry_price: Some(0.30),
                    trade_count: 2,
                },
                price_info: None, // Missing price data
            }
        ];
        let snapshot = DailySnapshot {
            date: "2026-01-20".to_string(),
            portfolio_value: 0.0,
            cost_basis: 0.0,
            unrealized_pnl: 0.0,
            timestamp: "2026-01-20T00:00:00Z".to_string(),
        };
        print_table_with_pnl(&positions, &snapshot); // Should not panic, should show N/A
    }

    #[test]
    fn test_print_table_with_pnl_mixed() {
        use std::time::Instant;

        let positions = vec![
            PositionWithPrice {
                position: Position {
                    token_id: "token_with_price".to_string(),
                    net_shares: 100.0,
                    avg_entry_price: Some(0.45),
                    trade_count: 5,
                },
                price_info: Some(PriceInfo {
                    bid_price: 0.52,
                    ask_price: 0.53,
                    timestamp: Instant::now(),
                }),
            },
            PositionWithPrice {
                position: Position {
                    token_id: "token_without_price".to_string(),
                    net_shares: 50.0,
                    avg_entry_price: Some(0.30),
                    trade_count: 2,
                },
                price_info: None,
            }
        ];
        let snapshot = DailySnapshot {
            date: "2026-01-20".to_string(),
            portfolio_value: 50.0,
            cost_basis: 45.0,
            unrealized_pnl: 5.0,
            timestamp: "2026-01-20T00:00:00Z".to_string(),
        };
        print_table_with_pnl(&positions, &snapshot); // Should not panic
    }

    // Tests for calculate_position_value()
    #[test]
    fn test_calculate_position_value_long() {
        // LONG position: 100 shares at bid $0.52
        // Value = 100 * 0.52 = $52.00
        let value = calculate_position_value(100.0, 0.52, 0.53);
        assert!((value - 52.00).abs() < 0.001, "Expected 52.00, got {}", value);
    }

    #[test]
    fn test_calculate_position_value_short() {
        // SHORT position: -50 shares at ask $0.58
        // Value = 50 * 0.58 = $29.00 (obligation)
        let value = calculate_position_value(-50.0, 0.57, 0.58);
        assert!((value - 29.00).abs() < 0.001, "Expected 29.00, got {}", value);
    }

    #[test]
    fn test_calculate_position_value_zero() {
        // Zero shares
        let value = calculate_position_value(0.0, 0.50, 0.51);
        assert!((value - 0.0).abs() < 0.001, "Expected 0.00, got {}", value);
    }

    // Tests for calculate_cost_basis()
    #[test]
    fn test_calculate_cost_basis_with_entry() {
        // 100 shares at avg entry $0.45
        // Cost basis = 100 * 0.45 = $45.00
        let basis = calculate_cost_basis(100.0, Some(0.45));
        assert!(basis.is_some());
        let basis_value = basis.unwrap();
        assert!((basis_value - 45.00).abs() < 0.001, "Expected 45.00, got {}", basis_value);
    }

    #[test]
    fn test_calculate_cost_basis_with_entry_short() {
        // -50 shares (short) at avg entry $0.62
        // Cost basis = 50 * 0.62 = $31.00
        let basis = calculate_cost_basis(-50.0, Some(0.62));
        assert!(basis.is_some());
        let basis_value = basis.unwrap();
        assert!((basis_value - 31.00).abs() < 0.001, "Expected 31.00, got {}", basis_value);
    }

    #[test]
    fn test_calculate_cost_basis_no_entry() {
        // No avg entry price
        let basis = calculate_cost_basis(100.0, None);
        assert!(basis.is_none());
    }

    // Tests for calculate_portfolio_summary()
    #[test]
    fn test_portfolio_summary_calculation() {
        use std::time::Instant;

        let positions = vec![
            PositionWithPrice {
                position: Position {
                    token_id: "token1".to_string(),
                    net_shares: 100.0,
                    avg_entry_price: Some(0.45),
                    trade_count: 5,
                },
                price_info: Some(PriceInfo {
                    bid_price: 0.52,
                    ask_price: 0.53,
                    timestamp: Instant::now(),
                }),
            },
            PositionWithPrice {
                position: Position {
                    token_id: "token2".to_string(),
                    net_shares: -50.0,
                    avg_entry_price: Some(0.62),
                    trade_count: 3,
                },
                price_info: Some(PriceInfo {
                    bid_price: 0.57,
                    ask_price: 0.58,
                    timestamp: Instant::now(),
                }),
            },
        ];

        let summary = calculate_portfolio_summary(&positions);

        // Position 1 value: 100 * 0.52 = 52.00
        // Position 2 value: 50 * 0.58 = 29.00
        // Total value: 52.00 + 29.00 = 81.00
        assert!((summary.total_value - 81.00).abs() < 0.001,
            "Expected total_value 81.00, got {}", summary.total_value);

        // Position 1 cost: 100 * 0.45 = 45.00
        // Position 2 cost: 50 * 0.62 = 31.00
        // Total cost: 45.00 + 31.00 = 76.00
        assert!((summary.cost_basis - 76.00).abs() < 0.001,
            "Expected cost_basis 76.00, got {}", summary.cost_basis);

        // Position 1 P&L: (0.52 - 0.45) * 100 = 7.00
        // Position 2 P&L: (0.62 - 0.58) * 50 = 2.00
        // Total P&L: 7.00 + 2.00 = 9.00
        assert!((summary.unrealized_pnl - 9.00).abs() < 0.001,
            "Expected unrealized_pnl 9.00, got {}", summary.unrealized_pnl);

        assert_eq!(summary.position_count, 2);
    }

    #[test]
    fn test_portfolio_summary_empty() {
        let positions: Vec<PositionWithPrice> = vec![];
        let summary = calculate_portfolio_summary(&positions);

        assert!((summary.total_value - 0.0).abs() < 0.001);
        assert!((summary.cost_basis - 0.0).abs() < 0.001);
        assert!((summary.unrealized_pnl - 0.0).abs() < 0.001);
        assert_eq!(summary.position_count, 0);
    }

    #[test]
    fn test_portfolio_summary_no_prices() {
        // Positions without price info should not contribute to summary
        let positions = vec![
            PositionWithPrice {
                position: Position {
                    token_id: "token_no_price".to_string(),
                    net_shares: 100.0,
                    avg_entry_price: Some(0.45),
                    trade_count: 5,
                },
                price_info: None,
            }
        ];

        let summary = calculate_portfolio_summary(&positions);

        assert!((summary.total_value - 0.0).abs() < 0.001);
        assert!((summary.cost_basis - 0.0).abs() < 0.001);
        assert!((summary.unrealized_pnl - 0.0).abs() < 0.001);
        assert_eq!(summary.position_count, 0);
    }

    #[test]
    fn test_portfolio_summary_mixed_prices() {
        use std::time::Instant;

        // Some positions with prices, some without
        let positions = vec![
            PositionWithPrice {
                position: Position {
                    token_id: "token_with_price".to_string(),
                    net_shares: 100.0,
                    avg_entry_price: Some(0.45),
                    trade_count: 5,
                },
                price_info: Some(PriceInfo {
                    bid_price: 0.52,
                    ask_price: 0.53,
                    timestamp: Instant::now(),
                }),
            },
            PositionWithPrice {
                position: Position {
                    token_id: "token_without_price".to_string(),
                    net_shares: 50.0,
                    avg_entry_price: Some(0.30),
                    trade_count: 2,
                },
                price_info: None,
            }
        ];

        let summary = calculate_portfolio_summary(&positions);

        // Only token_with_price should contribute
        // Value: 100 * 0.52 = 52.00
        assert!((summary.total_value - 52.00).abs() < 0.001);
        // Cost: 100 * 0.45 = 45.00
        assert!((summary.cost_basis - 45.00).abs() < 0.001);
        // P&L: (0.52 - 0.45) * 100 = 7.00
        assert!((summary.unrealized_pnl - 7.00).abs() < 0.001);
        assert_eq!(summary.position_count, 1);
    }

    // Tests for Increment 1a: Daily Snapshot Data Structure and Path Resolution

    #[test]
    fn test_daily_snapshot_serialization() {
        // Test that DailySnapshot can be serialized/deserialized
        let snapshot = DailySnapshot {
            date: "2026-01-20".to_string(),
            portfolio_value: 1250.75,
            cost_basis: 1100.00,
            unrealized_pnl: 150.75,
            timestamp: "2026-01-20T23:59:59Z".to_string(),
        };

        let json = serde_json::to_string(&snapshot).expect("Failed to serialize");
        let deserialized: DailySnapshot = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.date, "2026-01-20");
        assert!((deserialized.portfolio_value - 1250.75).abs() < 0.001);
        assert!((deserialized.cost_basis - 1100.00).abs() < 0.001);
        assert!((deserialized.unrealized_pnl - 150.75).abs() < 0.001);
        assert_eq!(deserialized.timestamp, "2026-01-20T23:59:59Z");
    }

    #[test]
    fn test_get_snapshot_path_default() {
        // Default database "trades.db" should produce ".portfolio_snapshot.json" in current dir
        let path = get_snapshot_path("trades.db");
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), ".portfolio_snapshot.json");
        assert_eq!(path.parent().unwrap(), Path::new(""));
    }

    #[test]
    fn test_get_snapshot_path_custom() {
        // Custom path "/path/to/mydata.db" should produce "/path/to/.portfolio_snapshot.json"
        let path = get_snapshot_path("/path/to/mydata.db");
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), ".portfolio_snapshot.json");
        assert_eq!(path.parent().unwrap(), Path::new("/path/to"));
    }

    #[test]
    fn test_get_today_utc_format() {
        // Test that get_today_utc returns a valid ISO date format "YYYY-MM-DD"
        let today = get_today_utc();

        // Should be 10 characters long
        assert_eq!(today.len(), 10);

        // Should match format YYYY-MM-DD
        assert_eq!(today.chars().nth(4).unwrap(), '-');
        assert_eq!(today.chars().nth(7).unwrap(), '-');

        // Should be parseable as a date
        let parts: Vec<&str> = today.split('-').collect();
        assert_eq!(parts.len(), 3);

        let year: i32 = parts[0].parse().expect("Year should be numeric");
        let month: u32 = parts[1].parse().expect("Month should be numeric");
        let day: u32 = parts[2].parse().expect("Day should be numeric");

        assert!(year >= 2020 && year <= 2100, "Year should be reasonable");
        assert!(month >= 1 && month <= 12, "Month should be 1-12");
        assert!(day >= 1 && day <= 31, "Day should be 1-31");
    }

    // Tests for Increment 1b: Daily Snapshot File I/O Operations

    #[test]
    fn test_load_nonexistent_snapshot() {
        // Should return None when file doesn't exist
        let nonexistent_path = Path::new("/tmp/nonexistent_snapshot_12345.json");
        let result = load_daily_snapshot(nonexistent_path);
        assert!(result.is_none());
    }

    #[test]
    fn test_save_and_load_snapshot() {
        use std::fs;

        // Create a temporary snapshot
        let snapshot = DailySnapshot {
            date: "2026-01-20".to_string(),
            portfolio_value: 1250.75,
            cost_basis: 1100.00,
            unrealized_pnl: 150.75,
            timestamp: "2026-01-20T23:59:59Z".to_string(),
        };

        // Save to a temp file
        let temp_path = std::env::temp_dir().join("test_snapshot.json");
        save_daily_snapshot(&temp_path, &snapshot).expect("Failed to save snapshot");

        // Load it back
        let loaded = load_daily_snapshot(&temp_path).expect("Failed to load snapshot");

        // Verify contents
        assert_eq!(loaded.date, "2026-01-20");
        assert!((loaded.portfolio_value - 1250.75).abs() < 0.001);
        assert!((loaded.cost_basis - 1100.00).abs() < 0.001);
        assert!((loaded.unrealized_pnl - 150.75).abs() < 0.001);
        assert_eq!(loaded.timestamp, "2026-01-20T23:59:59Z");

        // Cleanup
        fs::remove_file(&temp_path).ok();
    }

    // Tests for Increment 1c: Snapshot Management and Daily P&L Calculation

    #[test]
    fn test_calculate_daily_pnl_change() {
        let snapshot = DailySnapshot {
            date: "2026-01-20".to_string(),
            portfolio_value: 1000.00,
            cost_basis: 950.00,
            unrealized_pnl: 50.00,
            timestamp: "2026-01-20T00:00:00Z".to_string(),
        };

        // Current P&L is $75, snapshot was $50, change is $25
        let change = calculate_daily_pnl_change(75.00, &snapshot);
        assert!((change - 25.00).abs() < 0.001);

        // Current P&L is $30, snapshot was $50, change is -$20
        let change = calculate_daily_pnl_change(30.00, &snapshot);
        assert!((change - (-20.00)).abs() < 0.001);
    }

    #[test]
    fn test_check_and_update_snapshot_new() {
        use std::fs;

        // Test creating a new snapshot
        let temp_path = std::env::temp_dir().join("test_snapshot_new.json");

        // Ensure file doesn't exist
        fs::remove_file(&temp_path).ok();

        let current_summary = PortfolioSummary {
            total_value: 1250.75,
            cost_basis: 1100.00,
            unrealized_pnl: 150.75,
            position_count: 5,
        };

        let snapshot = check_and_update_snapshot(&temp_path, &current_summary);

        // Should create new snapshot with current values
        assert_eq!(snapshot.date, get_today_utc());
        assert!((snapshot.portfolio_value - 1250.75).abs() < 0.001);
        assert!((snapshot.cost_basis - 1100.00).abs() < 0.001);
        assert!((snapshot.unrealized_pnl - 150.75).abs() < 0.001);

        // Verify it was saved
        assert!(temp_path.exists());

        // Cleanup
        fs::remove_file(&temp_path).ok();
    }

    #[test]
    fn test_check_and_update_snapshot_same_day() {
        use std::fs;

        // Test that snapshot is not updated when called on same day
        let temp_path = std::env::temp_dir().join("test_snapshot_same_day.json");

        // Create initial snapshot for today
        let initial_snapshot = DailySnapshot {
            date: get_today_utc(),
            portfolio_value: 1000.00,
            cost_basis: 950.00,
            unrealized_pnl: 50.00,
            timestamp: "2026-01-21T00:00:00Z".to_string(),
        };
        save_daily_snapshot(&temp_path, &initial_snapshot).unwrap();

        // Now call with different current values
        let current_summary = PortfolioSummary {
            total_value: 1500.00,
            cost_basis: 1200.00,
            unrealized_pnl: 300.00,
            position_count: 10,
        };

        let snapshot = check_and_update_snapshot(&temp_path, &current_summary);

        // Should return the ORIGINAL snapshot (not update)
        assert_eq!(snapshot.date, get_today_utc());
        assert!((snapshot.portfolio_value - 1000.00).abs() < 0.001);
        assert!((snapshot.unrealized_pnl - 50.00).abs() < 0.001);

        // Cleanup
        fs::remove_file(&temp_path).ok();
    }

    // Tests for Increment 2: JSON Output Flag

    #[test]
    fn test_args_json_flag() {
        // Test that --json flag is parsed correctly
        let args = Args::parse_from(&["position_monitor", "--json"]);
        assert_eq!(args.json, true);

        let args = Args::parse_from(&["position_monitor"]);
        assert_eq!(args.json, false);
    }

    #[test]
    fn test_portfolio_json_serialization() {
        use std::time::Instant;

        let positions = vec![
            PositionWithPrice {
                position: Position {
                    token_id: "token1".to_string(),
                    net_shares: 100.0,
                    avg_entry_price: Some(0.45),
                    trade_count: 5,
                },
                price_info: Some(PriceInfo {
                    bid_price: 0.52,
                    ask_price: 0.53,
                    timestamp: Instant::now(),
                }),
            },
        ];

        let summary = PortfolioSummary {
            total_value: 52.00,
            cost_basis: 45.00,
            unrealized_pnl: 7.00,
            position_count: 1,
        };

        let snapshot = DailySnapshot {
            date: "2026-01-20".to_string(),
            portfolio_value: 1000.00,
            cost_basis: 950.00,
            unrealized_pnl: 50.00,
            timestamp: "2026-01-20T00:00:00Z".to_string(),
        };

        let portfolio_json = to_portfolio_json(&positions, &summary, &snapshot);

        // Verify structure
        assert!((portfolio_json.portfolio_value - 52.00).abs() < 0.001);
        assert!((portfolio_json.cost_basis - 45.00).abs() < 0.001);
        assert!((portfolio_json.unrealized_pnl - 7.00).abs() < 0.001);
        assert!((portfolio_json.daily_pnl_change - (-43.00)).abs() < 0.001); // 7 - 50 = -43
        assert_eq!(portfolio_json.snapshot_date, "2026-01-20");
        assert_eq!(portfolio_json.position_count, 1);
        assert_eq!(portfolio_json.positions.len(), 1);

        // Verify position
        let pos = &portfolio_json.positions[0];
        assert_eq!(pos.token_id, "token1");
        assert!((pos.net_shares - 100.0).abs() < 0.001);
        assert_eq!(pos.avg_entry_price, Some(0.45));
        assert_eq!(pos.current_price, Some(0.52));
        assert!((pos.position_value.unwrap() - 52.00).abs() < 0.001);
        assert!((pos.unrealized_pnl.unwrap() - 7.00).abs() < 0.001);

        // Test JSON serialization
        let json_str = serde_json::to_string(&portfolio_json).expect("Failed to serialize");
        assert!(json_str.contains("token1"));
        assert!(json_str.contains("portfolio_value"));
    }
}
