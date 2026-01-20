-- SQLite schema for Polymarket copy trading bot persistence
-- Version: 1.0
-- Optimized for: <100ms reads, WAL mode for concurrent access

-- Main trades table: Stores all executed trade attempts
CREATE TABLE IF NOT EXISTS trades (
    -- Primary key
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- Timing and identification
    timestamp_ms INTEGER NOT NULL,           -- Unix timestamp in milliseconds
    block_number INTEGER NOT NULL,           -- Ethereum block number
    tx_hash TEXT NOT NULL,                   -- Transaction hash (whale's trade)

    -- Trade participants
    trader_address TEXT NOT NULL,            -- Address of whale being copied
    token_id TEXT NOT NULL,                  -- Polymarket token ID

    -- Trade details
    side TEXT NOT NULL CHECK (side IN ('BUY', 'SELL')),  -- Order side
    whale_shares REAL NOT NULL,              -- Whale's trade size
    whale_price REAL NOT NULL,               -- Whale's execution price
    whale_usd REAL NOT NULL,                 -- Whale's USD value

    -- Our execution details (NULL if failed)
    our_shares REAL,                         -- Our executed size
    our_price REAL,                          -- Our execution price
    our_usd REAL,                            -- Our USD value
    fill_pct REAL,                           -- Fill percentage (0-100)

    -- Status tracking
    status TEXT NOT NULL,                    -- SUCCESS, PARTIAL, FAILED, SKIPPED
    latency_ms INTEGER,                      -- Time from detection to order placement

    -- Operational flags
    is_live BOOLEAN,                         -- Live trading vs dry run
    is_aggregated BOOLEAN DEFAULT FALSE,     -- Part of aggregated trade
    aggregation_count INTEGER DEFAULT 1      -- Number of trades aggregated
);

-- Indexes for common query patterns
CREATE INDEX IF NOT EXISTS idx_trades_timestamp ON trades(timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_trades_trader ON trades(trader_address);
CREATE INDEX IF NOT EXISTS idx_trades_token ON trades(token_id);
CREATE INDEX IF NOT EXISTS idx_trades_status ON trades(status);
CREATE INDEX IF NOT EXISTS idx_trades_trader_token ON trades(trader_address, token_id);

-- Positions view: Aggregated current positions by token
-- This is a VIEW, not a table, calculated on-demand from trades
CREATE VIEW IF NOT EXISTS positions AS
SELECT
    token_id,
    trader_address,
    SUM(CASE WHEN side = 'BUY' THEN our_shares ELSE -our_shares END) as net_shares,
    SUM(CASE WHEN side = 'BUY' THEN our_usd ELSE -our_usd END) /
        NULLIF(SUM(CASE WHEN side = 'BUY' THEN our_shares ELSE 0 END), 0) as avg_buy_price,
    COUNT(*) as trade_count,
    MAX(timestamp_ms) as last_trade_ms
FROM trades
WHERE our_shares IS NOT NULL  -- Only count successful fills
GROUP BY token_id, trader_address
HAVING ABS(net_shares) > 0.01;  -- Filter out closed positions

-- Trader statistics table: Per-trader performance tracking
CREATE TABLE IF NOT EXISTS trader_stats (
    trader_address TEXT PRIMARY KEY,
    label TEXT,                              -- Human-friendly name
    total_trades INTEGER DEFAULT 0,
    successful_trades INTEGER DEFAULT 0,
    failed_trades INTEGER DEFAULT 0,
    total_copied_usd REAL DEFAULT 0,
    last_trade_ts INTEGER,                   -- Last trade timestamp
    daily_reset_ts INTEGER,                  -- Last daily reset timestamp
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
