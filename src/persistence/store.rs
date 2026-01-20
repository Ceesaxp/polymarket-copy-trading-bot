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

/// TradeRecord represents a single trade execution record
///
/// This struct matches the trades table schema and includes:
/// - Whale's trade details (what we detected)
/// - Our execution details (what we achieved)
/// - Status and timing information
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
                status, latency_ms, is_live
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
                    status, latency_ms, is_live
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
}
