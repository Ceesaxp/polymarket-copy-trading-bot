# Rust Bot Feature Parity Implementation Plan

> Goal: Bring the Rust copy trading bot to feature parity with the Python implementation where it matters, while preserving the <100ms latency advantage.

## Overview

| Phase | Feature | Status | Priority |
|-------|---------|--------|----------|
| 1     | SQLite Persistence + Position Tracking | ✅ Complete | **High** |
| 2     | Multi-Trader Monitoring | Pending | **High** |
| 3     | Trade Aggregation | Pending | Medium |
| 4     | Research Tooling | Pending | Low |
| 5     | Live P&L Tracking | Pending | Low |

---

## Phase 1: Foundation - Persistence & Position Tracking

**Goal**: Store all trades in SQLite and provide CLI tools for position monitoring.

**End Result**:
- All trades persisted to `trades.db` with <1ms write latency (buffered)
- `cargo run --bin position_monitor` shows live positions with P&L
- CSV logging still works (backward compatible)

### Step 1.1: SQLite Schema & Store Module ✅ COMPLETE

**Files created**:
- `src/persistence/mod.rs`
- `src/persistence/schema.sql`
- `src/persistence/store.rs`

**Implementation**:
- [x] Create `src/persistence/` directory structure
- [x] Define SQLite schema with tables: `trades`, `positions` (view), `trader_stats`
- [x] Implement `TradeStore` struct with buffered writes
- [x] Configure WAL mode + NORMAL synchronous for performance
- [x] Implement `TradeRecord` struct matching schema
- [x] Implement buffered write (batch every 50 records)
- [x] Implement `record_trade()` - non-blocking, pushes to buffer
- [x] Implement `flush()` - writes buffer to DB
- [x] Implement `get_positions()` - aggregated view query
- [x] Implement `get_recent_trades(limit)` - for monitoring

**Testing**: 57 unit tests passing
- [x] Unit test: Create store, write 100 trades, verify count
- [x] Unit test: Buffer fills to 50, auto-flushes
- [x] Unit test: `get_positions()` returns correct aggregation
- [x] Unit test: WAL mode is enabled (PRAGMA check)
- [x] Integration test: Write trades during simulated event stream

**Documentation**:
- [x] Add rustdoc comments to all public functions
- [x] Document schema in `schema.sql` with comments

---

### Step 1.2: Integrate Persistence into Main Bot ✅ COMPLETE

**Files modified**:
- `src/main.rs`
- `src/settings.rs`
- `Cargo.toml`

**Implementation**:
- [x] Add `rusqlite` dependency to `Cargo.toml`
- [x] Add `DB_PATH` env var to settings (default: `trades.db`)
- [x] Add `DB_ENABLED` env var (default: true)
- [x] Initialize `TradeStore` in `main()` if enabled
- [x] Pass `TradeStore` reference to `handle_event()` (via channel pattern)
- [x] Record trade after order execution (success or failure)
- [x] Call `store.flush()` on graceful shutdown (SIGINT handler)
- [x] Ensure CSV logging still works alongside DB

**Testing**:
- [x] Integration test: Run bot in mock mode, verify DB writes
- [x] Test: Graceful shutdown flushes buffer
- [x] Test: DB_ENABLED=false disables persistence
- [x] Test: CSV still works when DB enabled

---

### Step 1.3: Position Monitor CLI Tool ✅ COMPLETE

**Files created**:
- `src/bin/position_monitor.rs`
- `tests/position_monitor_integration.rs`

**Implementation**:
- [x] Create binary that opens `trades.db` read-only
- [x] Query positions using `get_positions()`
- [x] Display formatted table with token ID, net shares, avg entry price, trade count
- [x] Auto-refresh every 10 seconds (configurable via `--interval`)
- [x] Add `--once` flag for single snapshot
- [x] Add `--db` flag for custom database path

**Testing**: 6 unit tests + 5 integration tests
- [x] Test: Displays positions correctly from test DB
- [x] Test: Handles empty DB gracefully
- [x] Test: Multiple positions display correctly
- [x] Test: Aggregated positions work correctly
- [x] Test: Long token IDs truncated properly

---

