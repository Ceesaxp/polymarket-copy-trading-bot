// auto_claim.rs - Automatically claim/redeem winning positions from resolved markets
//
// Usage:
//   cargo run --bin auto_claim                    # Show redeemable positions (dry run)
//   cargo run --bin auto_claim -- --execute       # Actually redeem positions
//   cargo run --bin auto_claim -- --batch         # Batch all redemptions in one TX
//   cargo run --bin auto_claim -- --min-value 5   # Only redeem positions worth >= $5

use anyhow::Result;
use clap::Parser;
use dotenvy::dotenv;
use pm_whale_follower::live_positions::{fetch_live_positions, LivePosition};
use pm_whale_follower::relayer::{BuilderCreds, RelayerClient};

#[derive(Parser)]
#[command(name = "auto_claim")]
#[command(about = "Automatically claim winning positions from resolved Polymarket markets")]
struct Args {
    /// Actually execute redemptions (default is dry run)
    #[arg(long)]
    execute: bool,

    /// Batch all redemptions into a single transaction
    #[arg(long)]
    batch: bool,

    /// Minimum value in USD to redeem (default: 0.01)
    #[arg(long, default_value = "0.01")]
    min_value: f64,

    /// Filter by market title (partial match, case-insensitive)
    #[arg(long)]
    title: Option<String>,

    /// Maximum number of positions to redeem
    #[arg(long)]
    limit: Option<usize>,

    /// Wait for transaction confirmation
    #[arg(long)]
    wait: bool,
}

fn main() -> Result<()> {
    dotenv().ok();
    let args = Args::parse();

    let wallet_address = std::env::var("FUNDER_ADDRESS")
        .map_err(|_| anyhow::anyhow!("FUNDER_ADDRESS not set in environment"))?;

    // Fetch positions
    println!("Fetching positions for {}...", wallet_address);
    let positions = fetch_live_positions(&wallet_address)?;

    // Filter for redeemable positions
    let mut redeemable: Vec<&LivePosition> = positions
        .iter()
        .filter(|p| {
            // Must be redeemable
            if !p.redeemable {
                return false;
            }
            // Must have shares
            if p.size < 0.01 {
                return false;
            }
            // Must meet minimum value threshold
            // For redeemable positions, current_value represents redemption value
            if p.current_value < args.min_value {
                return false;
            }
            // Title filter
            if let Some(ref title) = args.title {
                if !p.title.to_lowercase().contains(&title.to_lowercase()) {
                    return false;
                }
            }
            true
        })
        .collect();

    // Sort by value descending
    redeemable.sort_by(|a, b| {
        b.current_value
            .partial_cmp(&a.current_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Apply limit
    if let Some(limit) = args.limit {
        redeemable.truncate(limit);
    }

    if redeemable.is_empty() {
        println!("\nNo redeemable positions found.");
        println!("  Total positions: {}", positions.len());
        println!(
            "  Redeemable (before filters): {}",
            positions.iter().filter(|p| p.redeemable).count()
        );
        return Ok(());
    }

    // Display redeemable positions
    println!("\n=== REDEEMABLE POSITIONS ===\n");
    println!(
        "{:<40} {:>8} {:>10} {:>10}",
        "Market", "Outcome", "Shares", "Value"
    );
    println!("{}", "-".repeat(75));

    let mut total_value = 0.0;
    for pos in &redeemable {
        println!(
            "{:<40} {:>8} {:>10.2} {:>10.2}",
            truncate(&pos.title, 40),
            truncate(&pos.outcome, 8),
            pos.size,
            pos.current_value,
        );
        total_value += pos.current_value;
    }

    println!("{}", "-".repeat(75));
    println!(
        "{:<40} {:>8} {:>10} {:>10.2}",
        "TOTAL", "", "", total_value
    );
    println!("\nPositions to redeem: {}", redeemable.len());

    if !args.execute {
        println!("\n⚠️  DRY RUN - No transactions executed.");
        println!("   Use --execute to actually redeem positions.");
        return Ok(());
    }

    // Load builder credentials
    println!("\nLoading builder credentials...");
    let creds = BuilderCreds::from_env()?;
    let prepared = creds.prepare()?;

    // Create relayer client
    let client = RelayerClient::new(prepared, &wallet_address)?;

    if args.batch && redeemable.len() > 1 {
        // Batch redemption
        println!("\nExecuting batch redemption of {} positions...", redeemable.len());

        let positions_data: Vec<(String, u32)> = redeemable
            .iter()
            .map(|p| (p.condition_id.clone(), p.outcome_index as u32))
            .collect();

        match client.redeem_positions_batch(&positions_data) {
            Ok(response) => {
                println!("✓ Batch transaction submitted!");
                println!("  Transaction ID: {}", response.id);
                if let Some(hash) = &response.transaction_hash {
                    println!("  TX Hash: {}", hash);
                    println!("  Explorer: https://polygonscan.com/tx/{}", hash);
                }

                if args.wait {
                    println!("\nWaiting for confirmation...");
                    match client.wait_for_confirmation(&response.id, 30) {
                        Ok(confirmed) => {
                            println!("✓ Transaction confirmed!");
                            println!("  State: {}", confirmed.state);
                            if let Some(hash) = confirmed.transaction_hash {
                                println!("  TX Hash: {}", hash);
                            }
                        }
                        Err(e) => {
                            println!("⚠️  Confirmation timeout: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                println!("✗ Batch redemption failed: {}", e);
                return Err(e);
            }
        }
    } else {
        // Individual redemptions
        println!("\nExecuting {} individual redemptions...\n", redeemable.len());

        let mut success_count = 0;
        let mut fail_count = 0;
        let mut success_value = 0.0;

        for pos in &redeemable {
            print!(
                "Redeeming {} ({} shares, ${:.2})... ",
                truncate(&pos.title, 30),
                pos.size,
                pos.current_value
            );

            match client.redeem_position(&pos.condition_id, pos.outcome_index as u32) {
                Ok(response) => {
                    println!("✓");
                    println!("  TX ID: {}", response.id);

                    if args.wait {
                        match client.wait_for_confirmation(&response.id, 30) {
                            Ok(confirmed) => {
                                if let Some(hash) = confirmed.transaction_hash {
                                    println!("  Confirmed: https://polygonscan.com/tx/{}", hash);
                                }
                            }
                            Err(e) => {
                                println!("  ⚠️ Confirmation timeout: {}", e);
                            }
                        }
                    }

                    success_count += 1;
                    success_value += pos.current_value;
                }
                Err(e) => {
                    println!("✗ Failed: {}", e);
                    fail_count += 1;
                }
            }

            // Small delay between requests
            if !args.batch {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }

        println!("\n=== SUMMARY ===");
        println!("  Successful: {} (${:.2})", success_count, success_value);
        println!("  Failed: {}", fail_count);
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    } else {
        s.to_string()
    }
}
