# Rust Bot Feature Parity Implementation Plan

> Goal: Bring the Rust copy trading bot to feature parity with the Python implementation where it matters, while preserving the <100ms latency advantage.

## Overview

| Phase | Feature | Status | Priority |
|-------|---------|--------|----------|
| 1     | SQLite Persistence + Position Tracking | ✅ Complete | **High** |
| 2     | Multi-Trader Monitoring | ✅ Complete | **High** |
| 3     | Trade Aggregation | ✅ Complete | Medium |
| 4     | Research Tooling (incl. CSV Import) | Pending | Medium |
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

### Step 2.2: WebSocket Multi-Topic Subscription ✅ COMPLETE

**Files modified**:
- `src/main.rs` - Multi-topic subscription, trader extraction
- `src/settings.rs` - TradersConfig integration
- `src/models.rs` - Added trader fields to ParsedEvent

**Implementation**: ✅ ALL COMPLETE
- [x] Replace `TARGET_TOPIC_HEX` with `TradersConfig` in Config
- [x] Modify subscription to use topic array via `build_subscription_message()`
- [x] Smart filtering: server-side for ≤10 traders, client-side for >10
- [x] Update `parse_event()` to extract trader from topic[2]
- [x] Return trader_address and trader_label in `ParsedEvent`
- [x] Log which trader triggered each event with label
- [x] Handle case: >10 traders (switch to client-side filtering)

**Testing**: ✅ 13 new tests in main.rs
- [x] Test: ParsedEvent has trader info fields
- [x] Test: Address extraction from topic hex
- [x] Test: Subscription message format (single and multi-topic)
- [x] Test: >10 traders triggers client-side filtering
- [x] Test: Config includes TradersConfig

**Documentation**: ✅ COMPLETE
- [x] Startup logs show trader count and filtering mode
- [x] Added rustdoc for new functions

---

### Step 2.3: Per-Trader State Management ✅ COMPLETE

**Files created**:
- `src/trader_state.rs` - TraderState, TradeStatus, TraderManager (444 lines)

**Files modified**:
- `src/lib.rs` - Module export
- `src/main.rs` - TraderManager integration
- `src/persistence/store.rs` - Database persistence methods

**Implementation**: ✅ ALL COMPLETE
- [x] Define `TraderState` struct with all required fields
- [x] Define `TradeStatus` enum (Success, Failed, Partial, Skipped)
- [x] Define `TraderManager` with HashMap<String, TraderState>
- [x] Implement `new()` - initialize from TradersConfig
- [x] Implement `record_trade()` - update stats after execution
- [x] Implement `get_state()` / `get_all_states()` - for monitoring
- [x] Implement `check_daily_reset()` - resets at midnight UTC
- [x] Implement `get_summary_stats()` - aggregate stats
- [x] Persist stats to `trader_stats` table via `upsert_trader_stats()`
- [x] Integrated into main.rs with periodic logging (every 60s)

**Testing**: ✅ 16 new tests
- [x] Test: TraderState initializes with correct defaults
- [x] Test: record_trade increments counters correctly
- [x] Test: Success/Failed/Partial all tracked separately
- [x] Test: total_copied_usd accumulates correctly
- [x] Test: Daily reset clears daily counters but keeps totals
- [x] Test: TraderManager initializes all traders from config
- [x] Test: get_state returns None for unknown trader

**Documentation**: ✅ COMPLETE
- [x] Rustdoc comments on all public functions
- [x] Stats logging format documented

---

### Step 2.4: Trader Comparison CLI Tool ✅ COMPLETE

**Files created**:
- `src/bin/trader_comparison.rs` - CLI tool (965 lines, 32 tests)

**Files modified**:
- `src/persistence/store.rs` - Added get_trader_trade_metrics() methods
- `Cargo.toml` - Registered binary

