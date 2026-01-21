// live_positions.rs - CLI tool for fetching live positions from Polymarket Data API
//
// Usage:
//   cargo run --bin live_positions -- 0x1234...                    # Fetch positions for address
//   cargo run --bin live_positions -- --address 0x1234...          # Same as above
//   cargo run --bin live_positions                                  # Use TRADER_ADDRESSES or traders.json
//   cargo run --bin live_positions -- --format json                # Output as JSON
//   cargo run --bin live_positions -- --limit 10                   # Limit to 10 positions

use anyhow::Result;
use clap::Parser;
use pm_whale_follower::config::traders::TradersConfig;
use pm_whale_follower::live_positions::{
    fetch_live_positions_with_options, FetchOptions, LivePosition, LivePositionsSummary,
};

#[derive(Parser)]
#[command(name = "live_positions")]
#[command(about = "Fetch live positions from Polymarket Data API")]
struct Args {
    /// Wallet address to fetch positions for (overrides config)
    #[arg(index = 1)]
    address_positional: Option<String>,

    /// Wallet address to fetch positions for (overrides config)
    #[arg(long)]
    address: Option<String>,

    /// Maximum number of positions to return (default: 100, max: 500)
    #[arg(long)]
    limit: Option<u32>,

    /// Minimum position size threshold
    #[arg(long)]
    min_size: Option<f64>,

    /// Output format: table, csv, json
    #[arg(long, default_value = "table")]
    format: String,

    /// Show summary only (no individual positions)
    #[arg(long)]
    summary_only: bool,

    /// Fetch positions for all configured traders
    #[arg(long)]
    all_traders: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Validate format
    match args.format.to_lowercase().as_str() {
        "table" | "csv" | "json" => {}
        _ => anyhow::bail!("Invalid format: {}. Use table, csv, or json", args.format),
    }

    // Determine addresses to fetch
    let addresses = get_addresses(&args)?;

    if addresses.is_empty() {
        anyhow::bail!("No addresses specified. Use --address or configure traders.");
    }

    // Build fetch options
    let options = FetchOptions::new()
        .with_limit(args.limit.unwrap_or(100))
        .with_size_threshold(args.min_size.unwrap_or(0.01));

    // Fetch positions for each address
    let mut all_positions: Vec<(String, Vec<LivePosition>)> = Vec::new();

