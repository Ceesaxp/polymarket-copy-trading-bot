// store.rs - SQLite persistence implementation
//
// Provides TradeStore for managing trade persistence with:
// - WAL mode for concurrent reads
// - NORMAL synchronous mode for performance
// - Schema initialization and validation

use anyhow::{Result, Context};
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

/// Aggregated position for a token
#[derive(Debug, Clone)]
pub struct Position {
    pub token_id: String,
    pub net_shares: f64,
    pub avg_entry_price: Option<f64>,
    pub trade_count: i32,
}

/// Aggregation efficiency statistics
#[derive(Debug, Clone)]
pub struct AggregationStats {
    /// Total number of orders executed
    pub total_orders: u32,
    /// Number of orders that were aggregated (aggregation_count > 1)
    pub aggregated_orders: u32,
    /// Total number of individual trades combined through aggregation
    pub total_trades_combined: u32,
    /// Average number of trades per aggregated order
    pub avg_trades_per_aggregation: f64,
}

/// TradeRecord represents a single trade execution record
///
/// This struct matches the trades table schema and includes:
/// - Whale's trade details (what we detected)
/// - Our execution details (what we achieved)
/// - Status and timing information
/// - Aggregation analytics (if trade was aggregated)
///
/// Fields with Option<T> are nullable in the database (e.g., our_* fields for failed trades)
#[derive(Debug, Clone)]
pub struct TradeRecord {
    /// Unix timestamp in milliseconds
    pub timestamp_ms: i64,
    /// Ethereum block number
    pub block_number: u64,
    /// Transaction hash (whale's trade)
    pub tx_hash: String,
    /// Address of whale being copied
    pub trader_address: String,
    /// Polymarket token ID
    pub token_id: String,
    /// Order side: "BUY" or "SELL"
    pub side: String,
    /// Whale's trade size in shares
    pub whale_shares: f64,
    /// Whale's execution price
    pub whale_price: f64,
    /// Whale's USD value
    pub whale_usd: f64,
    /// Our executed size (None if failed)
    pub our_shares: Option<f64>,
    /// Our execution price (None if failed)
    pub our_price: Option<f64>,
    /// Our USD value (None if failed)
    pub our_usd: Option<f64>,
    /// Fill percentage 0-100 (None if failed)
    pub fill_pct: Option<f64>,
    /// Trade status: SUCCESS, PARTIAL, FAILED, SKIPPED
    pub status: String,
    /// Latency from detection to order placement in milliseconds
    pub latency_ms: Option<i64>,
    /// Live trading vs dry run
    pub is_live: Option<bool>,
    /// Number of trades combined in aggregation (None or 1 = not aggregated)
    pub aggregation_count: Option<u32>,
    /// Duration of aggregation window in milliseconds (None = not aggregated)
    pub aggregation_window_ms: Option<u64>,
}

/// TradeStore manages SQLite database connection for trade persistence
///
/// Features:
/// - Automatic schema initialization
/// - WAL mode for concurrent reads during writes
/// - NORMAL synchronous mode for <100ms writes
/// - Buffered writes for sub-millisecond hot path performance
pub struct TradeStore {
    pub(crate) conn: Connection,
    write_buffer: Mutex<Vec<TradeRecord>>,
    buffer_size: usize,
}

