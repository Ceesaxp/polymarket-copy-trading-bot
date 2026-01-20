// position_monitor.rs - CLI tool for monitoring current positions from trade database
//
// Usage:
//   cargo run --bin position_monitor                 # Monitor with default database
//   cargo run --bin position_monitor -- --db test.db  # Use custom database
//   cargo run --bin position_monitor -- --once        # Single snapshot and exit
//   cargo run --bin position_monitor -- --stats       # Show aggregation statistics

use anyhow::Result;
use clap::Parser;
use pm_whale_follower::persistence::{TradeStore, Position, AggregationStats};

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

        // Display positions
        print_table(&positions);
    }

    Ok(())
}

/// Print positions in a formatted table
fn print_table(positions: &[Position]) {
    println!("\n=== CURRENT POSITIONS ===\n");

    if positions.is_empty() {
        println!("No positions found.");
        return;
    }

    // Print header
    println!("{:<20} {:>12} {:>12} {:>10}",
        "Token ID", "Net Shares", "Avg Price", "Trades");
    println!("{}", "-".repeat(60));

    // Print each position
    for pos in positions {
        let avg_price_str = pos.avg_entry_price
            .map(|p| format!("{:.4}", p))
            .unwrap_or_else(|| "N/A".to_string());

        println!("{:<20} {:>12.2} {:>12} {:>10}",
            truncate_token_id(&pos.token_id),
            pos.net_shares,
            avg_price_str,
            pos.trade_count
        );
    }

    println!("\nTotal positions: {}", positions.len());
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
    fn test_print_table_empty() {
        // This test verifies that print_table handles empty positions gracefully
        // We can't easily test stdout, but we can ensure it doesn't panic
        let positions: Vec<Position> = vec![];
        print_table(&positions); // Should not panic
    }

    #[test]
    fn test_print_table_single_position() {
        let positions = vec![
            Position {
                token_id: "test_token_123".to_string(),
                net_shares: 100.0,
                avg_entry_price: Some(0.5),
                trade_count: 5,
            }
        ];
        print_table(&positions); // Should not panic
    }

    #[test]
    fn test_print_table_multiple_positions() {
        let positions = vec![
            Position {
                token_id: "token1".to_string(),
                net_shares: 100.0,
                avg_entry_price: Some(0.5),
                trade_count: 5,
            },
            Position {
                token_id: "token2_very_long_id_12345678".to_string(),
                net_shares: -50.0,
                avg_entry_price: None,
                trade_count: 2,
            }
        ];
        print_table(&positions); // Should not panic
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
}