**Implementation**: ✅ ALL COMPLETE
- [x] Compare your positions vs each trader's observed trades
- [x] Calculate tracking accuracy (copy_rate = copied / observed %)
- [x] Calculate success rate per trader
- [x] Calculate average fill rate per trader
- [x] Show total USD copied per trader
- [x] Support `--trader <label>` filter
- [x] Support `--since <timestamp>` filter
- [x] Support `--format <table|csv|json>` output

**Output formats**:
```bash
# Table (default)
cargo run --bin trader_comparison

# CSV export
cargo run --bin trader_comparison -- --format csv > traders.csv

# JSON export
cargo run --bin trader_comparison -- --format json
```

**Testing**: ✅ 32 tests
- [x] Test: CLI argument parsing (7 tests)
- [x] Test: Database queries and filtering (6 tests)
- [x] Test: Trade metrics calculation (3 tests)
- [x] Test: Table output formatting (5 tests)
- [x] Test: CSV output (3 tests)
- [x] Test: JSON output (3 tests)
- [x] Test: Edge cases (empty data, filtering)

---

### Phase 2 Completion Checklist ✅ COMPLETE

- [x] All Step 2.x tasks completed (2.1, 2.2, 2.3, 2.4)
- [x] All unit tests pass: `cargo test` (210 tests total)
- [x] Multi-trader configuration via env var or traders.json
- [x] Per-trader stats tracking with daily reset
- [x] Comparison CLI tool with multiple output formats

**Phase 2 Deliverable**: Bot monitors multiple traders with per-trader configuration, stats tracking, and comparison tools.

---

## Phase 3: Trade Aggregation

**Goal**: Combine rapid small trades into single orders for efficiency.

**End Result**:
- Small trades within 500-800ms window aggregated
- Large trades (4000+ shares) execute immediately
- Configurable aggregation parameters
- Reduced fee impact and API calls

### Step 3.1: Aggregator Module ✅ COMPLETE

**Files created**:
- `src/aggregator.rs` (720 lines, 20 tests)

**Implementation**: ✅ ALL COMPLETE
- [x] Define `AggregationConfig`:
  ```rust
  struct AggregationConfig {
      window_duration: Duration,    // Default: 800ms
      min_trades: usize,            // Minimum trades to aggregate (2)
      max_pending_usd: f64,         // Force flush threshold ($500)
      bypass_threshold: f64,        // Large trades skip aggregation (4000 shares)
  }
  ```
- [x] Define `PendingTrade` for accumulation
- [x] Define `AggregatedTrade` output struct
- [x] Implement `TradeAggregator`:
  - `add_trade()` - returns `Some(AggregatedTrade)` if ready
  - `flush_expired()` - check and flush expired windows
  - `flush_all()` - for shutdown
- [x] Use `(token_id, side)` as aggregation key
- [x] Calculate weighted average price
- [x] Track aggregation count for logging

**Measurable Result**:
```bash
# Aggregator combines 3 small trades into 1
# [AGG] 3 trades -> 1 order | 150 shares @ 0.4523 avg
```

**Testing**: ✅ 20 tests passing
- [x] Test: Large trades bypass aggregation
- [x] Test: Window expires and flushes
- [x] Test: Max USD threshold triggers flush
- [x] Test: Weighted average price correct
- [x] Test: Different tokens don't aggregate together
- [x] Benchmark: 654ns per add_trade() (152x better than 100µs requirement)

**Documentation**:
- [x] Rustdoc comments on all public types and methods

---

### Step 3.2: Integrate Aggregator into Main Loop ✅ COMPLETE

**Files modified**:
- `src/main.rs` - Added aggregator initialization, background flush task, shutdown handling
- `src/settings.rs` - Added aggregation config fields (7 new tests)

