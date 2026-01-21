// Example: Create test database with aggregated trades
//
// Usage: cargo run --example test_aggregation_stats

use pm_whale_follower::persistence::{TradeStore, TradeRecord};

fn main() -> anyhow::Result<()> {
    let db_path = "/tmp/test_aggregation.db";

    // Remove old database if exists
    let _ = std::fs::remove_file(db_path);

    let store = TradeStore::new(db_path)?;

    // Insert 5 normal (non-aggregated) trades
    for i in 1..=5 {
        let trade = TradeRecord {
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            block_number: 12345678 + i,
            tx_hash: format!("0x{:x}", rand::random::<u64>()),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: format!("token_{}", i),
            side: "BUY".to_string(),
            whale_shares: 100.0,
            whale_price: 0.5,
            whale_usd: 50.0,
            our_shares: Some(10.0),
            our_price: Some(0.5),
            our_usd: Some(5.0),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(50),
            is_live: Some(true),
            aggregation_count: None, // Not aggregated
            aggregation_window_ms: None,
        };
        store.insert_trade(&trade)?;
    }

    // Insert 3 aggregated trades (each combining multiple trades)
    for i in 6..=8 {
        let trade = TradeRecord {
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            block_number: 12345678 + i,
            tx_hash: format!("0x{:x}", rand::random::<u64>()),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: format!("token_{}", i),
            side: "BUY".to_string(),
            whale_shares: 300.0,
            whale_price: 0.5,
            whale_usd: 150.0,
            our_shares: Some(30.0),
            our_price: Some(0.5),
            our_usd: Some(15.0),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(50),
            is_live: Some(true),
            aggregation_count: Some(3), // This order aggregated 3 trades
            aggregation_window_ms: Some(500),
        };
        store.insert_trade(&trade)?;
    }

    println!("Test database created at: {}", db_path);
    println!("\nExpected stats:");
    println!("- Total orders: 8");
    println!("- Aggregated orders: 3");
    println!("- Total trades combined: 9 (3 orders × 3 trades each)");
    println!("- Avg trades per aggregation: 3.0");
    println!("- Estimated fees saved: $0.12 (6 trades saved × $0.02)");
    println!("\nRun: cargo run --bin position_monitor -- --db {} --stats", db_path);

    Ok(())
}