### Step 1.4: Trade History CLI Tool ✅ COMPLETE

**Files created**:
- `src/bin/trade_history.rs`
- `tests/test_trade_history.rs`

**Implementation**:
- [x] Create binary for querying trade history
- [x] Support filters: `--trader`, `--token`, `--since`, `--status`
- [x] Support output formats: table (default), CSV, JSON
- [x] Show summary statistics at end (status breakdown, volume, latency)
- [x] Paginate results (default 50, configurable via `--limit`)
- [x] Custom database path via `--db`

**Testing**: 20 unit tests + 1 integration test
- [x] Test: All filters work correctly (trader, token, status, since)
- [x] Test: Combined filters work
- [x] Test: CSV export is valid
- [x] Test: JSON export is valid
- [x] Test: Summary statistics display correctly
- [x] Test: Handles empty database gracefully

---

### Phase 1 Completion Checklist ✅ COMPLETE

- [x] All Step 1.x tasks completed (1.1, 1.2, 1.3, 1.4)
- [x] All unit tests pass: `cargo test` (89 tests total)
- [x] Persistence module: 57 tests
- [x] Position monitor: 6 unit + 5 integration tests
- [x] Trade history: 20 unit + 1 integration test

**Phase 1 Deliverable**: Bot persists all trades to SQLite with CLI tools for position monitoring and trade history export.

---

## Phase 2: Multi-Trader Monitoring

**Goal**: Monitor and copy trades from multiple whale addresses simultaneously.

**End Result**:
- Configure 2-10 traders via `TRADER_ADDRESSES` env var or `traders.json`
- Per-trader scaling ratios and thresholds
- Per-trader statistics tracking
- Single WebSocket subscription handles all traders

### Step 2.1: Trader Configuration Module ✅ COMPLETE

**Files created**:
- `src/config/mod.rs` - Module with 53 comprehensive tests
- `src/config/traders.rs` - Core implementation (363 lines)
- `traders.json.example` - Example JSON configuration

**Implementation**: ✅ ALL COMPLETE
- [x] Define `TraderConfig` struct with all required fields
- [x] Define `TradersConfig` struct with `Vec<TraderConfig>` + HashMap indexing
- [x] Implement `from_env()` - parse `TRADER_ADDRESSES=addr1,addr2`
- [x] Implement `from_file()` - load from `traders.json` with serde
- [x] Implement `load()` - smart fallback: TRADER_ADDRESSES → TARGET_WHALE_ADDRESS
- [x] Implement `build_topic_filter()` - returns Vec for WS subscription
- [x] Implement `get_by_topic()` - O(1) lookup by topic hex
- [x] Implement `get_by_address()` - O(1) lookup by address
- [x] Implement `len()`, `is_empty()`, `iter()` helper methods
- [x] Address validation: 40 hex chars, strips 0x prefix, lowercase normalization
- [x] Topic hex generation: zero-padding to 64 chars
- [x] Full backward compatibility: `TARGET_WHALE_ADDRESS` works via load()

**Testing**: ✅ 53 tests passing
- [x] Test: Parse comma-separated addresses (8 test cases)
- [x] Test: Parse JSON file format (8 test cases)
- [x] Test: Load from `traders.json` with defaults
- [x] Test: Backward compat with `TARGET_WHALE_ADDRESS` (4 test cases)
- [x] Test: Invalid address format rejected (14 validation tests)
- [x] Test: Duplicate addresses deduplicated
- [x] Test: 0x prefix stripped correctly
- [x] Test: Topic hex padding correct (5 test cases)
- [x] Test: get_by_topic/get_by_address lookups (6 test cases)
- [x] Test: Empty config returns error

**Documentation**: ✅ COMPLETE
- [x] Update `.env.example` with `TRADER_ADDRESSES` and 3 methods
- [x] Create `traders.json.example` with format documentation
- [x] Create `STEP_2_1_SUMMARY.md` with full implementation details
- [x] Added comprehensive rustdoc comments to all public functions

**Test Results**:
```bash
cargo test config::tests --lib -- --test-threads=1
# Result: 53 passed; 0 failed

cargo test --lib
# Result: 110 passed; 0 failed (53 new + 57 existing)
```

