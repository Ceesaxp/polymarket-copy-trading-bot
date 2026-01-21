// Persistence module for SQLite trade storage
//
// This module provides non-blocking trade persistence with <1ms latency on the hot path.
// Uses WAL mode for concurrent reads during writes and buffered writes for performance.

mod store;

pub use store::{TradeStore, TradeRecord, Position, AggregationStats};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::fs;
    use rusqlite::params;

    /// Helper to create a temporary database path for testing
    fn temp_db_path() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let mut path = std::env::temp_dir();
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
        path.push(format!("test_trades_{}_{}.db", std::process::id(), counter));
        path
    }

    /// Helper to clean up test database
    fn cleanup_db(path: &PathBuf) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("db-shm"));
        let _ = fs::remove_file(path.with_extension("db-wal"));
    }

    #[test]
    fn test_create_store_and_initialize_schema() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        // Create a new TradeStore - this should initialize the schema
        let store = TradeStore::new(&db_path).expect("Failed to create store");

        // Verify WAL mode is enabled
        let wal_mode = store.get_journal_mode().expect("Failed to get journal mode");
        assert_eq!(wal_mode, "wal", "WAL mode should be enabled");

        // Verify synchronous mode is NORMAL for performance
        let sync_mode = store.get_synchronous_mode().expect("Failed to get synchronous mode");
        assert_eq!(sync_mode, "1", "Synchronous mode should be NORMAL (1)");

        // Verify trades table exists
        let table_exists = store.table_exists("trades").expect("Failed to check table");
        assert!(table_exists, "trades table should exist");

        // Verify trades table has correct columns
        let columns = store.get_table_columns("trades").expect("Failed to get columns");
        assert!(columns.contains(&"id".to_string()));
        assert!(columns.contains(&"timestamp_ms".to_string()));
        assert!(columns.contains(&"trader_address".to_string()));
        assert!(columns.contains(&"token_id".to_string()));
        assert!(columns.contains(&"side".to_string()));

        cleanup_db(&db_path);
    }

    #[test]
    fn test_schema_has_required_constraints() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        let store = TradeStore::new(&db_path).expect("Failed to create store");

        // Get table info to check constraints
        let table_info = store.get_table_info("trades").expect("Failed to get table info");

        // Verify side column has CHECK constraint for BUY/SELL
        // This will be validated by attempting invalid inserts in integration tests

        // Verify timestamp_ms is NOT NULL
        let timestamp_col = table_info.iter()
            .find(|col| col.0 == "timestamp_ms")
            .expect("timestamp_ms column should exist");
        assert!(timestamp_col.1, "timestamp_ms should be NOT NULL");

        cleanup_db(&db_path);
    }

    #[test]
    fn test_initial_trade_count_is_zero() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        let store = TradeStore::new(&db_path).expect("Failed to create store");

        // New database should have 0 trades
        let count = store.get_trade_count().expect("Failed to get trade count");
        assert_eq!(count, 0, "New database should have 0 trades");

        cleanup_db(&db_path);
    }

    #[test]
    fn test_trade_record_construction_with_all_fields() {
        // Test that TradeRecord can be constructed with all required fields
        let record = TradeRecord {
            timestamp_ms: 1706000000000,
            block_number: 12345678,
            tx_hash: "0xabc123def456".to_string(),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 500.0,
            whale_price: 0.45,
            whale_usd: 225.0,
            our_shares: Some(10.0),
            our_price: Some(0.46),
            our_usd: Some(4.6),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(85),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        };

        // Verify all fields are accessible and have correct values
        assert_eq!(record.timestamp_ms, 1706000000000);
        assert_eq!(record.block_number, 12345678);
        assert_eq!(record.tx_hash, "0xabc123def456");
        assert_eq!(record.trader_address, "0x1234567890abcdef1234567890abcdef12345678");
        assert_eq!(record.token_id, "123456");
        assert_eq!(record.side, "BUY");
        assert_eq!(record.whale_shares, 500.0);
        assert_eq!(record.whale_price, 0.45);
        assert_eq!(record.whale_usd, 225.0);
        assert_eq!(record.our_shares, Some(10.0));
        assert_eq!(record.our_price, Some(0.46));
        assert_eq!(record.our_usd, Some(4.6));
        assert_eq!(record.fill_pct, Some(100.0));
        assert_eq!(record.status, "SUCCESS");
        assert_eq!(record.latency_ms, Some(85));
        assert_eq!(record.is_live, Some(true));
    }

    #[test]
    fn test_trade_record_construction_with_failed_trade() {
        // Test TradeRecord for a failed trade (our_* fields are None)
        let record = TradeRecord {
            timestamp_ms: 1706000000000,
            block_number: 12345678,
            tx_hash: "0xabc123def456".to_string(),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: "123456".to_string(),
            side: "SELL".to_string(),
            whale_shares: 1000.0,
            whale_price: 0.55,
            whale_usd: 550.0,
            our_shares: None,
            our_price: None,
            our_usd: None,
            fill_pct: None,
            status: "FAILED".to_string(),
            latency_ms: Some(120),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        };

        // Verify failed trade characteristics
        assert_eq!(record.status, "FAILED");
        assert_eq!(record.our_shares, None);
        assert_eq!(record.our_price, None);
        assert_eq!(record.our_usd, None);
        assert_eq!(record.fill_pct, None);
    }

    #[test]
    fn test_trade_record_clone() {
        // Test that TradeRecord implements Clone
        let original = TradeRecord {
            timestamp_ms: 1706000000000,
            block_number: 12345678,
            tx_hash: "0xabc123def456".to_string(),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 500.0,
            whale_price: 0.45,
            whale_usd: 225.0,
            our_shares: Some(10.0),
            our_price: Some(0.46),
            our_usd: Some(4.6),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(85),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        };

        let cloned = original.clone();

        // Verify clone has same values
        assert_eq!(cloned.timestamp_ms, original.timestamp_ms);
        assert_eq!(cloned.tx_hash, original.tx_hash);
        assert_eq!(cloned.status, original.status);
    }

    #[test]
    fn test_insert_trade() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        let store = TradeStore::new(&db_path).expect("Failed to create store");

        // Create a trade record
        let record = TradeRecord {
            timestamp_ms: 1706000000000,
            block_number: 12345678,
            tx_hash: "0xabc123def456".to_string(),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 500.0,
            whale_price: 0.45,
            whale_usd: 225.0,
            our_shares: Some(10.0),
            our_price: Some(0.46),
            our_usd: Some(4.6),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(85),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        };

        // Insert the trade
        store.insert_trade(&record).expect("Failed to insert trade");

        // Verify the trade was inserted
        let count = store.get_trade_count().expect("Failed to get trade count");
        assert_eq!(count, 1, "Should have 1 trade in database");

        // Query it back using direct SQL (since we don't have get_recent_trades yet)
        let retrieved: (i64, String, String, f64, f64) = store.conn.query_row(
            "SELECT timestamp_ms, tx_hash, side, whale_shares, whale_price FROM trades WHERE tx_hash = ?1",
            params![&record.tx_hash],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).expect("Failed to query trade");

        assert_eq!(retrieved.0, record.timestamp_ms);
        assert_eq!(retrieved.1, record.tx_hash);
        assert_eq!(retrieved.2, record.side);
        assert_eq!(retrieved.3, record.whale_shares);
        assert_eq!(retrieved.4, record.whale_price);

        cleanup_db(&db_path);
    }

    // ============================================================================
    // Helper functions for buffered write tests
    // ============================================================================

    /// Helper to create a test trade with minimal required fields
    fn make_test_trade(token_id: &str, side: &str, shares: f64) -> TradeRecord {
        TradeRecord {
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            block_number: 12345678,
            tx_hash: format!("0x{:x}", rand::random::<u64>()),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: token_id.to_string(),
            side: side.to_string(),
            whale_shares: shares,
            whale_price: 0.50,
            whale_usd: shares * 0.50,
            our_shares: None,
            our_price: None,
            our_usd: None,
            fill_pct: None,
            status: "SKIPPED".to_string(),
            latency_ms: None,
            is_live: Some(false),
            aggregation_count: None,
            aggregation_window_ms: None,
        }
    }

    /// Helper to create a test trade with our execution details
    fn make_trade_with_our_shares(token_id: &str, side: &str, our_shares: f64, our_price: f64) -> TradeRecord {
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

    // ============================================================================
    // Buffered Write Tests
    // ============================================================================

    #[test]
    fn test_record_trade_buffered() {
        let store = TradeStore::with_buffer_size(":memory:", 10).unwrap();
        store.record_trade(make_test_trade("token1", "BUY", 100.0));

        // Verify trade is in buffer, not in DB yet
        let count: i64 = store.conn
            .query_row("SELECT COUNT(*) FROM trades", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "Trade should be in buffer, not DB");
    }

    #[test]
    fn test_auto_flush_at_buffer_size() {
        let store = TradeStore::with_buffer_size(":memory:", 5).unwrap();
        for i in 0..5 {
            store.record_trade(make_test_trade(&format!("token{}", i), "BUY", 100.0));
        }
        // After 5 trades with buffer_size=5, should auto-flush
        let count: i64 = store.conn
            .query_row("SELECT COUNT(*) FROM trades", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 5, "Should auto-flush when buffer is full");
    }

    #[test]
    fn test_manual_flush() {
        let store = TradeStore::with_buffer_size(":memory:", 50).unwrap();
        store.record_trade(make_test_trade("token1", "BUY", 100.0));
        store.record_trade(make_test_trade("token2", "SELL", 50.0));

        let flushed = store.flush().unwrap();
        assert_eq!(flushed, 2);

        let count: i64 = store.conn
            .query_row("SELECT COUNT(*) FROM trades", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_flush_empty_buffer() {
        let store = TradeStore::with_buffer_size(":memory:", 50).unwrap();

        let flushed = store.flush().unwrap();
        assert_eq!(flushed, 0, "Flushing empty buffer should return 0");
    }

    #[test]
    fn test_flush_idempotent() {
        let store = TradeStore::with_buffer_size(":memory:", 50).unwrap();
        store.record_trade(make_test_trade("token1", "BUY", 100.0));

        let flushed1 = store.flush().unwrap();
        assert_eq!(flushed1, 1);

        let flushed2 = store.flush().unwrap();
        assert_eq!(flushed2, 0, "Second flush should return 0");

        let count: i64 = store.conn
            .query_row("SELECT COUNT(*) FROM trades", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "Should only have 1 trade in DB");
    }

    #[test]
    fn test_flush_atomicity() {
        // This test verifies that flush is atomic - either all records are written or none
        // We'll test this by flushing multiple records and verifying count
        let store = TradeStore::with_buffer_size(":memory:", 100).unwrap();

        for i in 0..10 {
            store.record_trade(make_test_trade(&format!("token{}", i), "BUY", 100.0));
        }

        let flushed = store.flush().unwrap();
        assert_eq!(flushed, 10);

        let count: i64 = store.conn
            .query_row("SELECT COUNT(*) FROM trades", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 10, "All 10 trades should be in DB");
    }

    // ============================================================================
    // Query Methods Tests - get_recent_trades
    // ============================================================================

    #[test]
    fn test_get_recent_trades_ordering() {
        let store = TradeStore::new(":memory:").unwrap();

        // Insert trades with different timestamps
        for i in 0..10i64 {
            let mut trade = make_test_trade(&format!("token{}", i), "BUY", 100.0);
            trade.timestamp_ms = 1000 + i;
            store.insert_trade(&trade).unwrap();
        }

        let recent = store.get_recent_trades(5).unwrap();
        assert_eq!(recent.len(), 5);

        // Should be in descending order (most recent first)
        assert_eq!(recent[0].timestamp_ms, 1009); // newest
        assert_eq!(recent[4].timestamp_ms, 1005); // 5th newest

        // Verify ordering is consistent
        for i in 0..4 {
            assert!(recent[i].timestamp_ms > recent[i+1].timestamp_ms);
        }
    }

    #[test]
    fn test_get_recent_trades_limit() {
        let store = TradeStore::new(":memory:").unwrap();

        for i in 0..3 {
            store.insert_trade(&make_test_trade(&format!("token{}", i), "BUY", 100.0)).unwrap();
        }

        // Request more than available
        let trades = store.get_recent_trades(100).unwrap();
        assert_eq!(trades.len(), 3);

        // Request exact amount
        let trades = store.get_recent_trades(3).unwrap();
        assert_eq!(trades.len(), 3);
    }

    // ============================================================================
    // Position Tests - get_positions
    // ============================================================================

    #[test]
    fn test_position_struct_exists() {
        use super::Position;
        let pos = Position {
            token_id: "test".to_string(),
            net_shares: 100.0,
            avg_entry_price: Some(0.5),
            trade_count: 1,
        };
        assert_eq!(pos.token_id, "test");
    }

    #[test]
    fn test_get_positions_empty_db() {
        let store = TradeStore::new(":memory:").unwrap();
        let positions = store.get_positions().unwrap();
        assert_eq!(positions.len(), 0);
    }

    #[test]
    fn test_get_positions_basic() {
        let store = TradeStore::new(":memory:").unwrap();

        let mut trade = make_test_trade("token1", "BUY", 100.0);
        trade.our_shares = Some(100.0);
        trade.our_price = Some(0.50);
        trade.our_usd = Some(50.0);
        store.insert_trade(&trade).unwrap();

        let positions = store.get_positions().unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].token_id, "token1");
        assert!((positions[0].net_shares - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_get_positions_buy_sell_aggregation() {
        let store = TradeStore::new(":memory:").unwrap();

        // BUY 100, BUY 50, SELL 30 = net 120
        for (side, shares, price) in [("BUY", 100.0, 0.50), ("BUY", 50.0, 0.55), ("SELL", 30.0, 0.60)] {
            let mut trade = make_test_trade("token1", side, shares);
            trade.our_shares = Some(shares);
            trade.our_price = Some(price);
            trade.our_usd = Some(shares * price);
            store.insert_trade(&trade).unwrap();
        }

        let positions = store.get_positions().unwrap();
        assert_eq!(positions.len(), 1);
        assert!((positions[0].net_shares - 120.0).abs() < 0.01);
        assert_eq!(positions[0].trade_count, 3);
    }

    #[test]
    fn test_get_positions_multiple_tokens() {
        let store = TradeStore::new(":memory:").unwrap();

        // Two different tokens
        for (token, shares) in [("token1", 100.0), ("token2", 200.0)] {
            let mut trade = make_test_trade(token, "BUY", shares);
            trade.our_shares = Some(shares);
            trade.our_price = Some(0.50);
            trade.our_usd = Some(shares * 0.50);
            store.insert_trade(&trade).unwrap();
        }

        let positions = store.get_positions().unwrap();
        assert_eq!(positions.len(), 2);
    }

    #[test]
    fn test_get_positions_excludes_zero() {
        let store = TradeStore::new(":memory:").unwrap();

        // BUY 100 then SELL 100 = net 0 (should not appear)
        for (side, shares) in [("BUY", 100.0), ("SELL", 100.0)] {
            let mut trade = make_test_trade("token1", side, shares);
            trade.our_shares = Some(shares);
            trade.our_price = Some(0.50);
            trade.our_usd = Some(shares * 0.50);
            store.insert_trade(&trade).unwrap();
        }

        let positions = store.get_positions().unwrap();
        assert_eq!(positions.len(), 0, "Zero position should not appear");
    }

    #[test]
    fn test_get_positions_avg_entry_price() {
        let store = TradeStore::new(":memory:").unwrap();

        // BUY at 0.50 and 0.60 - average should be 0.55
        for price in [0.50, 0.60] {
            let mut trade = make_test_trade("token1", "BUY", 100.0);
            trade.our_shares = Some(100.0);
            trade.our_price = Some(price);
            trade.our_usd = Some(100.0 * price);
            store.insert_trade(&trade).unwrap();
        }

        let positions = store.get_positions().unwrap();
        assert_eq!(positions.len(), 1);
        let avg_price = positions[0].avg_entry_price.unwrap();
        assert!((avg_price - 0.55).abs() < 0.01, "Average entry price should be 0.55, got {}", avg_price);
    }

    // ============================================================================
    // Aggregation Analytics Tests
    // ============================================================================

    #[test]
    fn test_trade_record_with_aggregation_fields() {
        // Test that TradeRecord can be constructed with aggregation fields
        let record = TradeRecord {
            timestamp_ms: 1706000000000,
            block_number: 12345678,
            tx_hash: "0xabc123def456".to_string(),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 500.0,
            whale_price: 0.45,
            whale_usd: 225.0,
            our_shares: Some(10.0),
            our_price: Some(0.46),
            our_usd: Some(4.6),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(85),
            is_live: Some(true),
            aggregation_count: Some(3),
            aggregation_window_ms: Some(750),
        };

        // Verify aggregation fields are accessible
        assert_eq!(record.aggregation_count, Some(3));
        assert_eq!(record.aggregation_window_ms, Some(750));
    }

    #[test]
    fn test_trade_record_without_aggregation_fields() {
        // Test that TradeRecord works with None aggregation fields (non-aggregated trade)
        let record = TradeRecord {
            timestamp_ms: 1706000000000,
            block_number: 12345678,
            tx_hash: "0xabc123def456".to_string(),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 5000.0,
            whale_price: 0.45,
            whale_usd: 2250.0,
            our_shares: Some(5000.0),
            our_price: Some(0.46),
            our_usd: Some(2300.0),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(85),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        };

        // Verify non-aggregated trade has None for aggregation fields
        assert_eq!(record.aggregation_count, None);
        assert_eq!(record.aggregation_window_ms, None);
    }

    #[test]
    fn test_insert_trade_with_aggregation_fields() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        let store = TradeStore::new(&db_path).expect("Failed to create store");

        // Create a trade record with aggregation info
        let record = TradeRecord {
            timestamp_ms: 1706000000000,
            block_number: 12345678,
            tx_hash: "0xabc123agg".to_string(),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 500.0,
            whale_price: 0.45,
            whale_usd: 225.0,
            our_shares: Some(10.0),
            our_price: Some(0.46),
            our_usd: Some(4.6),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(85),
            is_live: Some(true),
            aggregation_count: Some(3),
            aggregation_window_ms: Some(750),
        };

        // Insert the trade
        store.insert_trade(&record).expect("Failed to insert trade");

        // Verify the trade was inserted with aggregation fields
        let count = store.get_trade_count().expect("Failed to get trade count");
        assert_eq!(count, 1, "Should have 1 trade in database");

        // Retrieve the trade and verify aggregation fields
        let trades = store.get_recent_trades(1).expect("Failed to get recent trades");
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].aggregation_count, Some(3));
        assert_eq!(trades[0].aggregation_window_ms, Some(750));

        cleanup_db(&db_path);
    }

    #[test]
    fn test_retrieve_trade_without_aggregation_fields() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        let store = TradeStore::new(&db_path).expect("Failed to create store");

        // Create a non-aggregated trade (large trade that bypassed aggregation)
        let record = TradeRecord {
            timestamp_ms: 1706000000000,
            block_number: 12345678,
            tx_hash: "0xabc123noagg".to_string(),
            trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 5000.0,
            whale_price: 0.45,
            whale_usd: 2250.0,
            our_shares: Some(5000.0),
            our_price: Some(0.46),
            our_usd: Some(2300.0),
            fill_pct: Some(100.0),
            status: "SUCCESS".to_string(),
            latency_ms: Some(85),
            is_live: Some(true),
            aggregation_count: None,
            aggregation_window_ms: None,
        };

        // Insert the trade
        store.insert_trade(&record).expect("Failed to insert trade");

        // Retrieve and verify aggregation fields are None
        let trades = store.get_recent_trades(1).expect("Failed to get recent trades");
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].aggregation_count, None);
        assert_eq!(trades[0].aggregation_window_ms, None);

        cleanup_db(&db_path);
    }

    #[test]
    fn test_get_aggregation_stats_all_non_aggregated() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        let store = TradeStore::new(&db_path).expect("Failed to create store");

        // Insert 5 non-aggregated trades (large trades that bypassed aggregation)
        for i in 0..5 {
            let record = TradeRecord {
                timestamp_ms: 1706000000000 + i,
                block_number: 12345678,
                tx_hash: format!("0xtx{}", i),
                trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                token_id: "123456".to_string(),
                side: "BUY".to_string(),
                whale_shares: 5000.0,
                whale_price: 0.45,
                whale_usd: 2250.0,
                our_shares: Some(5000.0),
                our_price: Some(0.46),
                our_usd: Some(2300.0),
                fill_pct: Some(100.0),
                status: "SUCCESS".to_string(),
                latency_ms: Some(85),
                is_live: Some(true),
                aggregation_count: None, // Not aggregated
                aggregation_window_ms: None,
            };
            store.insert_trade(&record).expect("Failed to insert trade");
        }

        let stats = store.get_aggregation_stats().expect("Failed to get stats");

        assert_eq!(stats.total_orders, 5);
        assert_eq!(stats.aggregated_orders, 0);
        assert_eq!(stats.total_trades_combined, 0);
        assert_eq!(stats.avg_trades_per_aggregation, 0.0);

        cleanup_db(&db_path);
    }

    #[test]
    fn test_get_aggregation_stats_with_aggregations() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        let store = TradeStore::new(&db_path).expect("Failed to create store");

        // Insert 3 aggregated trades
        let aggregated_trades = vec![
            (Some(3), Some(750)), // 3 trades aggregated in 750ms
            (Some(2), Some(500)), // 2 trades aggregated in 500ms
            (Some(4), Some(800)), // 4 trades aggregated in 800ms
        ];

        for (i, (count, window)) in aggregated_trades.iter().enumerate() {
            let record = TradeRecord {
                timestamp_ms: 1706000000000 + i as i64,
                block_number: 12345678,
                tx_hash: format!("0xagg{}", i),
                trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                token_id: "123456".to_string(),
                side: "BUY".to_string(),
                whale_shares: 500.0,
                whale_price: 0.45,
                whale_usd: 225.0,
                our_shares: Some(500.0),
                our_price: Some(0.46),
                our_usd: Some(230.0),
                fill_pct: Some(100.0),
                status: "SUCCESS".to_string(),
                latency_ms: Some(85),
                is_live: Some(true),
                aggregation_count: *count,
                aggregation_window_ms: *window,
            };
            store.insert_trade(&record).expect("Failed to insert trade");
        }

        // Insert 2 non-aggregated trades
        for i in 0..2 {
            let record = TradeRecord {
                timestamp_ms: 1706000010000 + i,
                block_number: 12345678,
                tx_hash: format!("0xnoagg{}", i),
                trader_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
                token_id: "123456".to_string(),
                side: "BUY".to_string(),
                whale_shares: 5000.0,
                whale_price: 0.45,
                whale_usd: 2250.0,
                our_shares: Some(5000.0),
                our_price: Some(0.46),
                our_usd: Some(2300.0),
                fill_pct: Some(100.0),
                status: "SUCCESS".to_string(),
                latency_ms: Some(85),
                is_live: Some(true),
                aggregation_count: None,
                aggregation_window_ms: None,
            };
            store.insert_trade(&record).expect("Failed to insert trade");
        }

        let stats = store.get_aggregation_stats().expect("Failed to get stats");

        // Total: 3 aggregated + 2 non-aggregated = 5 orders
        assert_eq!(stats.total_orders, 5);
        // Aggregated: 3 orders had aggregation
        assert_eq!(stats.aggregated_orders, 3);
        // Total trades combined: 3 + 2 + 4 = 9
        assert_eq!(stats.total_trades_combined, 9);
        // Average: 9 / 3 = 3.0 trades per aggregation
        assert!((stats.avg_trades_per_aggregation - 3.0).abs() < 0.01);

        cleanup_db(&db_path);
    }

    #[test]
    fn test_get_aggregation_stats_empty_database() {
        let db_path = temp_db_path();
        cleanup_db(&db_path);

        let store = TradeStore::new(&db_path).expect("Failed to create store");
        let stats = store.get_aggregation_stats().expect("Failed to get stats");

        assert_eq!(stats.total_orders, 0);
        assert_eq!(stats.aggregated_orders, 0);
        assert_eq!(stats.total_trades_combined, 0);
        assert_eq!(stats.avg_trades_per_aggregation, 0.0);

        cleanup_db(&db_path);
    }
}
