// position_monitor.rs - CLI tool for monitoring current positions from trade database
//
// Usage:
//   cargo run --bin position_monitor                    # Monitor with live P&L
//   cargo run --bin position_monitor -- --db test.db    # Use custom database
//   cargo run --bin position_monitor -- --no-prices     # Show positions without prices/P&L
//   cargo run --bin position_monitor -- --ttl 60        # Set price cache TTL to 60 seconds
//   cargo run --bin position_monitor -- --once          # Single snapshot and exit
//   cargo run --bin position_monitor -- --stats         # Show aggregation statistics

use anyhow::Result;
use clap::Parser;
use pm_whale_follower::persistence::{TradeStore, Position, AggregationStats};
use pm_whale_follower::prices::{PriceCache, PriceInfo};

/// Position enriched with price information for P&L calculation
#[derive(Debug, Clone)]
struct PositionWithPrice {
    position: Position,
    price_info: Option<PriceInfo>,
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

        // Display positions with P&L
        print_table_with_pnl(&positions_with_prices);
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
fn print_table_with_pnl(positions: &[PositionWithPrice]) {
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
        print_table_with_pnl(&positions); // Should not panic
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
        print_table_with_pnl(&positions); // Should not panic
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
        print_table_with_pnl(&positions); // Should not panic, should show N/A
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
        print_table_with_pnl(&positions); // Should not panic
    }
}
