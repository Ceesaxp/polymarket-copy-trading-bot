// clob_history.rs - Complete trade history from CLOB API with PnL calculations
//
// Usage:
//   cargo run --bin clob_history                    # Show all positions with PnL
//   cargo run --bin clob_history -- --trades        # Show individual trades
//   cargo run --bin clob_history -- --activities    # Show all activities (TRADE, MERGE, REDEEM)
//   cargo run --bin clob_history -- --activities --activity-type merge  # Filter by type
//   cargo run --bin clob_history -- --market <id>   # Filter by market/condition ID
//   cargo run --bin clob_history -- --format json   # Output as JSON
//   cargo run --bin clob_history -- --reconcile     # Compare with position API

use anyhow::Result;
use clap::Parser;
use dotenvy::dotenv;
use pm_whale_follower::clob_trades::{
    build_positions_from_trades, calculate_summary, enrich_with_activities, enrich_with_position_api,
    fetch_all_activities, fetch_all_clob_trades, Activity, ActivitySummary, ActivityType,
    ClobTrade, PositionFromTrades, TradeSummary,
};
use pm_whale_follower::live_positions::fetch_live_positions;
use pm_whale_follower::{ApiCreds, PreparedCreds, RustClobClient};
use std::path::Path;

const CLOB_API_BASE: &str = "https://clob.polymarket.com";

#[derive(Parser)]
#[command(name = "clob_history")]
#[command(about = "Fetch complete trade history from CLOB API with PnL calculations")]
struct Args {
    /// Show individual trades instead of positions
    #[arg(long)]
    trades: bool,

    /// Show all activities (TRADE, MERGE, REDEEM) from Data API
    #[arg(long)]
    activities: bool,

    /// Filter by market/condition ID (partial match)
    #[arg(long)]
    market: Option<String>,

    /// Filter by title (partial match, case-insensitive)
    #[arg(long)]
    title: Option<String>,

    /// Output format: table, json, csv
    #[arg(long, default_value = "table")]
    format: String,

    /// Compare with position API and show discrepancies
    #[arg(long)]
    reconcile: bool,

    /// Show only positions with unexplained shares
    #[arg(long)]
    unexplained_only: bool,

    /// Limit number of results
    #[arg(long)]
    limit: Option<usize>,

    /// Sort by: pnl, value, shares, trades (default: value)
    #[arg(long, default_value = "value")]
    sort: String,

    /// Filter activities by type: trade, merge, redeem (only with --activities)
    #[arg(long, value_name = "TYPE")]
    activity_type: Option<String>,
}

