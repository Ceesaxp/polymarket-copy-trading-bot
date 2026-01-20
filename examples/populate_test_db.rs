// Example: Populate test database with sample trades
//
// Usage: cargo run --example populate_test_db

use pm_whale_follower::persistence::{TradeStore, TradeRecord};

fn make_trade(token_id: &str, side: &str, shares: f64, price: f64) -> TradeRecord {
    TradeRecord {
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
        block_number: 12345678,
        tx_hash: format!("0x{:x}", rand::random::<u64>()),
        trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
        token_id: token_id.to_string(),
        side: side.to_string(),
        whale_shares: shares,
        whale_price: price,
        whale_usd: shares * price,
        our_shares: Some(shares),
        our_price: Some(price),
        our_usd: Some(shares * price),
        fill_pct: Some(100.0),
        status: "SUCCESS".to_string(),
        latency_ms: Some(85),
        is_live: Some(false),
    }
}

fn main() -> anyhow::Result<()> {
    let db_path = "test_trades_demo.db";

    println!("Creating test database at: {}", db_path);

    let store = TradeStore::new(db_path)?;

    // Create sample positions
    println!("Adding sample trades...");

    // Position 1: Token A - net long 150 shares
    store.insert_trade(&make_trade("token_abc123", "BUY", 100.0, 0.45))?;
    store.insert_trade(&make_trade("token_abc123", "BUY", 50.0, 0.55))?;

    // Position 2: Token B - net short 75 shares
    store.insert_trade(&make_trade("token_xyz789", "SELL", 75.0, 0.62))?;

    // Position 3: Token C - mixed trades, net long 80 shares
    store.insert_trade(&make_trade("token_def456", "BUY", 200.0, 0.50))?;
    store.insert_trade(&make_trade("token_def456", "SELL", 120.0, 0.58))?;

    // Position 4: Very long token ID (will be truncated in display)
    store.insert_trade(&make_trade("1234567890abcdefghijklmnopqrstuvwxyz", "BUY", 50.0, 0.72))?;

    println!("Database populated successfully!");
    println!("\nRun: cargo run --bin position_monitor -- --db {} --once", db_path);

    Ok(())
}