---

### Step 2.2: WebSocket Multi-Topic Subscription

**Files to modify**:
- `src/main.rs`
- `src/settings.rs`

**Implementation**:
- [ ] Replace `TARGET_TOPIC_HEX` with `TradersConfig`
- [ ] Modify subscription to use topic array:
  ```rust
  "topics": [[EVENT_SIG], Value::Null, traders.topic_filter()]
  ```
- [ ] Update `parse_event()` to extract trader from topic[2]
- [ ] Return trader address/label in `ParsedEvent`
- [ ] Log which trader triggered each event
- [ ] Handle case: >10 traders (switch to client-side filtering)

**Measurable Result**:
```bash
# Logs show trader identification
# [Whale1] BUY_FILL | 500 shares | $175.00
# [Whale2] SELL_FILL | 200 shares | $62.00
```

**Testing**:
- [ ] Test: Subscription includes all configured topics
- [ ] Test: Events correctly attributed to traders
- [ ] Test: Unknown trader events ignored (if client-side filter)
- [ ] Integration test: Two test addresses, verify both detected

**Documentation**:
- [ ] Update startup message to show all monitored traders
- [ ] Document topic filter behavior

---

### Step 2.3: Per-Trader State Management

**Files to create**:
- `src/trader_state.rs`

**Files to modify**:
- `src/main.rs`

**Implementation**:
- [ ] Define `TraderState` struct:
  ```rust
  struct TraderState {
      address: String,
      label: String,
      total_copied_usd: f64,
      trades_today: u32,
      successful_trades: u32,
      failed_trades: u32,
      last_trade_ts: Option<Instant>,
      daily_reset_ts: DateTime<Utc>,
  }
  ```
- [ ] Define `TraderManager` with `FxHashMap<String, TraderState>`
- [ ] Implement `record_trade()` - update stats after execution
- [ ] Implement `get_stats()` - for monitoring
- [ ] Implement `reset_daily()` - called at midnight UTC
- [ ] Persist stats to `trader_stats` table periodically

**Measurable Result**:
```bash
# Stats visible in logs
# [Whale1] Stats: 15 trades, $450 copied, 93% success
```

**Testing**:
- [ ] Test: Stats accumulate correctly
- [ ] Test: Daily reset works
- [ ] Test: Stats persist to DB

**Documentation**:
- [ ] Add stats explanation to features doc

---

### Step 2.4: Trader Comparison CLI Tool

**Files to create**:
- `src/bin/trader_comparison.rs`

**Implementation**:
- [ ] Compare your positions vs each trader's observed trades
- [ ] Calculate tracking accuracy (% of whale trades copied)
- [ ] Calculate fill rate per trader
- [ ] Show P&L attribution per trader
- [ ] Support `--trader <label>` filter

**Measurable Result**:
```bash
cargo run --release --bin trader_comparison

# === TRADER COMPARISON ===
# Trader        Trades    Copied    Fill%    Your P&L
# --------------------------------------------------------
# Whale1            45        42      93%      +$125.50
# Whale2            23        20      87%       -$12.30
```

**Testing**:
- [ ] Test: Correct attribution of trades
- [ ] Test: Handles traders with no trades

**Documentation**:
- [ ] Add to README.md utility binaries section

---

### Phase 2 Completion Checklist

- [ ] All Step 2.x tasks completed
- [ ] All unit tests pass
- [ ] Integration test: Run with 2+ traders for 1 hour
- [ ] Verify correct trade attribution
- [ ] Documentation updated
- [ ] Code reviewed and merged

**Phase 2 Deliverable**: Bot monitors multiple traders with per-trader configuration, stats tracking, and comparison tools.

---

## Phase 3: Trade Aggregation

**Goal**: Combine rapid small trades into single orders for efficiency.

**End Result**:
- Small trades within 500-800ms window aggregated
- Large trades (4000+ shares) execute immediately
- Configurable aggregation parameters
- Reduced fee impact and API calls

### Step 3.1: Aggregator Module

**Files to create**:
- `src/aggregator.rs`