fn main() -> Result<()> {
    dotenv().ok();
    let args = Args::parse();

    // Validate format
    match args.format.to_lowercase().as_str() {
        "table" | "json" | "csv" => {}
        _ => anyhow::bail!("Invalid format: {}. Use table, json, or csv", args.format),
    }

    let private_key = std::env::var("PRIVATE_KEY")?;
    let funder = std::env::var("FUNDER_ADDRESS").ok();
    let wallet_address = funder.as_deref().unwrap_or("");

    // Handle --activities mode (uses Data API, no auth needed)
    if args.activities {
        println!("Fetching activities from Data API...");
        let activities = fetch_all_activities(wallet_address)?;
        println!("Fetched {} activities\n", activities.len());

        // Filter by type if specified
        let type_filter: Option<ActivityType> = args.activity_type.as_ref().and_then(|t| {
            match t.to_lowercase().as_str() {
                "trade" => Some(ActivityType::Trade),
                "merge" => Some(ActivityType::Merge),
                "redeem" => Some(ActivityType::Redeem),
                _ => None,
            }
        });

        let filtered_activities: Vec<_> = activities
            .iter()
            .filter(|a| {
                // Filter by type
                if let Some(ref filter_type) = type_filter {
                    if a.activity_type != *filter_type {
                        return false;
                    }
                }
                // Filter by market
                if let Some(ref market) = args.market {
                    if !a.condition_id.contains(market) {
                        return false;
                    }
                }
                // Filter by title
                if let Some(ref title) = args.title {
                    if !a.title.to_lowercase().contains(&title.to_lowercase()) {
                        return false;
                    }
                }
                true
            })
            .collect();

        let summary = ActivitySummary::from_activities(&filtered_activities.iter().cloned().cloned().collect::<Vec<_>>());

        match args.format.to_lowercase().as_str() {
            "json" => print_activities_json(&filtered_activities, &summary)?,
            "csv" => print_activities_csv(&filtered_activities),
            _ => print_activities_table(&filtered_activities, &summary),
        }

        return Ok(());
    }

    // Build CLOB client (for trades/positions mode)
    let client = RustClobClient::new(CLOB_API_BASE, 137, &private_key, funder.as_deref())?;

    // Load credentials
    let creds_path = ".clob_creds.json";
    let creds: ApiCreds = if Path::new(creds_path).exists() {
        let data = std::fs::read_to_string(creds_path)?;
        serde_json::from_str(&data)?
    } else {
        anyhow::bail!("No credentials found at {}. Run the main bot first.", creds_path);
    };

    let prepared = PreparedCreds::from_api_creds(&creds)?;

    // Fetch all trades from CLOB API
    println!("Fetching trades from CLOB API...");
    let trades = fetch_all_clob_trades(&client, &prepared, wallet_address)?;
    println!("Fetched {} trades\n", trades.len());

    // Build positions from trades
    let mut positions = build_positions_from_trades(&trades);

    // Fetch and enrich with position API data
    println!("Fetching current positions from Data API...");
    let api_positions = fetch_live_positions(wallet_address).unwrap_or_default();
    println!("Fetched {} current positions\n", api_positions.len());

    enrich_with_position_api(&mut positions, &api_positions);

    // If reconciling, also fetch activities to explain merges/redeems
    if args.reconcile {
        println!("Fetching activities (MERGE/REDEEM) from Data API...");
        if let Ok(activities) = fetch_all_activities(wallet_address) {
            let merge_count = activities.iter().filter(|a| a.activity_type == ActivityType::Merge).count();
            let redeem_count = activities.iter().filter(|a| a.activity_type == ActivityType::Redeem).count();
            println!("Found {} merges and {} redemptions\n", merge_count, redeem_count);
            enrich_with_activities(&mut positions, &activities);
        }
    }

    // Apply filters
    let mut filtered_positions: Vec<_> = positions
        .values()
        .filter(|p| {
            // Filter by market ID
            if let Some(ref market) = args.market {
                if !p.condition_id.contains(market) {
                    return false;
                }
            }
            // Filter by title
            if let Some(ref title) = args.title {
                if !p.title.to_lowercase().contains(&title.to_lowercase()) {
                    return false;
                }
            }
            // Filter unexplained only
            if args.unexplained_only && p.unexplained_shares().abs() < 0.01 {
                return false;
            }
            true
        })
        .cloned()
        .collect();

    // Sort
    match args.sort.as_str() {
        "pnl" => filtered_positions.sort_by(|a, b| {
            b.total_pnl().partial_cmp(&a.total_pnl()).unwrap_or(std::cmp::Ordering::Equal)
        }),
        "value" => filtered_positions.sort_by(|a, b| {
            b.current_value().partial_cmp(&a.current_value()).unwrap_or(std::cmp::Ordering::Equal)
        }),
        "shares" => filtered_positions.sort_by(|a, b| {
            b.net_shares.partial_cmp(&a.net_shares).unwrap_or(std::cmp::Ordering::Equal)
        }),
        "trades" => filtered_positions.sort_by(|a, b| {
            (b.buy_count + b.sell_count).cmp(&(a.buy_count + a.sell_count))
        }),
        _ => {}
    }

    // Apply limit
    if let Some(limit) = args.limit {
        filtered_positions.truncate(limit);
    }

    // Calculate summary
    let summary = calculate_summary(&positions, trades.len());

    // Output
    if args.trades {
        // Filter trades
        let filtered_trades: Vec<_> = trades
            .iter()
            .filter(|t| {
                if let Some(ref market) = args.market {
                    if !t.market.contains(market) {
                        return false;
                    }
                }
                true
            })
            .collect();

        match args.format.to_lowercase().as_str() {
            "json" => print_trades_json(&filtered_trades)?,
            "csv" => print_trades_csv(&filtered_trades),
            _ => print_trades_table(&filtered_trades),
        }
    } else {
        match args.format.to_lowercase().as_str() {
            "json" => print_positions_json(&filtered_positions, &summary, args.reconcile)?,
            "csv" => print_positions_csv(&filtered_positions, args.reconcile),
            _ => print_positions_table(&filtered_positions, &summary, args.reconcile),
        }
    }

    Ok(())
}

