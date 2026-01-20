// trader_comparison.rs - CLI tool for comparing trader performance stats
//
// Usage:
//   cargo run --bin trader_comparison                        # Compare all traders
//   cargo run --bin trader_comparison -- --db test.db        # Use custom database
//   cargo run --bin trader_comparison -- --trader Whale1     # Filter specific trader
//   cargo run --bin trader_comparison -- --format csv        # Output as CSV

use anyhow::Result;
use clap::Parser;
use pm_whale_follower::persistence::TradeStore;

#[derive(Parser)]
#[command(name = "trader_comparison")]
#[command(about = "Compare trader performance statistics")]
struct Args {
    /// Database path
    #[arg(long, default_value = "trades.db")]
    db: String,

    /// Filter to specific trader label
    #[arg(long)]
    trader: Option<String>,

    /// Show trades since Unix timestamp (seconds)
    #[arg(long)]
    since: Option<i64>,

    /// Output format: table, csv, json
    #[arg(long, default_value = "table")]
    format: String,
}

/// Trader statistics with calculated metrics
#[derive(Debug, Clone)]
struct TraderStats {
    address: String,
    label: String,
    observed_trades: u32,      // Total trades detected from this trader
    copied_trades: u32,        // Trades we actually executed (successful + failed)
    successful_trades: u32,    // Trades that succeeded
    failed_trades: u32,        // Trades that failed
    total_copied_usd: f64,     // Total USD we copied
    avg_fill_pct: f64,         // Average fill percentage
}

impl TraderStats {
    /// Calculate copy rate as percentage
    fn copy_rate(&self) -> f64 {
        if self.observed_trades == 0 {
            0.0
        } else {
            (self.copied_trades as f64 / self.observed_trades as f64) * 100.0
        }
    }

    /// Calculate success rate as percentage
    fn success_rate(&self) -> f64 {
        if self.copied_trades == 0 {
            0.0
        } else {
            (self.successful_trades as f64 / self.copied_trades as f64) * 100.0
        }
    }
}

/// Fetch trader statistics from database
fn fetch_trader_stats(
    store: &TradeStore,
    trader_filter: Option<&str>,
    since: Option<i64>,
) -> Result<Vec<TraderStats>> {
    // Get base stats from trader_stats table
    let db_stats = store.get_all_trader_stats()?;

    let mut stats = Vec::new();

    for (address, label, _total, successful, failed, total_usd, _, _) in db_stats {
        // Apply trader filter if specified
        if let Some(filter) = trader_filter {
            if !label.contains(filter) && !address.contains(filter) {
                continue;
            }
        }

        // Query trades for this trader to get observed count and avg fill
        let (observed, avg_fill) = get_trader_trade_metrics(store, &address, since)?;

        // If since filter is specified and this trader has no trades in that period, skip
        if since.is_some() && observed == 0 {
            continue;
        }

        let copied = successful + failed;

        stats.push(TraderStats {
            address: address.clone(),
            label: label.clone(),
            observed_trades: observed,
            copied_trades: copied,
            successful_trades: successful,
            failed_trades: failed,
            total_copied_usd: total_usd,
            avg_fill_pct: avg_fill,
        });
    }

    // Sort by total_copied_usd descending
    stats.sort_by(|a, b| b.total_copied_usd.partial_cmp(&a.total_copied_usd).unwrap());

    Ok(stats)
}