**Implementation**: ✅ ALL COMPLETE
- [x] Add aggregation config to settings (agg_enabled, agg_window_ms, agg_bypass_shares)
- [x] Add `AGG_ENABLED`, `AGG_WINDOW_MS`, `AGG_BYPASS_SHARES` env vars
- [x] Initialize `TradeAggregator` in main (wrapped in `Arc<Mutex<>>`)
- [x] Modify `handle_event()` to use aggregator with bypass/pending logic
- [x] Spawn background task for window expiry checks (every 100ms)
- [x] Shutdown flushes pending aggregations via Ctrl+C handler
- [x] Log aggregation stats with `[AGG]` prefix

**Measurable Result**:
```bash
# Bot logs aggregation activity
# [AGG] Bypass: 4500 shares executed immediately
# [AGG] Aggregated: 3 trades -> 150 shares @ 0.4523 avg
# [AGG] Pending: trade added to aggregation window
# [AGG] Window flush: 2 trades combined into 1 order
# [AGG] Shutdown: flushing N pending aggregations
```

**Testing**: ✅ 7 new tests in settings.rs
- [x] Test: Default aggregation config values
- [x] Test: AGG_ENABLED=true/false/1/0 parsing
- [x] Test: Custom window and bypass values
- [x] Test: Disabled by default (AGG_ENABLED=false default)

**Documentation**:
- [x] Environment variable descriptions in code

---

### Step 3.3: Aggregation Analytics ✅ COMPLETE

**Files modified**:
- `src/persistence/store.rs` - Added AggregationStats, aggregation fields to TradeRecord (7 new tests)
- `src/persistence/schema.sql` - Added aggregation_window_ms column
- `src/bin/position_monitor.rs` - Added `--stats` flag (3 new tests)

**Implementation**: ✅ ALL COMPLETE
- [x] Add `aggregation_count` field to trade records (Option<u32>)
- [x] Add `aggregation_window_ms` field (Option<u64>)
- [x] Update TradeRecord struct with aggregation fields
- [x] Update database schema with aggregation_window_ms column
- [x] Update insert_trade and get_recent_trades queries
- [x] Create `AggregationStats` struct
- [x] Create `get_aggregation_stats()` query method
- [x] Add `--stats` flag to position_monitor CLI

**Measurable Result**:
```bash
cargo run --bin position_monitor -- --stats

# === AGGREGATION STATISTICS ===
#
# Total orders:          8
# Aggregated orders:     3 (37.5%)
# Avg trades per agg:    3.0
# Estimated fees saved:  $0.12
```

**Testing**: ✅ 10 new tests
- [x] Test: TradeRecord with aggregation fields
- [x] Test: TradeRecord without aggregation fields
- [x] Test: Insert trade with aggregation data
- [x] Test: Retrieve trade without aggregation data
- [x] Test: Stats calculate correctly for all non-aggregated trades
- [x] Test: Stats calculate correctly with mixed aggregated/non-aggregated
- [x] Test: Stats handle empty database correctly
- [x] Test: Stats display formatting
- [x] Test: Fee calculation

---

### Phase 3 Completion Checklist ✅ COMPLETE

- [x] All Step 3.x tasks completed (3.1, 3.2, 3.3)
- [x] All unit tests pass: 165+ tests
- [x] Aggregator module with 20 tests (654ns per add_trade)
- [x] Main loop integration with bypass/pending/flush logic
- [x] Aggregation analytics with `--stats` CLI display
- [x] Documentation updated in code

**Phase 3 Deliverable**: Bot aggregates small rapid trades while preserving immediate execution for large trades. ✅

---

## Phase 4: Research Tooling

**Goal**: Enable trader discovery, strategy analysis, and historical data import.

**End Result**:
- Legacy CSV data imported into SQLite for historical analysis
- HTTP API exports data from Rust bot
- Python scripts for trader discovery and backtesting
- Jupyter notebooks for analysis

### Step 4.0: Import Legacy CSV Data ✅ COMPLETE

**Goal**: Import historical trades from `matches_optimized.csv` into SQLite database for analysis and position continuity.