fn print_trades_table(trades: &[&ClobTrade]) {
    println!("=== TRADE HISTORY ===\n");

    println!(
        "{:<19} {:<6} {:>10} {:>8} {:>10} {:<8} {:<30}",
        "Time", "Side", "Shares", "Price", "Cost", "Role", "Market"
    );
    println!("{}", "-".repeat(100));

    for trade in trades.iter().rev().take(50) {
        let time = chrono::DateTime::from_timestamp(trade.match_time, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| trade.match_time.to_string());

        let title = trade.title.as_deref().unwrap_or(&trade.market[..20.min(trade.market.len())]);

        println!(
            "{:<19} {:<6} {:>10.2} {:>8.4} {:>10.2} {:<8} {:<30}",
            time,
            trade.side,
            trade.size,
            trade.price,
            trade.cost(),
            trade.trader_side,
            truncate(title, 30),
        );
    }

    if trades.len() > 50 {
        println!("\n... and {} more trades (use --limit to show more)", trades.len() - 50);
    }

    println!("\nTotal: {} trades", trades.len());
}

fn print_trades_csv(trades: &[&ClobTrade]) {
    println!("timestamp,side,shares,price,cost,role,market,tx_hash");
    for trade in trades {
        println!(
            "{},{},{:.4},{:.4},{:.2},{},{},{}",
            trade.match_time,
            trade.side,
            trade.size,
            trade.price,
            trade.cost(),
            trade.trader_side,
            trade.market,
            trade.transaction_hash,
        );
    }
}

fn print_trades_json(trades: &[&ClobTrade]) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(trades)?);
    Ok(())
}

fn print_positions_table(positions: &[PositionFromTrades], summary: &TradeSummary, reconcile: bool) {
    println!("=== POSITIONS FROM CLOB TRADES ===\n");

    if reconcile {
        println!(
            "{:<30} {:>6} {:>9} {:>7} {:>7} {:>7} {:>9} {:>9}",
            "Market", "Side", "API", "Trades", "Merge", "Redeem", "UnexPlains", "Status"
        );
    } else {
        println!(
            "{:<35} {:>8} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "Market", "Outcome", "Shares", "Avg Price", "Value", "Realized", "Unrealized"
        );
    }
    println!("{}", "-".repeat(105));

    for pos in positions {
        if pos.net_shares.abs() < 0.01 && pos.realized_pnl.abs() < 0.01 {
            continue; // Skip closed positions with no realized PnL
        }

        let realized_str = format_pnl(pos.realized_pnl);
        let unrealized_str = format_pnl(pos.unrealized_pnl());

        if reconcile {
            let api_shares = pos.api_shares.unwrap_or(0.0);
            let unexplained = pos.unexplained_shares();

            // Status indicator
            let status = if unexplained.abs() < 0.01 {
                "✓"
            } else if unexplained > 0.0 {
                "?"  // More shares in API than we can explain
            } else {
                "!"  // Fewer shares in API (sold elsewhere?)
            };

            // Format merge/redeem columns
            let merge_str = if pos.merged_shares > 0.01 {
                format!("+{:.1}", pos.merged_shares)
            } else {
                "-".to_string()
            };
            let redeem_str = if pos.redeemed_shares > 0.01 {
                format!("-{:.1}", pos.redeemed_shares)
            } else {
                "-".to_string()
            };
            let unexplained_str = if unexplained.abs() > 0.01 {
                format!("{:+.2}", unexplained)
            } else {
                "-".to_string()
            };

            println!(
                "{:<30} {:>6} {:>9.2} {:>7.2} {:>7} {:>7} {:>9} {:>9}",
                truncate(&pos.title, 30),
                truncate(&pos.outcome, 6),
                api_shares,
                pos.net_shares,
                merge_str,
                redeem_str,
                unexplained_str,
                status,
            );
        } else {
            println!(
                "{:<35} {:>8} {:>10.2} {:>10.4} {:>10.2} {:>10} {:>10}",
                truncate(&pos.title, 35),
                truncate(&pos.outcome, 8),
                pos.net_shares,
                pos.avg_buy_price,
                pos.current_value(),
                realized_str,
                unrealized_str,
            );
        }
    }

    println!("{}", "-".repeat(105));

    // Summary
    println!("\n=== SUMMARY ===\n");
    println!("  Total Trades:        {}", summary.total_trades);
    println!("  Active Positions:    {}", positions.iter().filter(|p| p.net_shares.abs() > 0.01).count());
    println!("  Buy Volume:          ${:.2}", summary.total_buy_volume);
    println!("  Sell Volume:         ${:.2}", summary.total_sell_volume);
    println!();
    println!("  Realized PnL:        {}", format_pnl(summary.total_realized_pnl));
    println!("  Unrealized PnL:      {}", format_pnl(summary.total_unrealized_pnl));
    println!("  Total PnL:           {}", format_pnl(summary.total_realized_pnl + summary.total_unrealized_pnl));
    println!();
    println!("  Current Value:       ${:.2}", summary.total_current_value);

    if reconcile {
        // Calculate activity totals
        let total_merged: f64 = positions.iter().map(|p| p.merged_shares).sum();
        let total_redeemed: f64 = positions.iter().map(|p| p.redeemed_shares).sum();
        let still_unexplained: Vec<_> = positions.iter().filter(|p| p.unexplained_shares().abs() > 0.01).collect();

        println!();
        println!("=== RECONCILIATION ===\n");
        println!("  Shares from Trades:  {:.2}", positions.iter().map(|p| p.net_shares).sum::<f64>());
        println!("  Shares from Merges:  +{:.2}", total_merged);
        println!("  Shares Redeemed:     -{:.2}", total_redeemed);
        println!();

        if still_unexplained.is_empty() {
            println!("  ✓ All positions fully reconciled!");
        } else {
            let total_unexplained: f64 = still_unexplained.iter().map(|p| p.unexplained_shares()).sum();
            println!("  ⚠ {} positions have {:.2} unexplained shares:",
                still_unexplained.len(),
                total_unexplained);
            for pos in still_unexplained.iter().take(5) {
                println!("    - {}: {:+.2}", truncate(&pos.title, 40), pos.unexplained_shares());
            }
            if still_unexplained.len() > 5 {
                println!("    ... and {} more", still_unexplained.len() - 5);
            }
        }
    }
}

