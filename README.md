# Polymarket Copy Trading Bot

A high-performance Rust-based automated trading bot that copies trades from successful Polymarket traders (whales) in real-time.

## Table of Contents

1. [Quick Start Guide](#1-quick-start-guide-for-beginners)
2. [Documentation](#2-documentation)
3. [Requirements](#3-requirements)
4. [Security Notes](#4-security-notes)
5. [How It Works](#5-how-it-works)
6. [Features](#6-features)
7. [Advanced Usage](#7-advanced-usage)
8. [Output Files](#8-output-files)
9. [Getting Help](#9-getting-help)
10. [Disclaimer](#10-disclaimer)

## 1. Quick Start (For Beginners)

### 1.1 Step 1: Install Rust

**Windows:**
1. Download and run the installer from https://rustup.rs/
2. Follow the installation wizard
3. Restart your terminal/PowerShell

**macOS:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Linux:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 1.2 Step 2: Clone This Repository

```bash
git clone https://github.com/terauss/Polymarket-Copy-Trading-Bot.git
cd Polymarket-Copy-Trading-Bot
```

### 1.3 Step 3: Configure Your Settings

1. Copy the example environment file:
   ```bash
   # Windows (PowerShell)
   Copy-Item .env.example .env
   
   # macOS/Linux
   cp .env.example .env
   ```

2. Open `.env` in any text editor (Notepad, VS Code, etc.)

3. Fill in the required values (see [Configuration Guide](docs/03_CONFIGURATION.md) for details):
   - `PRIVATE_KEY` - Your wallet's private key (keep this SECRET!)
   - `FUNDER_ADDRESS` - Your wallet address (same wallet as private key)
   - `TARGET_WHALE_ADDRESS` - The whale address you want to copy (40-char hex, no 0x)
   - `ALCHEMY_API_KEY` - Get from https://www.alchemy.com/ (or use CHAINSTACK_API_KEY)

4. Optional: Adjust trading settings (see [Configuration Guide](docs/03_CONFIGURATION.md))

### 1.4 Step 4: Validate Your Configuration

Before running the bot, verify your setup is correct:

```bash
cargo run --release --bin validate_setup
```

This will check if all required settings are correct and provide helpful error messages if something is wrong.

### 1.5 Step 5: Test Mode (Recommended First)

Run in test mode to see what the bot would do without actually trading:

```bash
# Set MOCK_TRADING=true in your .env file, then:
cargo run --release
```

### 1.6 Step 6: Run the Bot

Once you're confident everything works:

```bash
# Enable trading in .env (ENABLE_TRADING=true, MOCK_TRADING=false)
cargo run --release
```

**Windows users:** You can also double-click `run.bat` after setting up your `.env` file.

## 2. Documentation

- **[01. Quick Start Guide](docs/01_QUICK_START.md)** - 5-minute setup guide
- **[02. Complete Setup Guide](docs/02_SETUP_GUIDE.md)** - Detailed step-by-step instructions
- **[03. Configuration Guide](docs/03_CONFIGURATION.md)** - All settings explained
- **[04. Features Overview](docs/04_FEATURES.md)** - What the bot does and how it works
- **[05. Trading Strategy](docs/05_STRATEGY.md)** - Complete strategy logic and decision-making
- **[06. Troubleshooting](docs/06_TROUBLESHOOTING.md)** - Common issues and solutions

## 3. Requirements

### 3.1 Required

1. **A Polymarket Account** - Sign up at https://polymarket.com
2. **A Web3 Wallet** - MetaMask recommended (with some USDC/USDC.e on Polygon)
3. **RPC Provider API Key** - Free tier from [Alchemy](https://www.alchemy.com/) or [Chainstack](https://chainstack.com/)
4. **The Whale Address** - The trader you want to copy (40-character hex address)

### 3.2 Recommended

- **Some Coding Knowledge** - Not required, but helpful for troubleshooting
- **Sufficient Funds** - The bot uses 2% of whale trade size by default (configurable)

## 4. Security Notes

⚠️ **IMPORTANT:**
- Never share your `PRIVATE_KEY` with anyone
- Never commit your `.env` file to git (it's already in `.gitignore`)
- Start with small amounts to test
- Use `MOCK_TRADING=true` first to verify everything works

## 5. How It Works

1. **Monitors** blockchain events for trades from your target whale (real-time via WebSocket)
2. **Analyzes** each trade (size, price, market conditions) using multi-layer risk checks
3. **Calculates** position size (2% default, with tier-based multipliers) and price (whale price + buffer)
4. **Executes** a scaled copy of the trade with optimized order types (FAK/GTD)
5. **Retries** failed orders with intelligent resubmission logic (up to 4-5 attempts)
6. **Protects** you with risk guards (circuit breakers) and safety features
7. **Logs** everything to CSV files for analysis

**Strategy Highlights:**
- **2% Position Scaling:** Reduces risk while maintaining meaningful positions
- **Tiered Execution:** Different strategies for large (4000+), medium (2000-3999), and small (<2000) trades
- **Multi-Layer Risk Management:** 4 layers of safety checks prevent dangerous trades
- **Intelligent Pricing:** Price buffers optimize fill rates (higher for large trades, none for small)
- **Sport-Specific Adjustments:** Additional buffers for tennis and soccer markets

See [Features Overview](docs/04_FEATURES.md) for feature details and [Strategy Guide](docs/05_STRATEGY.md) for complete trading logic.

## 6. Features

### Core Trading
- ✅ Real-time trade copying (<100ms latency)
- ✅ Intelligent position sizing (2% default, configurable)
- ✅ Circuit breakers for risk management
- ✅ Automatic order resubmission on failures
- ✅ Market cache system for fast lookups
- ✅ Live market detection
- ✅ Tiered execution based on trade size
- ✅ Trade aggregation (combines rapid small trades)

### Multi-Trader Support
- ✅ Monitor multiple whale addresses simultaneously
- ✅ Per-trader configuration (scaling ratios, thresholds)
- ✅ Per-trader statistics tracking
- ✅ Trader comparison tools
- ✅ Hot reload configuration (SIGHUP or API)

### Persistence & Analytics
- ✅ SQLite database for all trades
- ✅ CSV logging (backward compatible)
- ✅ Position monitoring with live P&L
- ✅ Portfolio summary statistics
- ✅ Daily P&L tracking
- ✅ HTTP API for data export
- ✅ JSON output for automation
- ✅ Complete CLOB trade history with PnL calculations
- ✅ Activity tracking (trades, merges, redemptions)
- ✅ Position reconciliation (CLOB vs API)

## 7. Advanced Usage

### 7.1 Running Different Modes

```bash
# Standard mode (monitors confirmed blocks)
cargo run --release

# Mempool mode (faster, but less reliable)
cargo run --release --bin mempool_monitor

# Monitor your own fills only (no trading)
cargo run --release --bin trade_monitor
```

### 7.2 Position & Analytics Tools

```bash
# Monitor positions with live P&L and portfolio summary
cargo run --release --bin position_monitor
cargo run --release --bin position_monitor -- --json          # JSON output
cargo run --release --bin position_monitor -- --no-prices     # Skip price fetching
cargo run --release --bin position_monitor -- --stats         # Show aggregation statistics

# Query trade history with filters
cargo run --release --bin trade_history
cargo run --release --bin trade_history -- --trader whale1    # Filter by trader
cargo run --release --bin trade_history -- --since 1704067200 # Since timestamp
cargo run --release --bin trade_history -- --format json      # JSON/CSV output
cargo run --release --bin trade_history -- --refresh          # Enrich with live market data

# Complete CLOB trade history with PnL and reconciliation
cargo run --release --bin clob_history                        # Show positions with PnL
cargo run --release --bin clob_history -- --trades            # Show individual trades
cargo run --release --bin clob_history -- --reconcile         # Compare CLOB vs Position API
cargo run --release --bin clob_history -- --activities        # Show all activities (TRADE/MERGE/REDEEM)
cargo run --release --bin clob_history -- --activities --activity-type merge  # Filter by type
cargo run --release --bin clob_history -- --format json       # JSON/CSV output

# Compare performance across traders
cargo run --release --bin trader_comparison
cargo run --release --bin trader_comparison -- --format csv   # CSV export
```

### 7.3 Utility Binaries

```bash
# Validate configuration before running
cargo run --release --bin validate_setup

# Check wallet balances (USDC, MATIC)
cargo run --release --bin check_balance

# Approve token allowances for trading
cargo run --release --bin approve_allowances

# Validate Polymarket profile/API credentials
cargo run --release --bin validate_profile

# Debug order signing (for troubleshooting)
cargo run --release --bin debug_signing

# Import legacy CSV data into SQLite database
cargo run --release --bin import_csv <csv_file> [--db <db_path>] [--dry-run]

# Auto-claim winning positions from resolved markets (requires Builder credentials)
cargo run --release --bin auto_claim                    # Dry run - show redeemable
cargo run --release --bin auto_claim -- --execute       # Actually redeem
cargo run --release --bin auto_claim -- --execute --batch  # Batch into single TX
cargo run --release --bin auto_claim -- --min-value 5   # Only redeem >= $5
```

### 7.4 Research Tools (Python)

Located in `research/` directory. Install with `pip install -r research/requirements.txt`.

```bash
# Fetch Polymarket leaderboard data
python research/fetch_leaderboard.py --top 50 --format csv

# Analyze trades from database
python research/analyze_trader.py --db trades.db

# Backtest copy trading strategy
python research/backtest_strategy.py --scale 0.5 --min-shares 10

# Interactive analysis (Jupyter notebook)
jupyter notebook research/notebooks/analysis.ipynb
```

### 7.5 Helper Scripts (Python)

Located in `scripts/` directory. Require Python 3 and dependencies from `scripts/pyproject.toml`.

```bash
# Transfer USDC to Polymarket proxy wallet
python scripts/transfer_usdc.py

# Build/refresh market caches
python scripts/build_live_cache.py
python scripts/build_sports_cache.py

# Monitor PNL divergence between you and whale
python scripts/realtime_divergence.py

# Test order signing (debugging)
python scripts/test_order.py
```

### 7.6 Building for Production

```bash
# Optimized release build
cargo build --release

# The binary will be at: target/release/pm_bot.exe (Windows)
#                        target/release/pm_bot (macOS/Linux)
```

## 8. Output Files

### Data Files
- `trades.db` - SQLite database with all trades (primary storage)
- `matches_optimized.csv` - CSV log of all trades (backward compatible)
- `.portfolio_snapshot.json` - Daily portfolio snapshot for P&L tracking

### Cache Files
- `.clob_creds.json` - Auto-generated API credentials (don't modify)
- `.clob_market_cache.json` - Market data cache (auto-updated)

### Configuration Files
- `.env` - Your configuration (from `.env.example`)
- `traders.json` - Multi-trader configuration (optional, see `traders.json.example`)

## 9. Getting Help

1. Check [Troubleshooting Guide](docs/06_TROUBLESHOOTING.md)
2. Run the config validator: `cargo run --release --bin validate_setup`
3. Review your `.env` file against `.env.example`
4. Check console output for error messages
5. Review [Strategy Guide](docs/05_STRATEGY.md) to understand bot logic

## 10. Disclaimer

This bot is provided as-is. Trading involves financial risk. Use at your own discretion. Test thoroughly before using real funds. The authors are not responsible for any losses.

## 📄 Contact
For questions or issues -- add an issue via GitHub



