// position_monitor.rs - CLI tool for monitoring current positions from trade database
//
// Usage:
//   cargo run --bin position_monitor                 # Monitor with default database
//   cargo run --bin position_monitor -- --db test.db  # Use custom database
//   cargo run --bin position_monitor -- --once        # Single snapshot and exit

use anyhow::Result;
use clap::Parser;
use pm_whale_follower::persistence::{TradeStore, Position};

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Open database read-only
    let store = TradeStore::new(&args.db)?;

    // Fetch positions
    let positions = store.get_positions()?;

    // Display positions
    print_table(&positions);

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
}