    for (address, label) in &addresses {
        match fetch_live_positions_with_options(address, &options) {
            Ok(positions) => {
                if !positions.is_empty() || addresses.len() == 1 {
                    all_positions.push((label.clone(), positions));
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to fetch positions for {}: {}", label, e);
            }
        }
    }

    // Display results
    match args.format.to_lowercase().as_str() {
        "csv" => print_csv(&all_positions, args.summary_only),
        "json" => print_json(&all_positions, args.summary_only)?,
        _ => print_table(&all_positions, args.summary_only),
    }

    Ok(())
}

/// Get addresses to fetch from args or config
fn get_addresses(args: &Args) -> Result<Vec<(String, String)>> {
    // Priority 1: Explicit address from args
    if let Some(addr) = args.address_positional.as_ref().or(args.address.as_ref()) {
        return Ok(vec![(addr.clone(), "Wallet".to_string())]);
    }

    // Priority 2: Load from config (only if --all-traders or no address specified)
    if args.all_traders {
        match TradersConfig::load() {
            Ok(config) => {
                let addresses: Vec<(String, String)> = config
                    .iter()
                    .filter(|t| t.enabled)
                    .map(|t| (format!("0x{}", t.address), t.label.clone()))
                    .collect();
                return Ok(addresses);
            }
            Err(e) => {
                anyhow::bail!("Failed to load trader config: {}", e);
            }
        }
    }

    // Priority 3: Try to load single trader from env
    if let Ok(addr) = std::env::var("TARGET_WHALE_ADDRESS") {
        if !addr.trim().is_empty() {
            return Ok(vec![(addr, "Whale".to_string())]);
        }
    }

    // Priority 4: Try to load from TRADER_ADDRESSES
    if let Ok(addresses) = std::env::var("TRADER_ADDRESSES") {
        let addrs: Vec<(String, String)> = addresses
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .enumerate()
            .map(|(i, addr)| (addr.trim().to_string(), format!("Trader{}", i + 1)))
            .collect();
        if !addrs.is_empty() {
            return Ok(addrs);
        }
    }

    // No addresses found
    Ok(vec![])
}

/// Print positions in table format
fn print_table(all_positions: &[(String, Vec<LivePosition>)], summary_only: bool) {
    if all_positions.is_empty() {
        println!("No positions found.");
        return;
    }

    for (label, positions) in all_positions {
        println!("\n=== {} LIVE POSITIONS ===\n", label.to_uppercase());

        if positions.is_empty() {
            println!("No open positions.");
            continue;
        }

        // Calculate summary
        let summary = LivePositionsSummary::from_positions(positions);

        if !summary_only {
            // Print header
            println!(
                "{:<40} {:>8} {:>10} {:>10} {:>10} {:>8}",
                "Market", "Side", "Shares", "Avg Price", "Value", "P&L %"
            );
            println!("{}", "-".repeat(96));

            // Print each position
            for pos in positions {
                let pnl_indicator = if pos.percent_pnl >= 0.0 { "+" } else { "" };
                println!(
                    "{:<40} {:>8} {:>10.2} {:>10.4} {:>10.2} {:>7}{:.1}%",
                    truncate_str(&pos.title, 40),
                    pos.outcome,
                    pos.size,
                    pos.avg_price,
                    pos.current_value,
                    pnl_indicator,
                    pos.percent_pnl,
                );
            }

            println!("{}", "-".repeat(96));
        }

        // Print summary
        let total_pnl_pct = if summary.total_value > 0.0 {
            (summary.total_pnl / (summary.total_value - summary.total_pnl)) * 100.0
        } else {
            0.0
        };
        let pnl_indicator = if summary.total_pnl >= 0.0 { "+" } else { "" };

        println!("\nSummary:");
        println!("  Positions:     {}", summary.position_count);
        println!("  Total Value:   ${:.2}", summary.total_value);
        println!(
            "  Unrealized PnL: {}${:.2} ({}{:.1}%)",
            pnl_indicator,
            summary.total_pnl.abs(),
            pnl_indicator,
            total_pnl_pct
        );
        println!(
            "  Realized PnL:  ${:.2}",
            summary.total_realized_pnl
        );
        println!(
            "  Win/Loss:      {} / {}",
            summary.profitable_count, summary.losing_count
        );
    }
}

/// Truncate string to max length
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len - 3])
    } else {
        s.to_string()
    }
}

/// Print positions in CSV format
fn print_csv(all_positions: &[(String, Vec<LivePosition>)], summary_only: bool) {
    if summary_only {
        // Print summary CSV
        println!("trader,positions,total_value,unrealized_pnl,realized_pnl,profitable,losing");
        for (label, positions) in all_positions {
            let summary = LivePositionsSummary::from_positions(positions);
            println!(
                "{},{},{:.2},{:.2},{:.2},{},{}",
                label,
                summary.position_count,
                summary.total_value,
                summary.total_pnl,
                summary.total_realized_pnl,
                summary.profitable_count,
                summary.losing_count,
            );
        }
    } else {
        // Print detailed CSV
        println!(
            "trader,market,outcome,condition_id,size,avg_price,current_value,cash_pnl,percent_pnl"
        );
        for (label, positions) in all_positions {
            for pos in positions {
                println!(
                    "{},{},{},{},{:.4},{:.4},{:.2},{:.2},{:.2}",
                    label,
                    escape_csv(&pos.title),
                    pos.outcome,
                    pos.condition_id,
                    pos.size,
                    pos.avg_price,
                    pos.current_value,
                    pos.cash_pnl,
                    pos.percent_pnl,
                );
            }
        }
    }
}