**Implementation**:
- [ ] Define `AggregationConfig`:
  ```rust
  struct AggregationConfig {
      window_duration: Duration,    // Default: 800ms
      min_trades: usize,            // Minimum trades to aggregate (2)
      max_pending_usd: f64,         // Force flush threshold ($500)
      bypass_threshold: f64,        // Large trades skip aggregation (4000 shares)
  }
  ```
- [ ] Define `PendingTrade` for accumulation
- [ ] Define `AggregatedTrade` output struct
- [ ] Implement `TradeAggregator`:
  - `add_trade()` - returns `Some(AggregatedTrade)` if ready
  - `flush_expired()` - check and flush expired windows
  - `flush_all()` - for shutdown
- [ ] Use `(token_id, side)` as aggregation key
- [ ] Calculate weighted average price
- [ ] Track aggregation count for logging

**Measurable Result**:
```bash
# Aggregator combines 3 small trades into 1
# [AGG] 3 trades -> 1 order | 150 shares @ 0.4523 avg
```

**Testing**:
- [ ] Test: Large trades bypass aggregation
- [ ] Test: Window expires and flushes
- [ ] Test: Max USD threshold triggers flush
- [ ] Test: Weighted average price correct
- [ ] Test: Different tokens don't aggregate together
- [ ] Benchmark: <100us overhead per add_trade()

**Documentation**:
- [ ] Document aggregation behavior
- [ ] Add configuration options to docs

---

### Step 3.2: Integrate Aggregator into Main Loop

**Files to modify**:
- `src/main.rs`
- `src/settings.rs`

**Implementation**:
- [ ] Add aggregation config to settings
- [ ] Add `AGG_ENABLED`, `AGG_WINDOW_MS`, `AGG_BYPASS_SHARES` env vars
- [ ] Initialize `TradeAggregator` in main
- [ ] Wrap in `Arc<Mutex<>>` for async access
- [ ] Modify `handle_event()` to use aggregator
- [ ] Spawn background task for window expiry checks
- [ ] Ensure shutdown flushes pending aggregations
- [ ] Log aggregation stats

**Measurable Result**:
```bash
# Bot logs aggregation activity
# [AGG] Window flush: 2 trades combined into 1 order
# [AGG] Bypass: 4500 shares executed immediately
```

**Testing**:
- [ ] Integration test: Rapid small trades aggregate
- [ ] Integration test: Large trades don't wait
- [ ] Test: AGG_ENABLED=false disables aggregation
- [ ] Test: Shutdown flushes pending

**Documentation**:
- [ ] Update configuration docs
- [ ] Add aggregation tuning guide

---

### Step 3.3: Aggregation Analytics

**Implementation**:
- [ ] Add `aggregation_count` field to trade records
- [ ] Add `aggregation_window_ms` field
- [ ] Create query for aggregation efficiency stats
- [ ] Add to position_monitor or separate tool

**Measurable Result**:
```bash
# Aggregation stats
# Total orders: 150
# Aggregated orders: 45 (30%)
# Avg trades per aggregation: 2.8
# Estimated fees saved: $X
```

**Testing**:
- [ ] Test: Stats calculate correctly

---

### Phase 3 Completion Checklist

- [ ] All Step 3.x tasks completed
- [ ] All unit tests pass
- [ ] Integration test: Verify aggregation under rapid trade conditions
- [ ] Verify large trades still execute immediately
- [ ] Documentation updated
- [ ] Code reviewed and merged

**Phase 3 Deliverable**: Bot aggregates small rapid trades while preserving immediate execution for large trades.

---

## Phase 4: Research Tooling

**Goal**: Enable trader discovery and strategy analysis.

**End Result**:
- HTTP API exports data from Rust bot
- Python scripts for trader discovery and backtesting
- Jupyter notebooks for analysis

### Step 4.1: HTTP Data Export API

**Files to create**:
- `src/api.rs`

**Files to modify**:
- `src/main.rs`
- `Cargo.toml`

**Implementation**:
- [ ] Add `axum` dependency
- [ ] Add `API_ENABLED`, `API_PORT` env vars
- [ ] Implement endpoints:
  - `GET /health` - bot status
  - `GET /positions` - current positions JSON
  - `GET /trades?limit=N&since=TS` - trade history
  - `GET /traders` - trader stats
  - `GET /stats` - overall statistics
