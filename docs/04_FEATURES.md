# Features Overview

This document explains what the Polymarket Copy Trading Bot does and how it works.

## Table of Contents

1. [Overview](#1-overview)
2. [Core Features](#2-core-features)
3. [Multi-Trader Monitoring](#3-multi-trader-monitoring)
4. [Trade Aggregation](#4-trade-aggregation)
5. [Persistence & Analytics](#5-persistence--analytics)
6. [Live P&L Tracking](#6-live-pl-tracking)
7. [Hot Configuration Reload](#7-hot-configuration-reload)
8. [CLOB Trade History Tool](#8-clob-trade-history-tool)
9. [Auto-Claim (Redemption)](#9-auto-claim-redemption)
10. [Trading Flow](#10-trading-flow-step-by-step)
11. [Performance Characteristics](#11-performance-characteristics)
12. [Limitations](#12-limitations)
13. [Safety Features](#13-safety-features-summary)
14. [Understanding Output](#14-understanding-the-output)
15. [Next Steps](#15-next-steps)

## 1. Overview

### 1.1 What This Bot Does

The bot monitors blockchain events for trades made by a specific "whale" (successful trader) on Polymarket and automatically copies those trades with scaled-down position sizes.

**Key Strategy Points:**
- **2% Position Scaling:** Copies trades at 2% of whale's size (configurable)
- **Tiered Execution:** Different strategies based on trade size (4000+, 2000+, 1000+, <1000)
- **Risk Guards:** Multi-layer safety system prevents dangerous trades
- **Intelligent Pricing:** Price buffers optimize fill rates while minimizing slippage
- **Automatic Retries:** Resubmission logic maximizes fill rates

For complete strategy details, see [Trading Strategy Guide](05_STRATEGY.md).

## 2. Core Features

### 2.1 Real-Time Trade Detection

- **WebSocket Connection:** Connects to Polygon blockchain via WebSocket for real-time event monitoring
- **Event Filtering:** Only processes trades from your target whale address
- **Blockchain Events:** Monitors `OrdersFilled` events from Polymarket's order book contracts

**How it works:**
1. Bot subscribes to blockchain logs
2. Filters for trades from target whale address
3. Parses trade details (token, size, price, side)
4. Queues trade for processing

---

### 2.2 Intelligent Position Sizing

The bot doesn't copy trades at 1:1 size. Instead, it uses scaled positions:

- **Default Scaling:** 2% of whale's position size
- **Minimum Size:** Orders below $1.01 USD are skipped (prevents dust)
- **Probabilistic Sizing:** Very small positions may be probabilistically executed or skipped

**Example:**
- Whale buys 10,000 shares at $0.50 = $5,000
- Bot buys 200 shares at $0.50 = $100 (2% of $5,000)

**Why scaling:**
- Reduces risk exposure
- Allows copying whales with larger accounts
- Prevents position size issues if whale uses full account

---

### 2.3 Tiered Execution Strategy

Different trade sizes get different execution strategies:

| Trade Size (Shares) | Price Buffer | Size Multiplier | Resubmit Buffer | Strategy |
|---------------------|--------------|-----------------|-----------------|----------|
| 4000+ (Large)       | +0.01        | 1.25x           | +0.01           | Aggressive |
| 2000-3999 (Medium)  | +0.01        | 1.0x            | +0.01           | Standard |
| 1000-1999 (Small)   | +0.00        | 1.0x            | +0.01           | Conservative |
| <1000 (Very Small)  | +0.00        | 1.0x            | +0.01           | Conservative |

**Price Buffer:** Additional amount paid above whale's price (improves fill rate)
**Size Multiplier:** Your position size relative to whale (1.25x = 25% larger than normal scaling)
**Resubmit Buffer:** Maximum additional buffer allowed during resubmission attempts

**Large trades (4000+ shares):**
- More aggressive (higher buffer, larger size)
- More resubmit attempts if order fails
- Price chasing on first retry

**Small trades (<1000 shares):**
- More conservative (no buffer)
- Fewer resubmit attempts
- No price chasing

---

### 2.4 Order Types

The bot uses different order types based on trade characteristics:

**FAK (Fill and Kill):**
- Executes immediately or cancels
- Used for buy orders (most trades)
- Fast execution, no order book placement

**GTD (Good Till Date):**
- Places order on book with expiration
- Used for:
  - Sell orders (all sells)
  - Final retry attempt on failed buys
- Expires after:
  - 61 seconds for live markets
  - 1800 seconds (30 min) for non-live markets

---

### 2.5 Automatic Order Resubmission

If an order fails to fill completely:

**Retry Logic:**
- Up to 4-5 attempts (depending on trade size)
- Price escalation on first retry for large trades
- Exponential backoff delays for small trades

**Example Flow:**
1. Initial order fails (FAK)
2. Retry #1: Same price or +0.01 (if large trade)
3. Retry #2-4: Same price (flat retries)
4. Final attempt: GTD order with spread-crossing

**GTD Spread-Crossing:**
On the final attempt, the bot fetches the current best ask from the order book and sets the GTD price to `min(max_price, best_ask)`. This ensures the GTD order crosses the spread and has a better chance of filling, rather than sitting passively on the book.

**Why this helps:**
- Market conditions change quickly
- Improves fill rate on volatile markets
- Balances speed vs. execution quality
- GTD spread-crossing maximizes final attempt success

---

### 2.6 Risk Management (Circuit Breaker Protection)

Protects you from copying trades in dangerous conditions:

**Triggers:**
- Multiple large trades in short time window
- Low order book depth (thin liquidity)
- Rapid-fire trading patterns

**Actions:**
- Blocks trades for specified duration (default: 2 minutes)
- Checks order book depth before allowing trades
- Prevents copying during potential manipulation

**Configuration:**
- `CB_LARGE_TRADE_SHARES`: Minimum size to trigger (default: 1500)
- `CB_CONSECUTIVE_TRIGGER`: Number of trades to trigger (default: 2)
- `CB_SEQUENCE_WINDOW_SECS`: Time window (default: 30 seconds)
- `CB_MIN_DEPTH_USD`: Minimum liquidity required (default: $200)
- `CB_TRIP_DURATION_SECS`: Block duration (default: 120 seconds)

---

### 2.7 Market Cache System

**Purpose:** Fast lookups without API delays

**Cached Data:**
- Market information (token IDs, slugs)
- Live/non-live status
- Sport-specific market data (ATP, Ligue 1)

**Refresh:** Automatically updated in background (periodic refresh)

**Benefits:**
- Faster execution (no API wait times)
- Reduces API rate limits
- More reliable (less dependent on external APIs)

---

### 2.8 Sport-Specific Optimizations

**ATP Markets:**
- Additional +0.01 price buffer
- Optimized for tennis market characteristics

**Ligue 1 Markets:**
- Additional +0.01 price buffer
- Optimized for soccer market characteristics

**Other Markets:**
- Standard execution strategy
- No additional buffers

**Automatic Detection:** Bot automatically detects market type and applies appropriate strategy.

---

### 2.9 Comprehensive Logging

**Console Output:**
- Real-time trade information
- Color-coded status messages
- Fill percentages
- Market conditions

**CSV Logging:**
- File: `matches_optimized.csv`
- All trades logged with timestamps
- Includes: block number, token ID, USD value, shares, price, direction, status, order book data, transaction hash, live status

**Use Cases:**
- Performance analysis
- Debugging
- Audit trail
- Post-trade analysis

---

### 2.10 Live Market Detection

**Automatic Detection:**
- Checks if market is "live" (event currently happening)
- Different expiration times for live vs. non-live markets
- Faster execution for live markets

**Impact:**
- Live markets: 61-second GTD expiration (faster)
- Non-live: 30-minute GTD expiration (more patient)

---

## 3. Multi-Trader Monitoring

### 3.1 Overview

Monitor and copy trades from multiple whale addresses simultaneously instead of just one.

**Benefits:**
- Diversify across multiple successful traders
- Different scaling ratios per trader
- Per-trader statistics and comparison tools

### 3.2 Configuration Methods

**Method 1: Environment Variable**
```bash
TRADER_ADDRESSES=addr1,addr2,addr3
```

**Method 2: JSON Configuration File**
```json
// traders.json
[
  {
    "address": "abc123...",
    "label": "whale1",
    "scale_percent": 2.0,
    "min_shares": 10
  },
  {
    "address": "def456...",
    "label": "whale2",
    "scale_percent": 1.5,
    "min_shares": 20
  }
]
```

**Method 3: Legacy Single Address**
```bash
TARGET_WHALE_ADDRESS=abc123...  # Still works for backward compatibility
```

### 3.3 Per-Trader Features

- **Custom Labels:** Identify traders by name in logs
- **Individual Scaling:** Different position sizes per trader
- **Threshold Overrides:** Custom minimum trade sizes
- **Statistics Tracking:** Success rate, volume, fill rates per trader

### 3.4 Comparison Tool

```bash
cargo run --bin trader_comparison
```

Output shows:
- Copy rate (% of trades successfully copied)
- Success rate per trader
- Average fill rate
- Total USD copied

---

## 4. Trade Aggregation

### 4.1 Overview

Combines multiple rapid small trades into single orders for efficiency.

**Benefits:**
- Reduced API calls and fees
- Better execution for burst trading patterns
- Configurable aggregation windows

### 4.2 How It Works

1. Small trades within a time window are collected
2. Trades for the same token/side are combined
3. Weighted average price is calculated
4. Single aggregated order is executed

**Example:**
```
Trade 1: BUY 50 shares @ $0.45
Trade 2: BUY 30 shares @ $0.46  } Combined: BUY 100 shares @ $0.455 avg
Trade 3: BUY 20 shares @ $0.45
```

### 4.3 Configuration

```bash
AGG_ENABLED=true       # Enable aggregation (default: false)
AGG_WINDOW_MS=800      # Aggregation window in milliseconds
AGG_BYPASS_SHARES=4000 # Large trades bypass aggregation
```

### 4.4 Bypass Logic

Large trades (4000+ shares by default) execute immediately without waiting for aggregation, ensuring time-sensitive large orders aren't delayed.

---

## 5. Persistence & Analytics

### 5.1 SQLite Database

All trades are stored in `trades.db` for analysis and position tracking.

**Schema includes:**
- Trade details (token, side, price, shares)
- Execution status and timestamps
- Trader identification
- Aggregation metadata
- Transaction hashes

### 5.2 Position Monitoring

```bash
cargo run --bin position_monitor
```

**Features:**
- Current positions with net shares
- Average entry prices
- Trade counts per position
- Real-time updates

### 5.3 Trade History

```bash
cargo run --bin trade_history
```

**Filters:**
- `--trader <label>` - Filter by trader
- `--token <id>` - Filter by token
- `--since <timestamp>` - Filter by time
- `--status <status>` - Filter by execution status

**Live Data Enrichment:**
- `--refresh` - Fetch live market data from Polymarket APIs

When using `--refresh`, the tool fetches:
- Market titles and outcome names from Gamma API
- Current market prices for P&L calculation
- Displays enriched data with unrealized P&L per position

**Output formats:**
- Table (default)
- CSV (`--format csv`)
- JSON (`--format json`)

### 5.4 HTTP API

When enabled (`API_ENABLED=true`), exposes data via HTTP:

```bash
curl http://127.0.0.1:8080/health     # Bot status
curl http://127.0.0.1:8080/positions  # Current positions
curl http://127.0.0.1:8080/trades     # Recent trades
curl http://127.0.0.1:8080/stats      # Statistics
curl -X POST http://127.0.0.1:8080/reload  # Reload trader config (see Section 7)
```

### 5.5 CSV Import

Import historical trades from legacy CSV files:

```bash
cargo run --bin import_csv matches_optimized.csv --db trades.db
```

---

## 6. Live P&L Tracking

### 6.1 Overview

Real-time profit and loss calculation using live market prices.

### 6.2 Price Fetching

- Fetches current bid/ask from Polymarket CLOB API
- Caches prices with configurable TTL (default: 30 seconds)
- Rate limiting to avoid API throttling
- Graceful fallback to cached prices on errors

### 6.3 P&L Calculation

**Long Positions:**
```
Unrealized P&L = (bid_price - avg_entry_price) * shares
```

**Short Positions:**
```
Unrealized P&L = (avg_entry_price - ask_price) * abs(shares)
```

### 6.4 Portfolio Summary

```bash
cargo run --bin position_monitor
```

**Output includes:**
- Total Portfolio Value
- Total Cost Basis
- Total Unrealized P&L
- Daily P&L Change (since midnight UTC)
- Per-position P&L breakdown

### 6.5 Daily P&L Tracking

- Automatically snapshots portfolio at start of each day (UTC)
- Calculates change from daily starting point
- Persisted to `.portfolio_snapshot.json`

### 6.6 JSON Output

```bash
cargo run --bin position_monitor -- --json
```

Returns complete portfolio data in JSON format for automation and integration.

**Example output:**
```json
{
  "timestamp": "2026-01-21T10:30:00Z",
  "portfolio_value": 1234.56,
  "cost_basis": 1000.00,
  "unrealized_pnl": 234.56,
  "daily_pnl_change": 50.00,
  "snapshot_date": "2026-01-21",
  "position_count": 5,
  "positions": [...]
}
```

---

## 7. Hot Configuration Reload

### 7.1 Overview

Reload trader configuration without restarting the bot. This allows you to add/remove traders, change scaling ratios, or update thresholds on the fly.

### 7.2 Methods

**Method 1: SIGHUP Signal (Unix/Linux/macOS)**

Send the SIGHUP signal to the running bot process:

```bash
# Find the bot's process ID
pgrep -f pm_whale_follower

# Send SIGHUP to reload configuration
kill -HUP <pid>

# Or in one command
pkill -HUP -f pm_whale_follower
```

**Method 2: HTTP API Endpoint**

When `API_ENABLED=true`, use the `/reload` endpoint:

```bash
curl -X POST http://127.0.0.1:8080/reload
```

**Response:**
```json
{
  "success": true,
  "changed": true,
  "message": "Configuration reloaded successfully. Trader filter updated.",
  "trader_count": 3
}
```

### 7.3 What Gets Reloaded

- Trader addresses and labels
- Per-trader scaling ratios
- Per-trader minimum share thresholds
- WebSocket subscription filter (automatically updated)

### 7.4 What Does NOT Get Reloaded

- Environment variables (`.env` file)
- Circuit breaker settings
- API port/host settings

---

## 8. CLOB Trade History Tool

### 8.1 Overview

The `clob_history` tool provides complete trade history from the Polymarket CLOB API with accurate PnL calculations, activity tracking, and position reconciliation.

**Key Features:**
- Full trade history from CLOB API (authenticated)
- Activity tracking (trades, merges, redemptions)
- Realized and unrealized PnL calculations
- Position reconciliation with Position API
- Multiple output formats (table, JSON, CSV)

### 8.2 Basic Usage

```bash
# Show all positions with PnL (default view)
cargo run --bin clob_history

# Show individual trades
cargo run --bin clob_history -- --trades

# Filter by market title
cargo run --bin clob_history -- --title "Trump"

# Output as JSON
cargo run --bin clob_history -- --format json

# Sort by PnL instead of value
cargo run --bin clob_history -- --sort pnl
```

### 8.3 Reconciliation Mode

Compare CLOB trade data with Position API to find discrepancies:

```bash
cargo run --bin clob_history -- --reconcile
```

**Output columns:**
| Column | Description |
|--------|-------------|
| API | Current shares from Position API |
| Trades | Net shares from CLOB trades (buys - sells) |
| Merge | Shares added via token merging |
| Redeem | Shares removed via redemption |
| Unexplained | Shares that can't be explained |
| Status | ✓ (reconciled), ? (more than expected), ! (less than expected) |

**Example output:**
```
=== RECONCILIATION ===

  Shares from Trades:  2741.08
  Shares from Merges:  +6.73
  Shares Redeemed:     -1095.53

  ✓ All positions fully reconciled!
```

### 8.4 Activity Mode

View all wallet activities including trades, merges, and redemptions:

```bash
# Show all activities
cargo run --bin clob_history -- --activities

# Filter by activity type
cargo run --bin clob_history -- --activities --activity-type merge
cargo run --bin clob_history -- --activities --activity-type redeem
cargo run --bin clob_history -- --activities --activity-type trade

# Combine with title filter
cargo run --bin clob_history -- --activities --title "Chelsea"
```

**Activity Types:**
| Type | Description |
|------|-------------|
| TRADE | Regular buy/sell trades |
| MERGE | Token merging (combining Yes+No → USDC) |
| REDEEM | Market resolution redemptions |

### 8.5 All Options

```bash
cargo run --bin clob_history -- --help
```

| Option | Description |
|--------|-------------|
| `--trades` | Show individual trades instead of positions |
| `--activities` | Show all activities (TRADE, MERGE, REDEEM) |
| `--activity-type <TYPE>` | Filter activities by type (trade, merge, redeem) |
| `--reconcile` | Compare CLOB data with Position API |
| `--market <ID>` | Filter by market/condition ID |
| `--title <TEXT>` | Filter by title (case-insensitive) |
| `--format <FMT>` | Output format: table, json, csv |
| `--sort <FIELD>` | Sort by: pnl, value, shares, trades |
| `--limit <N>` | Limit number of results |
| `--unexplained-only` | Show only positions with unexplained shares |

### 8.6 PnL Calculation

**Realized PnL:**
```
Revenue from sells - Cost basis of sold shares
```

**Unrealized PnL:**
```
(Current price - Avg buy price) × Net shares held
```

**Total PnL:**
```
Realized PnL + Unrealized PnL
```

### 8.7 Data Sources

| Data | Source | Authentication |
|------|--------|----------------|
| Trade history | CLOB API (`/trades`) | L2 HMAC auth required |
| Current positions | Data API (`/positions`) | Public (no auth) |
| Activities | Data API (`/activity`) | Public (no auth) |

---

## 9. Auto-Claim (Redemption)

### 9.1 Overview

Automatically claim winning positions from resolved Polymarket markets. Uses the Polymarket Builder Relayer for gasless transactions.

**Requirements:**
- Builder credentials (apply at [Polymarket Builder Program](https://docs.polymarket.com/developers/builders))
- Set `POLY_BUILDER_API_KEY`, `POLY_BUILDER_SECRET`, `POLY_BUILDER_PASSPHRASE` in `.env`

### 9.2 Usage

```bash
# Dry run - show what would be redeemed
cargo run --bin auto_claim

# Actually execute redemptions
cargo run --bin auto_claim -- --execute

# Batch all redemptions into a single transaction (saves gas for Polymarket)
cargo run --bin auto_claim -- --execute --batch

# Wait for transaction confirmation
cargo run --bin auto_claim -- --execute --wait

# Only redeem positions worth at least $5
cargo run --bin auto_claim -- --execute --min-value 5

# Filter by market title
cargo run --bin auto_claim -- --title "Trump"
```

### 9.3 How It Works

1. Fetches all positions from Data API
2. Filters for `redeemable: true` positions with positive value
3. Builds redemption transactions for the CTF contract
4. Sends transactions through the Builder Relayer (gasless)
5. Optionally waits for on-chain confirmation

### 9.4 Options

| Option | Description |
|--------|-------------|
| `--execute` | Actually execute (default is dry run) |
| `--batch` | Combine all redemptions into one TX |
| `--wait` | Wait for transaction confirmation |
| `--min-value <USD>` | Minimum value to redeem (default: 0.01) |
| `--title <TEXT>` | Filter by market title |
| `--limit <N>` | Maximum positions to redeem |

### 9.5 Transaction Flow

```
Your Wallet → Builder Relayer → Safe Wallet → CTF Contract → USDC to Wallet
                    ↓
           (Polymarket pays gas)
```

---

## 10. Trading Flow (Step-by-Step)

This is a simplified overview. For complete detailed logic, see [Strategy Guide](05_STRATEGY.md).

1. **Detection:** Whale makes trade on Polymarket
2. **Event Received:** Bot receives blockchain event via WebSocket (<1 second latency)
3. **Parsing:** Bot extracts trade details (token, size, price, side)
4. **Filtering:** 
   - Check if trade is from target whale (skip if not)
   - Check if trade size is large enough (skip if too small, <10 shares)
5. **Risk Guard Check:** Multi-layer safety system checks:
   - Layer 1: Fast check (trade size, sequence detection)
   - Layer 2: Order book depth analysis (if triggered)
   - Layer 3: Trip status check
   - Result: Block trade if dangerous conditions detected
6. **Position Sizing:** Calculate your order size:
   - Base: 2% of whale's size
   - Apply tier multiplier (1.25x for 4000+, 1.0x otherwise)
   - Check minimum size ($1.01 requirement)
   - Probabilistic execution for very small positions
7. **Position Check (SELL orders only):**
   - For SELL orders, check if we hold shares of this token
   - If no position exists, skip with SKIPPED_NO_POSITION
   - Prevents "not enough balance" errors from exchange
8. **Price Calculation:** Determine limit price:
   - Get base buffer from tier (0.01 for large, 0.00 for small)
   - Add sport-specific buffers (tennis/soccer: +0.01)
   - Calculate: whale_price + total_buffer
   - Clamp to valid range (0.01-0.99)
9. **Order Type Selection:**
   - SELL orders: Always GTD
   - BUY orders: FAK initially, GTD on final retry
10. **Order Creation:** Create signed order with calculated parameters
11. **Submission:** Submit order to Polymarket API
12. **Result Handling:**
    - Success: Check fill amount, resubmit if partial
    - Failure: Enter resubmission loop (4-5 attempts)
    - Final attempt: Switch to GTD order if still not filled
13. **Logging:** Record all details to CSV and console with color-coded status

---

## 11. Performance Characteristics

**Latency:**
- Event detection: <1 second (blockchain dependent)
- Order processing: <100ms
- Total time to order: <2 seconds from whale trade

**Throughput:**
- Handles multiple concurrent trades
- Queued processing for high-frequency scenarios
- Automatic backpressure handling

**Reliability:**
- Automatic reconnection on WebSocket failures
- Retry logic for failed orders
- Circuit breakers prevent bad trades
- Error handling throughout

---

## 12. Limitations

**What the bot does NOT do:**
- ❌ Market analysis or prediction
- ❌ Stop-loss or take-profit orders
- ❌ Automatic exit strategies (you manage closing positions)
- ❌ Automatic rebalancing

**What you need to do manually:**
- Close positions when appropriate
- Adjust risk parameters
- Find good whales to copy

**What the bot NOW supports:**
- ✅ Position monitoring with live P&L (see Section 6)
- ✅ Portfolio summary and daily tracking
- ✅ Multiple whale copying (see Section 3)
- ✅ Complete trade history with reconciliation (see Section 8)

---

## 13. Safety Features Summary

✅ Scaled position sizes (2% default)
✅ Circuit breakers for dangerous conditions
✅ Minimum trade size filters
✅ Order book depth checks
✅ Automatic retry with limits
✅ Comprehensive error handling
✅ Mock trading mode for testing
✅ Extensive logging for audit
✅ SELL order position check (skips if no shares held)

---

## 14. Understanding the Output

**Console Messages:**

```
⚡ [B:12345] BUY_FILL | $100 | 200 OK | ...
```

- `[B:12345]`: Block number
- `BUY_FILL`: Trade direction and type
- `$100`: USD value of whale's trade
- `200 OK`: HTTP status (200 = success)
- Following numbers: Your fill details, prices, sizes

**Color Coding:**
- 🟢 Green: Successful fills (high percentage)
- 🟡 Yellow: Partial fills (medium percentage)
- 🔴 Red: Failed or low fills (low percentage)
- 🔵 Blue: Live market indicator

**CSV Format:**
All trades are logged with: timestamp, block, token_id, usd_value, shares, price, direction, status, order_book_data, tx_hash, is_live

---

## 15. Next Steps

- Read [Configuration Guide](03_CONFIGURATION.md) to adjust settings
- Review [Trading Strategy Guide](05_STRATEGY.md) for detailed strategy logic
- Check [Setup Guide](02_SETUP_GUIDE.md) if you haven't set up yet
- Review [Troubleshooting](06_TROUBLESHOOTING.md) if you have issues