/// Get observed trades count and average fill percentage for a trader
fn get_trader_trade_metrics(
    store: &TradeStore,
    trader_address: &str,
    since: Option<i64>,
) -> Result<(u32, f64)> {
    if let Some(since_ts) = since {
        store.get_trader_trade_metrics_since(trader_address, since_ts)
    } else {
        store.get_trader_trade_metrics(trader_address)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Validate format
    match args.format.to_lowercase().as_str() {
        "table" | "csv" | "json" => {},
        _ => anyhow::bail!("Invalid format: {}. Use table, csv, or json", args.format),
    }

    // Open database
    let store = TradeStore::new(&args.db)?;

    // Fetch trader stats
    let stats = fetch_trader_stats(&store, args.trader.as_deref(), args.since)?;

    if stats.is_empty() {
        println!("No trader data found.");
        return Ok(());
    }

    // Display in requested format
    match args.format.to_lowercase().as_str() {
        "csv" => print_csv(&stats),
        "json" => print_json(&stats)?,
        _ => print_table(&stats),
    }

    Ok(())
}

/// Print trader comparison in table format
fn print_table(stats: &[TraderStats]) {
    println!("\n=== TRADER COMPARISON ===\n");

    if stats.is_empty() {
        println!("No trader data found.");
        return;
    }

    // Print header
    println!(
        "{:<15} {:>10} {:>10} {:>8} {:>10} {:>12} {:>10}",
        "Trader", "Observed", "Copied", "Copy%", "Success%", "Total USD", "Avg Fill"
    );
    println!("{}", "-".repeat(85));

    // Calculate totals
    let mut total_observed = 0u32;
    let mut total_copied = 0u32;
    let mut total_successful = 0u32;
    let mut total_usd = 0.0f64;
    let mut total_fill_sum = 0.0f64;
    let mut fill_count = 0u32;

    // Print each trader
    for stat in stats {
        println!(
            "{:<15} {:>10} {:>10} {:>7.1}% {:>9.1}% {:>11.2} {:>9.1}%",
            truncate_label(&stat.label, 15),
            stat.observed_trades,
            stat.copied_trades,
            stat.copy_rate(),
            stat.success_rate(),
            stat.total_copied_usd,
            stat.avg_fill_pct,
        );

        total_observed += stat.observed_trades;
        total_copied += stat.copied_trades;
        total_successful += stat.successful_trades;
        total_usd += stat.total_copied_usd;
        if stat.avg_fill_pct > 0.0 {
            total_fill_sum += stat.avg_fill_pct;
            fill_count += 1;
        }
    }

    // Print separator
    println!("{}", "-".repeat(85));

    // Calculate aggregate percentages
    let total_copy_rate = if total_observed > 0 {
        (total_copied as f64 / total_observed as f64) * 100.0
    } else {
        0.0
    };

    let total_success_rate = if total_copied > 0 {
        (total_successful as f64 / total_copied as f64) * 100.0
    } else {
        0.0
    };

    let avg_fill = if fill_count > 0 {
        total_fill_sum / fill_count as f64
    } else {
        0.0
    };

    // Print totals
    println!(
        "{:<15} {:>10} {:>10} {:>7.1}% {:>9.1}% {:>11.2} {:>9.1}%",
        "TOTAL",
        total_observed,
        total_copied,
        total_copy_rate,
        total_success_rate,
        total_usd,
        avg_fill,
    );
}

/// Truncate label to max length
fn truncate_label(label: &str, max_len: usize) -> String {
    if label.len() > max_len {
        format!("{}...", &label[..max_len - 3])
    } else {
        label.to_string()
    }
}

/// Print trader comparison in CSV format
fn print_csv(stats: &[TraderStats]) {
    // Print CSV header
    println!("trader,address,observed,copied,copy_pct,success_pct,total_usd,avg_fill_pct");

    // Print each trader
    for stat in stats {
        println!(
            "{},{},{},{},{:.1},{:.1},{:.2},{:.1}",
            stat.label,
            stat.address,
            stat.observed_trades,
            stat.copied_trades,
            stat.copy_rate(),
            stat.success_rate(),
            stat.total_copied_usd,
            stat.avg_fill_pct,
        );
    }
}

/// Print trader comparison in JSON format
fn print_json(stats: &[TraderStats]) -> Result<()> {
    use serde_json::json;

    let json_stats: Vec<_> = stats
        .iter()
        .map(|stat| {
            json!({
                "trader": stat.label,
                "address": stat.address,
                "observed_trades": stat.observed_trades,
                "copied_trades": stat.copied_trades,
                "successful_trades": stat.successful_trades,
                "failed_trades": stat.failed_trades,
                "copy_rate_pct": stat.copy_rate(),
                "success_rate_pct": stat.success_rate(),
                "total_copied_usd": stat.total_copied_usd,
                "avg_fill_pct": stat.avg_fill_pct,
            })
        })
        .collect();

    println!("{}", serde_json::to_string_pretty(&json_stats)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use pm_whale_follower::persistence::TradeRecord;

    #[test]
    fn test_args_defaults() {
        // Test that default values are correctly set
        let args = Args::parse_from(&["trader_comparison"]);
        assert_eq!(args.db, "trades.db");
        assert_eq!(args.format, "table");
        assert!(args.trader.is_none());
        assert!(args.since.is_none());
    }

    #[test]
    fn test_args_custom_db() {
        let args = Args::parse_from(&["trader_comparison", "--db", "test.db"]);
        assert_eq!(args.db, "test.db");
    }

    #[test]
    fn test_args_trader_filter() {
        let args = Args::parse_from(&["trader_comparison", "--trader", "Whale1"]);
        assert_eq!(args.trader, Some("Whale1".to_string()));
    }

    #[test]
    fn test_args_since_filter() {
        let args = Args::parse_from(&["trader_comparison", "--since", "1704067200"]);
        assert_eq!(args.since, Some(1704067200));
    }

    #[test]
    fn test_args_format_csv() {
        let args = Args::parse_from(&["trader_comparison", "--format", "csv"]);
        assert_eq!(args.format, "csv");
    }

    #[test]
    fn test_args_format_json() {
        let args = Args::parse_from(&["trader_comparison", "--format", "json"]);
        assert_eq!(args.format, "json");
    }

    #[test]
    fn test_args_combined() {
        let args = Args::parse_from(&[
            "trader_comparison",
            "--db", "test.db",
            "--trader", "Whale2",
            "--since", "1704067200",
            "--format", "json",
        ]);
        assert_eq!(args.db, "test.db");
        assert_eq!(args.trader, Some("Whale2".to_string()));
        assert_eq!(args.since, Some(1704067200));
        assert_eq!(args.format, "json");
    }

    #[test]
    fn test_trader_stats_copy_rate() {
        let stats = TraderStats {
            address: "0x123".to_string(),
            label: "Test".to_string(),
            observed_trades: 100,
            copied_trades: 80,
            successful_trades: 75,
            failed_trades: 5,
            total_copied_usd: 1000.0,
            avg_fill_pct: 98.5,
        };
        assert_eq!(stats.copy_rate(), 80.0);
    }

    #[test]
    fn test_trader_stats_copy_rate_zero_observed() {
        let stats = TraderStats {
            address: "0x123".to_string(),
            label: "Test".to_string(),
            observed_trades: 0,
            copied_trades: 0,
            successful_trades: 0,
            failed_trades: 0,
            total_copied_usd: 0.0,
            avg_fill_pct: 0.0,
        };
        assert_eq!(stats.copy_rate(), 0.0);
    }

    #[test]
    fn test_trader_stats_success_rate() {
        let stats = TraderStats {
            address: "0x123".to_string(),
            label: "Test".to_string(),
            observed_trades: 100,
            copied_trades: 80,
            successful_trades: 76,
            failed_trades: 4,
            total_copied_usd: 1000.0,
            avg_fill_pct: 98.5,
        };
        assert_eq!(stats.success_rate(), 95.0);
    }

    #[test]
    fn test_trader_stats_success_rate_zero_copied() {
        let stats = TraderStats {
            address: "0x123".to_string(),
            label: "Test".to_string(),
            observed_trades: 10,
            copied_trades: 0,
            successful_trades: 0,
            failed_trades: 0,
            total_copied_usd: 0.0,
            avg_fill_pct: 0.0,
        };
        assert_eq!(stats.success_rate(), 0.0);
    }

    #[test]
    fn test_fetch_trader_stats_empty_database() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = TradeStore::new(temp_file.path()).unwrap();

        let stats = fetch_trader_stats(&store, None, None).unwrap();
        assert_eq!(stats.len(), 0);
    }

    #[test]
    fn test_fetch_trader_stats_with_data() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = TradeStore::new(temp_file.path()).unwrap();

        // Insert trader stats
        store.upsert_trader_stats(
            "0xtrader1",
            "Whale1",
            50,  // total_trades
            45,  // successful_trades
            5,   // failed_trades
            500.0,  // total_copied_usd
            Some(1704067200000),
            1704067200000,
        ).unwrap();

        store.upsert_trader_stats(
            "0xtrader2",
            "Whale2",
            30,  // total_trades
            25,  // successful_trades
            5,   // failed_trades
            300.0,  // total_copied_usd
            Some(1704067200000),
            1704067200000,
        ).unwrap();

        let stats = fetch_trader_stats(&store, None, None).unwrap();
        assert_eq!(stats.len(), 2);

        // Should be sorted by total_copied_usd descending
        assert_eq!(stats[0].label, "Whale1");
        assert_eq!(stats[0].total_copied_usd, 500.0);
        assert_eq!(stats[1].label, "Whale2");
        assert_eq!(stats[1].total_copied_usd, 300.0);
    }

    #[test]
    fn test_fetch_trader_stats_with_filter() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = TradeStore::new(temp_file.path()).unwrap();

        store.upsert_trader_stats(
            "0xtrader1",
            "Whale1",
            50, 45, 5, 500.0,
            Some(1704067200000),
            1704067200000,
        ).unwrap();

        store.upsert_trader_stats(
            "0xtrader2",
            "Whale2",
            30, 25, 5, 300.0,
            Some(1704067200000),
            1704067200000,
        ).unwrap();

        let stats = fetch_trader_stats(&store, Some("Whale1"), None).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].label, "Whale1");
    }

    #[test]
    fn test_fetch_trader_stats_filter_by_address() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = TradeStore::new(temp_file.path()).unwrap();

        store.upsert_trader_stats(
            "0xtrader1",
            "Whale1",
            50, 45, 5, 500.0,
            Some(1704067200000),
            1704067200000,
        ).unwrap();

        store.upsert_trader_stats(
            "0xtrader2",
            "Whale2",
            30, 25, 5, 300.0,
            Some(1704067200000),
            1704067200000,
        ).unwrap();

        let stats = fetch_trader_stats(&store, Some("trader1"), None).unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].address, "0xtrader1");
    }

    #[test]
    fn test_get_trader_trade_metrics_no_trades() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = TradeStore::new(temp_file.path()).unwrap();

        let (observed, avg_fill) = get_trader_trade_metrics(&store, "0xtrader1", None).unwrap();
        assert_eq!(observed, 0);
        assert_eq!(avg_fill, 0.0);
    }

    #[test]
    fn test_get_trader_trade_metrics_with_trades() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = TradeStore::new(temp_file.path()).unwrap();

        // Insert some trades
        let trade1 = TradeRecord {
            timestamp_ms: 1704067200000,
            block_number: 12345678,
            tx_hash: "0xabc123".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token1".to_string(),
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
        };

        let trade2 = TradeRecord {
            timestamp_ms: 1704067260000,
            block_number: 12345679,
            tx_hash: "0xdef456".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token2".to_string(),
            side: "BUY".to_string(),
            whale_shares: 500.0,
            whale_price: 0.55,
            whale_usd: 275.0,
            our_shares: Some(10.0),
            our_price: Some(0.56),
            our_usd: Some(5.6),
            fill_pct: Some(98.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(60),
            is_live: Some(true),
        };

        let trade3 = TradeRecord {
            timestamp_ms: 1704067320000,
            block_number: 12345680,
            tx_hash: "0xghi789".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token3".to_string(),
            side: "SELL".to_string(),
            whale_shares: 200.0,
            whale_price: 0.60,
            whale_usd: 120.0,
            our_shares: None,
            our_price: None,
            our_usd: None,
            fill_pct: None,
            status: "FAILED".to_string(),
            latency_ms: Some(70),
            is_live: Some(true),
        };

        store.insert_trade(&trade1).unwrap();
        store.insert_trade(&trade2).unwrap();
        store.insert_trade(&trade3).unwrap();

        let (observed, avg_fill) = get_trader_trade_metrics(&store, "0xtrader1", None).unwrap();
        assert_eq!(observed, 3);
        assert_eq!(avg_fill, 99.0); // (100 + 98) / 2
    }

    #[test]
    fn test_fetch_trader_stats_with_trades() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = TradeStore::new(temp_file.path()).unwrap();

        // Setup trader stats
        store.upsert_trader_stats(
            "0xtrader1",
            "Whale1",
            5,   // total_trades (not used in our calculation)
            2,   // successful_trades
            1,   // failed_trades
            500.0,
            Some(1704067200000),
            1704067200000,
        ).unwrap();

        // Insert actual trades
        let trade1 = TradeRecord {
            timestamp_ms: 1704067200000,
            block_number: 12345678,
            tx_hash: "0xabc123".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token1".to_string(),
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
        };

        let trade2 = TradeRecord {
            timestamp_ms: 1704067260000,
            block_number: 12345679,
            tx_hash: "0xdef456".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token2".to_string(),
            side: "BUY".to_string(),
            whale_shares: 500.0,
            whale_price: 0.55,
            whale_usd: 275.0,
            our_shares: Some(10.0),
            our_price: Some(0.56),
            our_usd: Some(5.6),
            fill_pct: Some(95.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(60),
            is_live: Some(true),
        };

        let trade3 = TradeRecord {
            timestamp_ms: 1704067320000,
            block_number: 12345680,
            tx_hash: "0xghi789".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token3".to_string(),
            side: "SELL".to_string(),
            whale_shares: 200.0,
            whale_price: 0.60,
            whale_usd: 120.0,
            our_shares: None,
            our_price: None,
            our_usd: None,
            fill_pct: None,
            status: "FAILED".to_string(),
            latency_ms: Some(70),
            is_live: Some(true),
        };

        store.insert_trade(&trade1).unwrap();
        store.insert_trade(&trade2).unwrap();
        store.insert_trade(&trade3).unwrap();

        let stats = fetch_trader_stats(&store, None, None).unwrap();
        assert_eq!(stats.len(), 1);

        let stat = &stats[0];
        assert_eq!(stat.label, "Whale1");
        assert_eq!(stat.observed_trades, 3);  // All trades
        assert_eq!(stat.copied_trades, 3);    // successful + failed = 2 + 1
        assert_eq!(stat.successful_trades, 2);
        assert_eq!(stat.failed_trades, 1);
        assert_eq!(stat.total_copied_usd, 500.0);
        assert_eq!(stat.avg_fill_pct, 97.5);  // (100 + 95) / 2

        // Verify calculated metrics
        assert_eq!(stat.copy_rate(), 100.0);  // 3/3 = 100%
        assert_eq!(stat.success_rate(), 66.66666666666666); // 2/3 = 66.67%
    }

    #[test]
    fn test_fetch_trader_stats_with_since_filter() {
        let temp_file = NamedTempFile::new().unwrap();
        let store = TradeStore::new(temp_file.path()).unwrap();

        // Setup trader stats
        store.upsert_trader_stats(
            "0xtrader1",
            "Whale1",
            3, 2, 1, 500.0,
            Some(1704067320000),
            1704067200000,
        ).unwrap();

        // Insert trades at different timestamps
        let trade1 = TradeRecord {
            timestamp_ms: 1704067200000,  // Jan 1, 2024 00:00:00
            block_number: 12345678,
            tx_hash: "0xabc123".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token1".to_string(),
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
        };

        let trade2 = TradeRecord {
            timestamp_ms: 1704067260000,  // Jan 1, 2024 00:01:00
            block_number: 12345679,
            tx_hash: "0xdef456".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token2".to_string(),
            side: "BUY".to_string(),
            whale_shares: 500.0,
            whale_price: 0.55,
            whale_usd: 275.0,
            our_shares: Some(10.0),
            our_price: Some(0.56),
            our_usd: Some(5.6),
            fill_pct: Some(95.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(60),
            is_live: Some(true),
        };

        let trade3 = TradeRecord {
            timestamp_ms: 1704067320000,  // Jan 1, 2024 00:02:00
            block_number: 12345680,
            tx_hash: "0xghi789".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token3".to_string(),
            side: "SELL".to_string(),
            whale_shares: 200.0,
            whale_price: 0.60,
            whale_usd: 120.0,
            our_shares: None,
            our_price: None,
            our_usd: None,
            fill_pct: None,
            status: "FAILED".to_string(),
            latency_ms: Some(70),
            is_live: Some(true),
        };

        store.insert_trade(&trade1).unwrap();
        store.insert_trade(&trade2).unwrap();
        store.insert_trade(&trade3).unwrap();

        // Filter to show only trades after 00:01:00 (1704067260 seconds)
        let stats = fetch_trader_stats(&store, None, Some(1704067260)).unwrap();
        assert_eq!(stats.len(), 1);

        let stat = &stats[0];
        assert_eq!(stat.observed_trades, 2);  // Only trades 2 and 3
        assert_eq!(stat.avg_fill_pct, 95.0);  // Only trade2 has fill_pct
    }

    #[test]
    fn test_truncate_label_short() {
        assert_eq!(truncate_label("Whale1", 15), "Whale1");
    }

    #[test]
    fn test_truncate_label_exact() {
        let label = "ExactLength1234"; // 15 chars
        assert_eq!(truncate_label(label, 15), label);
    }

    #[test]
    fn test_truncate_label_long() {
        let label = "VeryLongTraderName12345";
        let result = truncate_label(label, 15);
        assert_eq!(result.len(), 15);
        assert!(result.ends_with("..."));
        assert_eq!(result, "VeryLongTrad...");
    }

    #[test]
    fn test_print_table_empty() {
        let stats: Vec<TraderStats> = vec![];
        print_table(&stats); // Should not panic
    }

    #[test]
    fn test_print_table_single_trader() {
        let stats = vec![
            TraderStats {
                address: "0x123".to_string(),
                label: "Whale1".to_string(),
                observed_trades: 100,
                copied_trades: 90,
                successful_trades: 85,
                failed_trades: 5,
                total_copied_usd: 1000.0,
                avg_fill_pct: 98.5,
            }
        ];
        print_table(&stats); // Should not panic
    }

    #[test]
    fn test_print_table_multiple_traders() {
        let stats = vec![
            TraderStats {
                address: "0x123".to_string(),
                label: "Whale1".to_string(),
                observed_trades: 100,
                copied_trades: 90,
                successful_trades: 85,
                failed_trades: 5,
                total_copied_usd: 1000.0,
                avg_fill_pct: 98.5,
            },
            TraderStats {
                address: "0x456".to_string(),
                label: "Whale2".to_string(),
                observed_trades: 50,
                copied_trades: 40,
                successful_trades: 38,
                failed_trades: 2,
                total_copied_usd: 500.0,
                avg_fill_pct: 97.0,
            }
        ];
        print_table(&stats); // Should not panic
    }

    #[test]
    fn test_print_table_zero_values() {
        let stats = vec![
            TraderStats {
                address: "0x123".to_string(),
                label: "Whale1".to_string(),
                observed_trades: 0,
                copied_trades: 0,
                successful_trades: 0,
                failed_trades: 0,
                total_copied_usd: 0.0,
                avg_fill_pct: 0.0,
            }
        ];
        print_table(&stats); // Should not panic
    }

    #[test]
    fn test_print_csv_empty() {
        let stats: Vec<TraderStats> = vec![];
        print_csv(&stats); // Should not panic
    }

    #[test]
    fn test_print_csv_single_trader() {
        let stats = vec![
            TraderStats {
                address: "0x123".to_string(),
                label: "Whale1".to_string(),
                observed_trades: 100,
                copied_trades: 90,
                successful_trades: 85,
                failed_trades: 5,
                total_copied_usd: 1000.0,
                avg_fill_pct: 98.5,
            }
        ];
        print_csv(&stats); // Should not panic
    }

    #[test]
    fn test_print_csv_multiple_traders() {
        let stats = vec![
            TraderStats {
                address: "0x123".to_string(),
                label: "Whale1".to_string(),
                observed_trades: 100,
                copied_trades: 90,
                successful_trades: 85,
                failed_trades: 5,
                total_copied_usd: 1000.0,
                avg_fill_pct: 98.5,
            },
            TraderStats {
                address: "0x456".to_string(),
                label: "Whale2".to_string(),
                observed_trades: 50,
                copied_trades: 40,
                successful_trades: 38,
                failed_trades: 2,
                total_copied_usd: 500.0,
                avg_fill_pct: 97.0,
            }
        ];
        print_csv(&stats); // Should not panic
    }

    #[test]
    fn test_print_json_empty() {
        let stats: Vec<TraderStats> = vec![];
        assert!(print_json(&stats).is_ok());
    }

    #[test]
    fn test_print_json_single_trader() {
        let stats = vec![
            TraderStats {
                address: "0x123".to_string(),
                label: "Whale1".to_string(),
                observed_trades: 100,
                copied_trades: 90,
                successful_trades: 85,
                failed_trades: 5,
                total_copied_usd: 1000.0,
                avg_fill_pct: 98.5,
            }
        ];
        assert!(print_json(&stats).is_ok());
    }

    #[test]
    fn test_print_json_multiple_traders() {
        let stats = vec![
            TraderStats {
                address: "0x123".to_string(),
                label: "Whale1".to_string(),
                observed_trades: 100,
                copied_trades: 90,
                successful_trades: 85,
                failed_trades: 5,
                total_copied_usd: 1000.0,
                avg_fill_pct: 98.5,
            },
            TraderStats {
                address: "0x456".to_string(),
                label: "Whale2".to_string(),
                observed_trades: 50,
                copied_trades: 40,
                successful_trades: 38,
                failed_trades: 2,
                total_copied_usd: 500.0,
                avg_fill_pct: 97.0,
            }
        ];
        assert!(print_json(&stats).is_ok());
    }
}
