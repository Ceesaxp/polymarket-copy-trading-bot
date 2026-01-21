# Polymarket Copy Trading Research Tools

Python scripts for trader discovery, analysis, and strategy backtesting.

## Setup

```bash
cd research
python -m venv venv
source venv/bin/activate  # On Windows: venv\Scripts\activate
pip install -r requirements.txt
```

## Scripts

### 1. Fetch Leaderboard (`fetch_leaderboard.py`)

Discover top traders from Polymarket's leaderboard.

```bash
# Fetch top 50 traders (default)
python fetch_leaderboard.py

# Fetch top 100 as CSV
python fetch_leaderboard.py --top 100 --format csv > traders.csv

# Output as JSON
python fetch_leaderboard.py --format json
```

**Note:** The leaderboard API may require authentication or have rate limits.

### 2. Analyze Trader (`analyze_trader.py`)

Analyze trading patterns from the local SQLite database.

```bash
# Analyze all trades in database
python analyze_trader.py --db ../trades.db

# Analyze specific trader
python analyze_trader.py LEGACY_CSV_IMPORT --db ../trades.db

# Last 30 days only
python analyze_trader.py --days 30

# Detailed output with status breakdown
python analyze_trader.py -v

# Use HTTP API instead of database
python analyze_trader.py --api http://127.0.0.1:8080
```

### 3. Backtest Strategy (`backtest_strategy.py`)

Simulate copy trading strategies on historical data.

```bash
# Basic backtest with default parameters
python backtest_strategy.py --db ../trades.db

# Scale down trades to 50%
python backtest_strategy.py --scale 0.5

# Skip trades smaller than 20 shares
python backtest_strategy.py --min-shares 20

# Limit max position to $500
python backtest_strategy.py --max-position 500

# Random 50% copy probability
python backtest_strategy.py --probability 0.5

# Combine parameters
python backtest_strategy.py --scale 0.3 --min-shares 15 --max-position 200 -v
```

## Jupyter Notebook

For interactive analysis, use the included notebook:

```bash
jupyter notebook notebooks/analysis.ipynb
```

## Data Sources

The scripts can consume data from:

1. **SQLite Database** (default)
   - Path: `../trades.db`
   - Contains all trades recorded by the bot

2. **HTTP API** (when bot is running with API enabled)
   - Endpoint: `http://127.0.0.1:8080`
   - Requires `API_ENABLED=true` in bot's `.env`

## Typical Workflow

1. **Discover traders** using the leaderboard:
   ```bash
   python fetch_leaderboard.py --top 100 --format csv > candidates.csv
   ```

2. **Import historical data** (run from project root):
   ```bash
   cargo run --bin import_csv -- matches_optimized.csv
   ```

3. **Analyze performance**:
   ```bash
   python analyze_trader.py -v
   ```

4. **Backtest strategies**:
   ```bash
   python backtest_strategy.py --scale 0.5 --min-shares 10 -v
   ```

5. **Configure bot** with discovered trader addresses in `traders.json`

## Output Formats

All scripts support multiple output formats:

- `--format text` - Human-readable tables (default)
- `--format json` - Machine-readable JSON
- `--format csv` - CSV for spreadsheets (where applicable)