- [ ] Start API server on separate tokio task
- [ ] Bind to localhost only (security)

**Measurable Result**:
```bash
curl http://localhost:8080/positions | jq
# [{"token_id":"...","net_shares":150.0,"avg_price":0.45,"pnl":12.5}]
```

**Testing**:
- [ ] Test: All endpoints return valid JSON
- [ ] Test: API doesn't impact main loop latency
- [ ] Test: API disabled by default

**Documentation**:
- [ ] Document API endpoints
- [ ] Add curl examples

---

### Step 4.2: Python Research Scripts

**Files to create**:
- `research/requirements.txt`
- `research/fetch_leaderboard.py`
- `research/analyze_trader.py`
- `research/backtest_strategy.py`
- `research/notebooks/analysis.ipynb`

**Implementation**:
- [ ] Setup Python project with dependencies (pandas, httpx, matplotlib)
- [ ] `fetch_leaderboard.py` - scrape Polymarket leaderboards
- [ ] `analyze_trader.py` - fetch and analyze trader history
- [ ] `backtest_strategy.py` - simulate copy trading on historical data
- [ ] Jupyter notebook for interactive analysis
- [ ] Scripts consume data from HTTP API or SQLite directly

**Measurable Result**:
```bash
cd research
python fetch_leaderboard.py --top 50 > traders.csv
python analyze_trader.py 0xabc123... --days 30
```

**Testing**:
- [ ] Scripts run without errors
- [ ] Output formats are consistent

**Documentation**:
- [ ] README in research/ directory
- [ ] Document workflow for trader discovery

---

### Phase 4 Completion Checklist

- [ ] All Step 4.x tasks completed
- [ ] API tested with real bot
- [ ] Python scripts documented
- [ ] End-to-end workflow verified

**Phase 4 Deliverable**: Research toolkit for trader discovery and strategy analysis.

---

## Phase 5: Live P&L Tracking

**Goal**: Enhance position monitor with real-time market prices and P&L calculation.

**End Result**:
- Position monitor fetches current prices from CLOB API
- Displays unrealized P&L per position
- Shows total portfolio value and daily P&L change
- Price caching to avoid API rate limits

### Step 5.1: Price Fetcher Module

**Files to create**:
- `src/prices.rs`

**Implementation**:
- [ ] Define `PriceCache` with TTL (default: 30 seconds)
- [ ] Implement `fetch_price(token_id, side)` using CLOB API
- [ ] Implement batch price fetching for multiple tokens
- [ ] Handle API errors gracefully (return cached or None)
- [ ] Add rate limiting (max 10 requests/second)

**API Endpoint**: `GET https://clob.polymarket.com/price?token_id=<id>&side=<buy|sell>`

**Testing**:
- [ ] Test: Cache returns cached price within TTL
- [ ] Test: Cache refreshes after TTL expires
- [ ] Test: API errors don't crash, return cached value
- [ ] Test: Rate limiting works correctly

---

### Step 5.2: Enhance Position Monitor with P&L

**Files to modify**:
- `src/bin/position_monitor.rs`