/// Escape CSV field
fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Print positions in JSON format
fn print_json(all_positions: &[(String, Vec<LivePosition>)], summary_only: bool) -> Result<()> {
    use serde_json::json;

    if summary_only {
        let summaries: Vec<_> = all_positions
            .iter()
            .map(|(label, positions)| {
                let summary = LivePositionsSummary::from_positions(positions);
                json!({
                    "trader": label,
                    "position_count": summary.position_count,
                    "total_value": summary.total_value,
                    "unrealized_pnl": summary.total_pnl,
                    "realized_pnl": summary.total_realized_pnl,
                    "profitable_count": summary.profitable_count,
                    "losing_count": summary.losing_count,
                })
            })
            .collect();

        println!("{}", serde_json::to_string_pretty(&summaries)?);
    } else {
        let output: Vec<_> = all_positions
            .iter()
            .map(|(label, positions)| {
                let summary = LivePositionsSummary::from_positions(positions);
                json!({
                    "trader": label,
                    "summary": {
                        "position_count": summary.position_count,
                        "total_value": summary.total_value,
                        "unrealized_pnl": summary.total_pnl,
                        "realized_pnl": summary.total_realized_pnl,
                        "profitable_count": summary.profitable_count,
                        "losing_count": summary.losing_count,
                    },
                    "positions": positions,
                })
            })
            .collect();

        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_address_positional() {
        let args = Args::parse_from(&["live_positions", "0x1234567890abcdef1234567890abcdef12345678"]);
        assert_eq!(
            args.address_positional,
            Some("0x1234567890abcdef1234567890abcdef12345678".to_string())
        );
    }

    #[test]
    fn test_args_address_flag() {
        let args = Args::parse_from(&[
            "live_positions",
            "--address",
            "0x1234567890abcdef1234567890abcdef12345678",
        ]);
        assert_eq!(
            args.address,
            Some("0x1234567890abcdef1234567890abcdef12345678".to_string())
        );
    }

    #[test]
    fn test_args_limit() {
        let args = Args::parse_from(&["live_positions", "--limit", "50"]);
        assert_eq!(args.limit, Some(50));
    }

    #[test]
    fn test_args_min_size() {
        let args = Args::parse_from(&["live_positions", "--min-size", "0.5"]);
        assert_eq!(args.min_size, Some(0.5));
    }

    #[test]
    fn test_args_format() {
        let args = Args::parse_from(&["live_positions", "--format", "json"]);
        assert_eq!(args.format, "json");
    }

    #[test]
    fn test_args_summary_only() {
        let args = Args::parse_from(&["live_positions", "--summary-only"]);
        assert!(args.summary_only);
    }

    #[test]
    fn test_args_all_traders() {
        let args = Args::parse_from(&["live_positions", "--all-traders"]);
        assert!(args.all_traders);
    }

    #[test]
    fn test_args_combined() {
        let args = Args::parse_from(&[
            "live_positions",
            "0xabc",
            "--limit",
            "25",
            "--format",
            "csv",
            "--summary-only",
        ]);
        assert_eq!(args.address_positional, Some("0xabc".to_string()));
        assert_eq!(args.limit, Some(25));
        assert_eq!(args.format, "csv");
        assert!(args.summary_only);
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("Short", 10), "Short");
    }

    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("1234567890", 10), "1234567890");
    }

    #[test]
    fn test_truncate_str_long() {
        let result = truncate_str("This is a very long market title", 20);
        assert_eq!(result.len(), 20);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_escape_csv_simple() {
        assert_eq!(escape_csv("simple"), "simple");
    }

    #[test]
    fn test_escape_csv_comma() {
        assert_eq!(escape_csv("has,comma"), "\"has,comma\"");
    }

    #[test]
    fn test_escape_csv_quote() {
        assert_eq!(escape_csv("has\"quote"), "\"has\"\"quote\"");
    }

    #[test]
    fn test_escape_csv_newline() {
        assert_eq!(escape_csv("has\nnewline"), "\"has\nnewline\"");
    }
}