**Files created**:
- `src/bin/import_csv.rs` (7 unit tests)
- `src/persistence/store.rs` (added `tx_hash_exists()` method)

**Implementation**:
- [x] Parse CSV format: `timestamp,block,clob_asset_id,usd_value,shares,price_per_share,direction,order_status,...`
- [x] Map CSV fields to `TradeRecord` struct:
  - `clob_asset_id` → `token_id`
  - `direction` (BUY_FILL/SELL_FILL) → `side` (BUY/SELL)
  - `order_status` → `status`
  - `price_per_share` → `whale_price`
  - `shares` → `whale_shares`
  - `usd_value` → `whale_usd`
  - `tx_hash` → `tx_hash`
  - `timestamp` → `timestamp_ms`
  - `block` → `block_number`
  - `is_live` → `is_live`
- [x] Handle different order statuses: SKIPPED_SMALL, SKIPPED_PROBABILITY, MOCK_ONLY, SUCCESS, etc.
- [x] Skip duplicates (check by tx_hash using `TradeStore::tx_hash_exists()`)
- [x] Support `--dry-run` flag to preview import
- [x] Support `--db` flag for custom database path (default: trades.db)
- [x] Support `--skip-duplicates` flag (default: true)
- [x] Show import summary: total rows, imported, skipped, errors

**Testing**: 7 unit tests passing
- [x] Test: Parse CSV row (BUY_FILL)
- [x] Test: Parse CSV row (SELL_FILL)
- [x] Test: Read CSV file with multiple rows
- [x] Test: CLI args parsing
- [x] Test: Import trades basic functionality
- [x] Test: Import trades with duplicate detection
- [x] Test: Dry-run mode doesn't write to database

**Measurable Result**:
```bash
cargo run --bin import_csv -- matches_optimized.csv --db trades.db

# Import Summary:
# Total rows:      1567
# Imported:        1559
# Skipped:            8 (duplicates)
# Errors:             2 (malformed rows)
#
# Position monitor shows 129 historical positions!
```

**Additional features**:
- [x] Populate `our_*` fields for executed trades (200 OK, MOCK_ONLY) for position tracking
- [x] Handle malformed rows gracefully (flexible CSV parser, continues on errors)

---

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
    import_csv.rs            # NEW (Phase 4)
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

### Phase 1: Foundation ✅ COMPLETE
- [x] Step 1.1: SQLite Schema & Store Module ✅ (57 tests passing)
- [x] Step 1.2: Integrate Persistence into Main Bot ✅
- [x] Step 1.3: Position Monitor CLI Tool ✅ (6 unit tests + 5 integration tests)
- [x] Step 1.4: Trade History CLI Tool ✅ (20 unit tests + 1 integration test)
- [x] Phase 1 Complete ✅

### Phase 2: Multi-Trader ✅ COMPLETE
- [x] Step 2.1: Trader Configuration Module ✅ (54 tests passing)
- [x] Step 2.2: WebSocket Multi-Topic Subscription ✅ (13 tests)
- [x] Step 2.3: Per-Trader State Management ✅ (16 tests)
- [x] Step 2.4: Trader Comparison CLI Tool ✅ (32 tests)
- [x] Phase 2 Complete ✅

### Phase 3: Trade Aggregation ✅ COMPLETE
- [x] Step 3.1: Aggregator Module ✅ (20 tests, 654ns perf)
- [x] Step 3.2: Integrate Aggregator into Main Loop ✅ (7 new tests)
- [x] Step 3.3: Aggregation Analytics ✅ (10 new tests)
- [x] Phase 3 Complete ✅

### Phase 4: Research Tooling
- [x] Step 4.0: Import Legacy CSV Data ✅ (8 tests, 129 positions imported)
- [ ] Step 4.1: HTTP Data Export API
- [ ] Step 4.2: Python Research Scripts
- [ ] Phase 4 Complete

---

*Last updated: 2026-01-20*