**Implementation**:
- [ ] Integrate `PriceCache` into position monitor
- [ ] Fetch current bid/ask for each position's token
- [ ] Calculate unrealized P&L: `(current_price - avg_entry) * shares`
- [ ] For long positions, use bid price (what you'd sell at)
- [ ] For short positions, use ask price (what you'd buy at)
- [ ] Display P&L with color coding (green positive, red negative)
- [ ] Add `--no-prices` flag to skip price fetching
- [ ] Show last price update timestamp

**Measurable Result**:
```bash
cargo run --release --bin position_monitor

# === CURRENT POSITIONS ===
# Token ID             Shares    Avg Entry    Current    Unrealized P&L
# ------------------------------------------------------------------------
# 1234567890...        150.00       0.4500     0.5200        +$10.50
# 9876543210...        -50.00       0.6200     0.5800         +$2.00
#
# Portfolio Value: $XXX.XX | Daily P&L: +$XX.XX
# Prices updated: 5 seconds ago
```

**Testing**:
- [ ] Test: P&L calculates correctly for long positions
- [ ] Test: P&L calculates correctly for short positions
- [ ] Test: `--no-prices` skips API calls
- [ ] Test: Handles tokens with no price data gracefully

---

### Step 5.3: Portfolio Summary Statistics

**Implementation**:
- [ ] Calculate total portfolio value (sum of position values)
- [ ] Track daily starting value (persist to DB or file)
- [ ] Calculate daily P&L change
- [ ] Add `--json` flag with full portfolio data
- [ ] Optionally integrate with position_monitor or create separate tool

**Testing**:
- [ ] Test: Portfolio value sums correctly
- [ ] Test: Daily P&L resets at midnight UTC

---

### Phase 5 Completion Checklist

- [ ] All Step 5.x tasks completed
- [ ] All unit tests pass
- [ ] P&L displays correctly for real positions
- [ ] Price caching works (verify with rate limit testing)
- [ ] Documentation updated

**Phase 5 Deliverable**: Position monitor shows real-time P&L with live market prices.

---

## Appendix A: New Dependencies

```toml
# Cargo.toml additions

[dependencies]
# Phase 1: Persistence
rusqlite = { version = "0.31", features = ["bundled"] }

# Phase 4: API (optional)
axum = { version = "0.7", optional = true }

[features]
default = []
api = ["axum"]
```

---

## Appendix B: Environment Variables Summary

```bash
# Phase 1: Persistence
DB_ENABLED=true              # Enable SQLite persistence
DB_PATH=trades.db            # Database file path

# Phase 2: Multi-Trader
TRADER_ADDRESSES=addr1,addr2 # Comma-separated addresses
# Or use traders.json file

# Phase 3: Aggregation
AGG_ENABLED=true             # Enable trade aggregation
AGG_WINDOW_MS=800            # Aggregation window
AGG_BYPASS_SHARES=4000       # Bypass threshold

# Phase 4: API
API_ENABLED=false            # Enable HTTP API
API_PORT=8080                # API port
```

---

## Appendix C: File Structure After All Phases

```
src/
  main.rs                    # Modified
  lib.rs                     # Minor changes
  settings.rs                # Extended
  risk_guard.rs              # Unchanged
  market_cache.rs            # Unchanged
  models.rs                  # Extended

  config/
    mod.rs                   # NEW
    traders.rs               # NEW

  persistence/
    mod.rs                   # NEW
    schema.sql               # NEW
    store.rs                 # NEW

  aggregator.rs              # NEW
  trader_state.rs            # NEW
  api.rs                     # NEW (optional)

  bin/
    position_monitor.rs      # NEW
    trade_history.rs         # NEW
    trader_comparison.rs     # NEW
    # ... existing bins ...

research/
  requirements.txt           # NEW
  fetch_leaderboard.py       # NEW
  analyze_trader.py          # NEW
  backtest_strategy.py       # NEW
  notebooks/
    analysis.ipynb           # NEW
```

---

## Progress Tracking

### Phase 1: Foundation
- [x] Step 1.1: SQLite Schema & Store Module ✅ (52 tests passing)
- [x] Step 1.2: Integrate Persistence into Main Bot ✅ (57 tests passing)
- [x] Step 1.3: Position Monitor CLI Tool ✅ (6 unit tests + integration test)
- [x] Step 1.4: Trade History CLI Tool ✅ (20 unit tests + integration test)
- [ ] Phase 1 Complete

### Phase 2: Multi-Trader
- [ ] Step 2.1: Trader Configuration Module
- [ ] Step 2.2: WebSocket Multi-Topic Subscription
- [ ] Step 2.3: Per-Trader State Management
- [ ] Step 2.4: Trader Comparison CLI Tool
- [ ] Phase 2 Complete

### Phase 3: Trade Aggregation
- [ ] Step 3.1: Aggregator Module
- [ ] Step 3.2: Integrate Aggregator into Main Loop
- [ ] Step 3.3: Aggregation Analytics
- [ ] Phase 3 Complete

### Phase 4: Research Tooling
- [ ] Step 4.1: HTTP Data Export API
- [ ] Step 4.2: Python Research Scripts
- [ ] Phase 4 Complete

---

*Last updated: 2026-01-20*
