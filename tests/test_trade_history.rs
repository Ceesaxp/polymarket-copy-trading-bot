// Integration test for trade_history binary
//
// This test creates a test database, populates it with sample trades,
// and verifies that the trade_history binary can query it correctly.

use pm_whale_follower::persistence::{TradeStore, TradeRecord};
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_trade_history_binary_with_data() {
    // Create a temporary directory for the test database
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db_path = temp_dir.path().join("test_trades.db");

    // Create and populate test database
    let store = TradeStore::new(&db_path).expect("Failed to create store");

    // Insert sample trades
    let trades = vec![
        TradeRecord {
            timestamp_ms: 1704067200000, // 2024-01-01 00:00:00
            block_number: 12345678,
            tx_hash: "0xabc123".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token123".to_string(),
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
            aggregation_count: None,
            aggregation_window_ms: None,
        },
        TradeRecord {
            timestamp_ms: 1704067260000, // 2024-01-01 00:01:00
            block_number: 12345679,
            tx_hash: "0xdef456".to_string(),
            trader_address: "0xtrader2".to_string(),
            token_id: "token456".to_string(),
            side: "SELL".to_string(),
            whale_shares: 500.0,
            whale_price: 0.55,
            whale_usd: 275.0,
            our_shares: Some(10.0),
            our_price: Some(0.54),
            our_usd: Some(5.4),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(75),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        },
        TradeRecord {
            timestamp_ms: 1704067320000, // 2024-01-01 00:02:00
            block_number: 12345680,
            tx_hash: "0xghi789".to_string(),
            trader_address: "0xtrader1".to_string(),
            token_id: "token789".to_string(),
            side: "BUY".to_string(),
            whale_shares: 2000.0,
            whale_price: 0.60,
            whale_usd: 1200.0,
            our_shares: None,
            our_price: None,
            our_usd: None,
            fill_pct: None,
            status: "FAILED".to_string(),
            latency_ms: Some(100),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        },
    ];

    for trade in trades {
        store.insert_trade(&trade).expect("Failed to insert trade");
    }

    // Test 1: Query all trades
    let output = Command::new("./target/release/trade_history")
        .arg("--db")
        .arg(db_path.to_str().unwrap())
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success(), "Command failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("TRADE HISTORY"));
    assert!(stdout.contains("Total trades: 3"));
    assert!(stdout.contains("SUMMARY STATISTICS"));

    // Test 2: Filter by trader
    let output = Command::new("./target/release/trade_history")
        .arg("--db")
        .arg(db_path.to_str().unwrap())
        .arg("--trader")
        .arg("trader1")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Total trades: 2"));

    // Test 3: Filter by status
    let output = Command::new("./target/release/trade_history")
        .arg("--db")
        .arg(db_path.to_str().unwrap())
        .arg("--status")
        .arg("SUCCESS")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Total trades: 2"));

    // Test 4: CSV format
    let output = Command::new("./target/release/trade_history")
        .arg("--db")
        .arg(db_path.to_str().unwrap())
        .arg("--format")
        .arg("csv")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("timestamp,side,token_id"));

    // Test 5: JSON format
    let output = Command::new("./target/release/trade_history")
        .arg("--db")
        .arg(db_path.to_str().unwrap())
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to execute command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Verify it's valid JSON by parsing
    let _: serde_json::Value = serde_json::from_str(&stdout).expect("Invalid JSON output");
}