fn print_positions_csv(positions: &[PositionFromTrades], reconcile: bool) {
    if reconcile {
        println!("market,outcome,condition_id,shares,api_shares,diff,avg_price,current_price,realized_pnl,unrealized_pnl");
    } else {
        println!("market,outcome,condition_id,shares,avg_price,current_price,value,realized_pnl,unrealized_pnl,total_pnl");
    }

    for pos in positions {
        if reconcile {
            println!(
                "{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2},{:.2}",
                escape_csv(&pos.title),
                pos.outcome,
                pos.condition_id,
                pos.net_shares,
                pos.api_shares.unwrap_or(0.0),
                pos.unexplained_shares(),
                pos.avg_buy_price,
                pos.current_price.unwrap_or(0.0),
                pos.realized_pnl,
                pos.unrealized_pnl(),
            );
        } else {
            println!(
                "{},{},{},{:.4},{:.4},{:.4},{:.2},{:.2},{:.2},{:.2}",
                escape_csv(&pos.title),
                pos.outcome,
                pos.condition_id,
                pos.net_shares,
                pos.avg_buy_price,
                pos.current_price.unwrap_or(0.0),
                pos.current_value(),
                pos.realized_pnl,
                pos.unrealized_pnl(),
                pos.total_pnl(),
            );
        }
    }
}