impl TradeStore {
    /// Create a new TradeStore with default buffer size (50), initializing schema if needed
    ///
    /// # Arguments
    /// * `db_path` - Path to SQLite database file
    ///
    /// # Returns
    /// * `Result<TradeStore>` - Initialized store or error
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        Self::with_buffer_size(db_path, 50)
    }

    /// Create a new TradeStore with configurable buffer size
    ///
    /// # Arguments
    /// * `db_path` - Path to SQLite database file
    /// * `buffer_size` - Number of trades to buffer before auto-flushing
    ///
    /// # Returns
    /// * `Result<TradeStore>` - Initialized store or error
    pub fn with_buffer_size<P: AsRef<Path>>(db_path: P, buffer_size: usize) -> Result<Self> {
        let conn = Connection::open(db_path.as_ref())
            .context("Failed to open SQLite database")?;

        // Configure database settings for performance
        // WAL mode: enables concurrent reads during writes
        // NORMAL synchronous: balance between safety and performance
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;"
        ).context("Failed to configure database settings")?;

        // Initialize schema
        let schema_sql = include_str!("schema.sql");
        conn.execute_batch(schema_sql)
            .context("Failed to initialize schema")?;

        Ok(TradeStore {
            conn,
            write_buffer: Mutex::new(Vec::with_capacity(buffer_size)),
            buffer_size,
        })
    }

    /// Get current journal mode (for testing)
    pub fn get_journal_mode(&self) -> Result<String> {
        let mode: String = self.conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .context("Failed to query journal_mode")?;
        Ok(mode)
    }

    /// Get current synchronous mode (for testing)
    pub fn get_synchronous_mode(&self) -> Result<String> {
        let mode: i32 = self.conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .context("Failed to query synchronous mode")?;
        Ok(mode.to_string())
    }

    /// Check if a table exists (for testing)
    pub fn table_exists(&self, table_name: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            params![table_name],
            |row| row.get(0),
        ).context("Failed to check table existence")?;
        Ok(count > 0)
    }

    /// Get column names for a table (for testing)
    pub fn get_table_columns(&self, table_name: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn
            .prepare(&format!("PRAGMA table_info({})", table_name))
            .context("Failed to prepare table_info query")?;

        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .context("Failed to query columns")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect columns")?;

        Ok(columns)
    }

    /// Get table info including NOT NULL constraints (for testing)
    /// Returns: Vec<(column_name, is_not_null)>
    pub fn get_table_info(&self, table_name: &str) -> Result<Vec<(String, bool)>> {
        let mut stmt = self.conn
            .prepare(&format!("PRAGMA table_info({})", table_name))
            .context("Failed to prepare table_info query")?;

        let info = stmt
            .query_map([], |row| {
                let name: String = row.get(1)?;
                let not_null: i32 = row.get(3)?;
                Ok((name, not_null == 1))
            })
            .context("Failed to query table info")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect table info")?;

        Ok(info)
    }

    /// Get count of trades in database
    pub fn get_trade_count(&self) -> Result<i64> {
        let count: i64 = self.conn
            .query_row("SELECT COUNT(*) FROM trades", [], |row| row.get(0))
            .context("Failed to query trade count")?;
        Ok(count)
    }

    /// Insert a single trade record into the database
    ///
    /// This is a synchronous write operation that blocks until the trade is persisted.
    /// For non-blocking writes, use `record_trade` which uses buffered writes.
    ///
    /// # Arguments
    /// * `record` - The trade record to insert
    ///
    /// # Returns
    /// * `Result<()>` - Ok if inserted successfully, Err otherwise
    pub fn insert_trade(&self, record: &TradeRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO trades (
                timestamp_ms, block_number, tx_hash, trader_address, token_id,
                side, whale_shares, whale_price, whale_usd,
                our_shares, our_price, our_usd, fill_pct,
                status, latency_ms, is_live, aggregation_count, aggregation_window_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                record.timestamp_ms,
                record.block_number as i64,
                &record.tx_hash,
                &record.trader_address,
                &record.token_id,
                &record.side,
                record.whale_shares,
                record.whale_price,
                record.whale_usd,
                record.our_shares,
                record.our_price,
                record.our_usd,
                record.fill_pct,
                &record.status,
                record.latency_ms,
                record.is_live,
                record.aggregation_count.map(|c| c as i64),
                record.aggregation_window_ms.map(|w| w as i64),
            ],
        ).context("Failed to insert trade record")?;
        Ok(())
    }

    /// Record a trade using buffered writes for sub-millisecond performance
    ///
    /// This method adds the trade to an in-memory buffer and returns immediately.
    /// The buffer is automatically flushed when it reaches `buffer_size` capacity.
    /// Call `flush()` to manually persist buffered trades.
    ///
    /// # Arguments
    /// * `record` - The trade record to buffer
    pub fn record_trade(&self, record: TradeRecord) {
        let mut buffer = self.write_buffer.lock().unwrap();
        buffer.push(record);

        // Auto-flush when buffer reaches capacity
        if buffer.len() >= self.buffer_size {
            drop(buffer); // Release lock before flushing
            let _ = self.flush(); // Ignore errors in auto-flush
        }
    }

    /// Flush all buffered trades to the database
    ///
    /// Returns the number of trades that were persisted.
    /// This is a synchronous operation that blocks until all trades are written.
    ///
    /// # Returns
    /// * `Result<usize>` - Number of trades flushed, or error
    pub fn flush(&self) -> Result<usize> {
        let mut buffer = self.write_buffer.lock().unwrap();

        if buffer.is_empty() {
            return Ok(0);
        }

        // Take ownership of buffered trades and clear the buffer
        let trades = buffer.drain(..).collect::<Vec<_>>();
        let count = trades.len();
        drop(buffer); // Release lock during I/O

        // Insert all trades
        for trade in trades {
            self.insert_trade(&trade)?;
        }

        Ok(count)
    }

    /// Get recent trades ordered by timestamp descending
    ///
    /// # Arguments
    /// * `limit` - Maximum number of trades to return
    ///
    /// # Returns
    /// * `Result<Vec<TradeRecord>>` - Recent trades, most recent first
    pub fn get_recent_trades(&self, limit: usize) -> Result<Vec<TradeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp_ms, block_number, tx_hash, trader_address, token_id,
                    side, whale_shares, whale_price, whale_usd,
                    our_shares, our_price, our_usd, fill_pct,
                    status, latency_ms, is_live, aggregation_count, aggregation_window_ms
             FROM trades
             ORDER BY timestamp_ms DESC
             LIMIT ?1"
        ).context("Failed to prepare get_recent_trades query")?;

        let trades = stmt.query_map(params![limit as i64], |row| {
            Ok(TradeRecord {
                timestamp_ms: row.get(0)?,
                block_number: row.get::<_, i64>(1)? as u64,
                tx_hash: row.get(2)?,
                trader_address: row.get(3)?,
                token_id: row.get(4)?,
                side: row.get(5)?,
                whale_shares: row.get(6)?,
                whale_price: row.get(7)?,
                whale_usd: row.get(8)?,
                our_shares: row.get(9)?,
                our_price: row.get(10)?,
                our_usd: row.get(11)?,
                fill_pct: row.get(12)?,
                status: row.get(13)?,
                latency_ms: row.get(14)?,
                is_live: row.get(15)?,
                aggregation_count: row.get::<_, Option<i64>>(16)?.map(|c| c as u32),
                aggregation_window_ms: row.get::<_, Option<i64>>(17)?.map(|w| w as u64),
            })
        })
        .context("Failed to execute get_recent_trades query")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to collect trade records")?;

        Ok(trades)
    }

    /// Get current positions aggregated from trades
    /// Only returns positions with non-zero net shares
    ///
    /// # Returns
    /// * `Result<Vec<Position>>` - Current positions with non-zero holdings
    pub fn get_positions(&self) -> Result<Vec<Position>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                token_id,
                SUM(CASE WHEN side = 'BUY' THEN our_shares ELSE -our_shares END) as net_shares,
                SUM(CASE WHEN side = 'BUY' THEN our_usd ELSE 0 END) /
                    NULLIF(SUM(CASE WHEN side = 'BUY' THEN our_shares ELSE 0 END), 0) as avg_entry_price,
                COUNT(*) as trade_count
             FROM trades
             WHERE our_shares IS NOT NULL
             GROUP BY token_id
             HAVING ABS(net_shares) > 0.0001"
        ).context("Failed to prepare get_positions query")?;

        let positions = stmt.query_map([], |row| {
            Ok(Position {
                token_id: row.get(0)?,
                net_shares: row.get(1)?,
                avg_entry_price: row.get(2)?,
                trade_count: row.get(3)?,
            })
        })
        .context("Failed to execute get_positions query")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to collect positions")?;

        Ok(positions)
    }

    /// Update or insert trader statistics
    ///
    /// # Arguments
    /// * `address` - Trader address
    /// * `label` - Trader label
    /// * `total_trades` - Total number of trades
    /// * `successful_trades` - Number of successful trades
    /// * `failed_trades` - Number of failed trades
    /// * `total_copied_usd` - Total USD copied
    /// * `last_trade_ts` - Last trade timestamp (milliseconds)
    /// * `daily_reset_ts` - Daily reset timestamp (milliseconds)
    ///
    /// # Returns
    /// * `Result<()>` - Ok if updated successfully
    pub fn upsert_trader_stats(
        &self,
        address: &str,
        label: &str,
        total_trades: u32,
        successful_trades: u32,
        failed_trades: u32,
        total_copied_usd: f64,
        last_trade_ts: Option<i64>,
        daily_reset_ts: i64,
    ) -> Result<()> {
        let now_ms = chrono::Utc::now().timestamp_millis();

        self.conn.execute(
            "INSERT INTO trader_stats (
                trader_address, label, total_trades, successful_trades, failed_trades,
                total_copied_usd, last_trade_ts, daily_reset_ts, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(trader_address) DO UPDATE SET
                label = excluded.label,
                total_trades = excluded.total_trades,
                successful_trades = excluded.successful_trades,
                failed_trades = excluded.failed_trades,
                total_copied_usd = excluded.total_copied_usd,
                last_trade_ts = excluded.last_trade_ts,
                daily_reset_ts = excluded.daily_reset_ts,
                updated_at = excluded.updated_at",
            params![
                address,
                label,
                total_trades,
                successful_trades,
                failed_trades,
                total_copied_usd,
                last_trade_ts,
                daily_reset_ts,
                now_ms,
                now_ms,
            ],
        ).context("Failed to upsert trader stats")?;

        Ok(())
    }

    /// Get all trader statistics
    ///
    /// # Returns
    /// * `Result<Vec<(address, label, total_trades, successful_trades, failed_trades, total_copied_usd, last_trade_ts, daily_reset_ts)>>`
    pub fn get_all_trader_stats(&self) -> Result<Vec<(String, String, u32, u32, u32, f64, Option<i64>, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT trader_address, label, total_trades, successful_trades, failed_trades,
                    total_copied_usd, last_trade_ts, daily_reset_ts
             FROM trader_stats
             ORDER BY trader_address"
        ).context("Failed to prepare get_all_trader_stats query")?;

        let stats = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as u32,
                row.get::<_, i64>(3)? as u32,
                row.get::<_, i64>(4)? as u32,
                row.get::<_, f64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .context("Failed to execute get_all_trader_stats query")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to collect trader stats")?;

        Ok(stats)
    }

    /// Get trade metrics for a specific trader
    /// Returns: (total_observed_trades, avg_fill_pct)
    ///
    /// # Arguments
    /// * `trader_address` - Trader address to query
    ///
    /// # Returns
    /// * `Result<(u32, f64)>` - (observed count, average fill percentage)
    pub fn get_trader_trade_metrics(&self, trader_address: &str) -> Result<(u32, f64)> {
        // Count all trades for this trader (observed trades)
        let total_trades: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM trades WHERE trader_address = ?1",
            params![trader_address],
            |row| row.get(0),
        ).context("Failed to count trader trades")?;

        // Calculate average fill percentage for successful trades
        let avg_fill: Option<f64> = self.conn.query_row(
            "SELECT AVG(fill_pct) FROM trades
             WHERE trader_address = ?1 AND fill_pct IS NOT NULL",
            params![trader_address],
            |row| row.get(0),
        ).context("Failed to calculate average fill percentage")?;

        Ok((total_trades as u32, avg_fill.unwrap_or(0.0)))
    }

    /// Get trade metrics for a specific trader since a timestamp
    /// Returns: (total_observed_trades, avg_fill_pct)
    ///
    /// # Arguments
    /// * `trader_address` - Trader address to query
    /// * `since_ts` - Unix timestamp in seconds
    ///
    /// # Returns
    /// * `Result<(u32, f64)>` - (observed count since timestamp, average fill percentage)
    pub fn get_trader_trade_metrics_since(&self, trader_address: &str, since_ts: i64) -> Result<(u32, f64)> {
        let since_ms = since_ts * 1000; // Convert to milliseconds

        // Count trades for this trader since timestamp
        let total_trades: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM trades WHERE trader_address = ?1 AND timestamp_ms >= ?2",
            params![trader_address, since_ms],
            |row| row.get(0),
        ).context("Failed to count trader trades since timestamp")?;

        // Calculate average fill percentage for successful trades since timestamp
        let avg_fill: Option<f64> = self.conn.query_row(
            "SELECT AVG(fill_pct) FROM trades
             WHERE trader_address = ?1 AND timestamp_ms >= ?2 AND fill_pct IS NOT NULL",
            params![trader_address, since_ms],
            |row| row.get(0),
        ).context("Failed to calculate average fill percentage since timestamp")?;

        Ok((total_trades as u32, avg_fill.unwrap_or(0.0)))
    }

    /// Get aggregation efficiency statistics
    ///
    /// Calculates statistics about trade aggregation:
    /// - Total number of orders executed
    /// - Number of orders that were aggregated
    /// - Total trades combined through aggregation
    /// - Average trades per aggregated order
    ///
    /// # Returns
    /// * `Result<AggregationStats>` - Aggregation efficiency statistics
    pub fn get_aggregation_stats(&self) -> Result<AggregationStats> {
        // Get total order count
        let total_orders: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM trades",
            [],
            |row| row.get(0),
        ).context("Failed to count total orders")?;

        // Get count of aggregated orders (where aggregation_count > 1)
        let aggregated_orders: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM trades WHERE aggregation_count IS NOT NULL AND aggregation_count > 1",
            [],
            |row| row.get(0),
        ).context("Failed to count aggregated orders")?;

        // Get sum of all aggregation_count values (total trades combined)
        let total_trades_combined: Option<i64> = self.conn.query_row(
            "SELECT SUM(aggregation_count) FROM trades WHERE aggregation_count IS NOT NULL AND aggregation_count > 1",
            [],
            |row| row.get(0),
        ).context("Failed to sum aggregation counts")?;

        // Calculate average
        let avg_trades_per_aggregation = if aggregated_orders > 0 {
            total_trades_combined.unwrap_or(0) as f64 / aggregated_orders as f64
        } else {
            0.0
        };

        Ok(AggregationStats {
            total_orders: total_orders as u32,
            aggregated_orders: aggregated_orders as u32,
            total_trades_combined: total_trades_combined.unwrap_or(0) as u32,
            avg_trades_per_aggregation,
        })
    }
}
