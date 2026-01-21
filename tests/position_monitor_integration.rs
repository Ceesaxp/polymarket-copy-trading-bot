// Integration tests for position_monitor binary
//
// These tests verify end-to-end functionality by:
// 1. Creating a test database with known positions
// 2. Running the position_monitor binary
// 3. Verifying the output is correct

use pm_whale_follower::persistence::{TradeStore, TradeRecord};
use std::process::Command;
use std::path::PathBuf;
use std::fs;

/// Helper to create a temporary database path for testing
fn temp_db_path(test_name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("test_position_monitor_{}_{}.db", test_name, std::process::id()));
    path
}

/// Helper to clean up test database
fn cleanup_db(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("db-shm"));
    let _ = fs::remove_file(path.with_extension("db-wal"));
}

/// Helper to create a test trade with minimal required fields
fn make_test_trade(token_id: &str, side: &str, our_shares: f64, our_price: f64) -> TradeRecord {
    let our_usd = our_shares * our_price;
    TradeRecord {
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
        block_number: 12345678,
        tx_hash: format!("0x{:x}", rand::random::<u64>()),
        trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
        token_id: token_id.to_string(),
        side: side.to_string(),
        whale_shares: our_shares,
        whale_price: our_price,
        whale_usd: our_usd,
        our_shares: Some(our_shares),
        our_price: Some(our_price),
        our_usd: Some(our_usd),
        fill_pct: Some(100.0),
        status: "SUCCESS".to_string(),
        latency_ms: Some(85),
        is_live: Some(false),
        aggregation_count: None,
        aggregation_window_ms: None,
    }
}

#[test]
fn test_position_monitor_empty_database() {
    let db_path = temp_db_path("empty");
    cleanup_db(&db_path);

    // Create empty database
    let _store = TradeStore::new(&db_path).expect("Failed to create store");

    // Run position_monitor with --once flag
    let output = Command::new("cargo")
        .args(&["run", "--bin", "position_monitor", "--", "--db", db_path.to_str().unwrap(), "--once"])
        .output()
        .expect("Failed to execute position_monitor");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify output contains expected text
    assert!(stdout.contains("=== CURRENT POSITIONS ==="));
    assert!(stdout.contains("No positions found."));

    cleanup_db(&db_path);
}

#[test]
fn test_position_monitor_single_position() {
    let db_path = temp_db_path("single");
    cleanup_db(&db_path);

    // Create database with a single position
    let store = TradeStore::new(&db_path).expect("Failed to create store");
    let trade = make_test_trade("token123", "BUY", 100.0, 0.50);
    store.insert_trade(&trade).expect("Failed to insert trade");

    // Run position_monitor with --once flag
    let output = Command::new("cargo")
        .args(&["run", "--bin", "position_monitor", "--", "--db", db_path.to_str().unwrap(), "--once"])
        .output()
        .expect("Failed to execute position_monitor");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify output contains expected information
    assert!(stdout.contains("=== CURRENT POSITIONS ==="));
    assert!(stdout.contains("Token ID"));
    assert!(stdout.contains("Net Shares"));
    assert!(stdout.contains("Avg Price"));
    assert!(stdout.contains("token123"));
    assert!(stdout.contains("100.00")); // Net shares
    assert!(stdout.contains("0.5000")); // Avg price
    assert!(stdout.contains("Total positions: 1"));

    cleanup_db(&db_path);
}

#[test]
fn test_position_monitor_multiple_positions() {
    let db_path = temp_db_path("multiple");
    cleanup_db(&db_path);

    // Create database with multiple positions
    let store = TradeStore::new(&db_path).expect("Failed to create store");

    // Position 1: token1 - net BUY 150 shares
    store.insert_trade(&make_test_trade("token1", "BUY", 100.0, 0.45)).unwrap();
    store.insert_trade(&make_test_trade("token1", "BUY", 50.0, 0.55)).unwrap();

    // Position 2: token2 - net SELL 50 shares
    store.insert_trade(&make_test_trade("token2", "SELL", 50.0, 0.60)).unwrap();

    // Run position_monitor with --once flag
    let output = Command::new("cargo")
        .args(&["run", "--bin", "position_monitor", "--", "--db", db_path.to_str().unwrap(), "--once"])
        .output()
        .expect("Failed to execute position_monitor");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify output contains expected information
    assert!(stdout.contains("=== CURRENT POSITIONS ==="));
    assert!(stdout.contains("token1"));
    assert!(stdout.contains("token2"));
    assert!(stdout.contains("150.00")); // token1 net shares
    assert!(stdout.contains("-50.00")); // token2 net shares (negative for sell)
    assert!(stdout.contains("Total positions: 2"));

    cleanup_db(&db_path);
}

#[test]
fn test_position_monitor_aggregated_position() {
    let db_path = temp_db_path("aggregated");
    cleanup_db(&db_path);

    // Create database with buy and sell trades for the same token
    let store = TradeStore::new(&db_path).expect("Failed to create store");

    // BUY 200, SELL 80 = net 120
    store.insert_trade(&make_test_trade("token1", "BUY", 200.0, 0.50)).unwrap();
    store.insert_trade(&make_test_trade("token1", "SELL", 80.0, 0.55)).unwrap();

    // Run position_monitor with --once flag
    let output = Command::new("cargo")
        .args(&["run", "--bin", "position_monitor", "--", "--db", db_path.to_str().unwrap(), "--once"])
        .output()
        .expect("Failed to execute position_monitor");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify aggregation is correct
    assert!(stdout.contains("120.00")); // net shares after BUY 200 - SELL 80
    assert!(stdout.contains("Total positions: 1"));

    cleanup_db(&db_path);
}

#[test]
fn test_position_monitor_long_token_id_truncation() {
    let db_path = temp_db_path("long_token");
    cleanup_db(&db_path);

    // Create database with a very long token ID
    let store = TradeStore::new(&db_path).expect("Failed to create store");
    let long_token_id = "1234567890abcdefghijklmnopqrstuvwxyz";
    store.insert_trade(&make_test_trade(long_token_id, "BUY", 100.0, 0.50)).unwrap();

    // Run position_monitor with --once flag
    let output = Command::new("cargo")
        .args(&["run", "--bin", "position_monitor", "--", "--db", db_path.to_str().unwrap(), "--once"])
        .output()
        .expect("Failed to execute position_monitor");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify token ID is truncated with "..."
    assert!(stdout.contains("1234567890abcd..."));

    cleanup_db(&db_path);
}