fn print_positions_json(positions: &[PositionFromTrades], summary: &TradeSummary, _reconcile: bool) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Output {
        summary: SummaryJson,
        positions: Vec<PositionJson>,
    }

    #[derive(serde::Serialize)]
    struct SummaryJson {
        total_trades: usize,
        total_positions: usize,
        buy_volume: f64,
        sell_volume: f64,
        realized_pnl: f64,
        unrealized_pnl: f64,
        total_pnl: f64,
        current_value: f64,
        positions_with_unexplained: usize,
    }

    #[derive(serde::Serialize)]
    struct PositionJson {
        title: String,
        outcome: String,
        condition_id: String,
        shares: f64,
        api_shares: Option<f64>,
        unexplained_shares: f64,
        avg_buy_price: f64,
        current_price: Option<f64>,
        current_value: f64,
        realized_pnl: f64,
        unrealized_pnl: f64,
        total_pnl: f64,
        buy_count: usize,
        sell_count: usize,
    }

    let output = Output {
        summary: SummaryJson {
            total_trades: summary.total_trades,
            total_positions: summary.total_positions,
            buy_volume: summary.total_buy_volume,
            sell_volume: summary.total_sell_volume,
            realized_pnl: summary.total_realized_pnl,
            unrealized_pnl: summary.total_unrealized_pnl,
            total_pnl: summary.total_realized_pnl + summary.total_unrealized_pnl,
            current_value: summary.total_current_value,
            positions_with_unexplained: summary.positions_with_unexplained,
        },
        positions: positions
            .iter()
            .map(|p| PositionJson {
                title: p.title.clone(),
                outcome: p.outcome.clone(),
                condition_id: p.condition_id.clone(),
                shares: p.net_shares,
                api_shares: p.api_shares,
                unexplained_shares: p.unexplained_shares(),
                avg_buy_price: p.avg_buy_price,
                current_price: p.current_price,
                current_value: p.current_value(),
                realized_pnl: p.realized_pnl,
                unrealized_pnl: p.unrealized_pnl(),
                total_pnl: p.total_pnl(),
                buy_count: p.buy_count,
                sell_count: p.sell_count,
            })
            .collect(),
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn format_pnl(pnl: f64) -> String {
    if pnl >= 0.0 {
        format!("+${:.2}", pnl)
    } else {
        format!("-${:.2}", pnl.abs())
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    } else {
        s.to_string()
    }
}

fn escape_csv(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ============================================================================
// Activity output functions
// ============================================================================

fn print_activities_table(activities: &[&Activity], summary: &ActivitySummary) {
    println!("=== ACTIVITY HISTORY ===\n");

    println!(
        "{:<19} {:<8} {:<35} {:>8} {:>10} {:>10}",
        "Time", "Type", "Market", "Outcome", "Shares", "Value"
    );
    println!("{}", "-".repeat(100));

    for activity in activities.iter().rev().take(50) {
        let time = chrono::DateTime::from_timestamp(activity.timestamp, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| activity.timestamp.to_string());

        let type_str = match activity.activity_type {
            ActivityType::Trade => "TRADE",
            ActivityType::Merge => "MERGE",
            ActivityType::Redeem => "REDEEM",
        };

        let value = activity.value();

        println!(
            "{:<19} {:<8} {:<35} {:>8} {:>10.2} {:>10.2}",
            time,
            type_str,
            truncate(&activity.title, 35),
            truncate(&activity.outcome, 8),
            activity.size,
            value,
        );
    }

    if activities.len() > 50 {
        println!("\n... and {} more activities (use --limit to show more)", activities.len() - 50);
    }

    println!("{}", "-".repeat(100));

    // Summary
    println!("\n=== SUMMARY ===\n");
    println!("  Total Activities:    {}", summary.total_activities);
    println!("  Trades:              {}", summary.trade_count);
    println!("  Merges:              {}", summary.merge_count);
    println!("  Redemptions:         {}", summary.redeem_count);
    println!();
    println!("  Total Merged:        ${:.2}", summary.total_merged_usdc);
    println!("  Total Redeemed:      ${:.2}", summary.total_redeemed_usdc);
}

fn print_activities_csv(activities: &[&Activity]) {
    println!("timestamp,type,market,outcome,condition_id,shares,price,value,fee,tx_hash");
    for activity in activities {
        println!(
            "{},{},{},{},{},{:.4},{:.4},{:.2},{:.4},{}",
            activity.timestamp,
            activity.activity_type,
            escape_csv(&activity.title),
            activity.outcome,
            activity.condition_id,
            activity.size,
            activity.price,
            activity.value(),
            activity.fee,
            activity.transaction_hash,
        );
    }
}

fn print_activities_json(activities: &[&Activity], summary: &ActivitySummary) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Output {
        summary: SummaryJson,
        activities: Vec<ActivityJson>,
    }

    #[derive(serde::Serialize)]
    struct SummaryJson {
        total_activities: usize,
        trade_count: usize,
        merge_count: usize,
        redeem_count: usize,
        total_merged_usdc: f64,
        total_redeemed_usdc: f64,
    }

    #[derive(serde::Serialize)]
    struct ActivityJson {
        timestamp: i64,
        #[serde(rename = "type")]
        activity_type: String,
        title: String,
        outcome: String,
        condition_id: String,
        shares: f64,
        price: f64,
        value: f64,
        fee: f64,
        usdc_size: f64,
        transaction_hash: String,
    }

    let output = Output {
        summary: SummaryJson {
            total_activities: summary.total_activities,
            trade_count: summary.trade_count,
            merge_count: summary.merge_count,
            redeem_count: summary.redeem_count,
            total_merged_usdc: summary.total_merged_usdc,
            total_redeemed_usdc: summary.total_redeemed_usdc,
        },
        activities: activities
            .iter()
            .map(|a| ActivityJson {
                timestamp: a.timestamp,
                activity_type: a.activity_type.to_string(),
                title: a.title.clone(),
                outcome: a.outcome.clone(),
                condition_id: a.condition_id.clone(),
                shares: a.size,
                price: a.price,
                value: a.value(),
                fee: a.fee,
                usdc_size: a.usdc_size,
                transaction_hash: a.transaction_hash.clone(),
            })
            .collect(),
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
