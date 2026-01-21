# Trade History CLI Tool - Usage Guide

The `trade_history` binary provides a powerful interface for querying and analyzing trade data from the SQLite database.

## Installation

Build the binary:
```bash
cargo build --release --bin trade_history
```

## Basic Usage

### View Recent Trades (Default)
```bash
./target/release/trade_history
```

This displays the 50 most recent trades in table format with summary statistics.

### Custom Database Path
```bash
./target/release/trade_history --db /path/to/custom.db
```

## Filtering Options

### Filter by Trader Address
Show only trades from a specific whale address:
```bash
./target/release/trade_history --trader 0x1234567890abcdef
```

Supports partial address matching for convenience:
```bash
./target/release/trade_history --trader 1234
```

### Filter by Token ID
Show trades for a specific Polymarket token:
```bash
./target/release/trade_history --token 0xabc...
```

### Filter by Trade Status
Show only trades with a specific outcome:
```bash
./target/release/trade_history --status SUCCESS
./target/release/trade_history --status FAILED
./target/release/trade_history --status PARTIAL
./target/release/trade_history --status SKIPPED
```

Status filtering is case-insensitive.

### Filter by Time Range
Show trades since a specific Unix timestamp (in seconds):
```bash
# Show trades from the last hour
./target/release/trade_history --since $(date -v-1H +%s)

# Show trades from a specific date
./target/release/trade_history --since 1704067200
```

### Combine Multiple Filters
Filters can be combined for precise queries:
```bash
./target/release/trade_history \
  --trader 0x1234 \
  --status SUCCESS \
  --since 1704067200
```

## Output Formats

### Table Format (Default)
Human-readable table with columns for timestamp, side, token, USD values, fill %, and status:
```bash
./target/release/trade_history --format table
```

Example output:
```
=== TRADE HISTORY ===

Timestamp            Side     Token           Whale $       Our $    Fill %   Status
----------------------------------------------------------------------------------------------------
2024-01-01 00:00:00  BUY      token12345...     $450.00      $9.20     100.0%   SUCCESS
2024-01-01 00:01:00  SELL     token67890...     $275.00      $5.40     100.0%   SUCCESS

Total trades: 2

=== SUMMARY STATISTICS ===

Status breakdown:
  SUCCESS: 2 (100.0%)

Side breakdown:
  BUY:  1
  SELL: 1

Volume:
  Whale total: $725.00
  Our total:   $14.60
  Scaling:     2.01%

Latency (ms):
  Average: 62.5
  Min:     50
  Max:     75

Average fill: 100.0%
```

### CSV Format
Export data as CSV for analysis in spreadsheet applications:
```bash
./target/release/trade_history --format csv > trades_export.csv
```

CSV includes all fields: timestamp, side, token_id, trader_address, whale metrics, our metrics, fill %, status, latency, and tx_hash.

### JSON Format
Export as JSON for programmatic processing:
```bash
./target/release/trade_history --format json > trades_export.json
```

JSON output is pretty-printed and includes all trade fields with proper typing.

## Pagination

Control the number of results displayed:
```bash
# Show last 10 trades
./target/release/trade_history --limit 10

# Show last 100 trades
./target/release/trade_history --limit 100

# Show all trades (use a very large limit)
./target/release/trade_history --limit 999999
```

Default limit is 50 trades.

## Summary Statistics

When using table format, summary statistics are automatically displayed including:

- **Status breakdown**: Count and percentage of SUCCESS/FAILED/PARTIAL/SKIPPED trades
- **Side breakdown**: Number of BUY vs SELL trades
- **Volume metrics**: Total USD traded by whale and our execution, with scaling ratio
- **Latency statistics**: Average, minimum, and maximum execution latency in milliseconds
- **Fill rate**: Average fill percentage across successful trades

Statistics are calculated only for the displayed/filtered trades.

## Common Use Cases

### Daily Trading Report
```bash
# Show all successful trades from today
./target/release/trade_history \
  --since $(date -v0H -v0M -v0S +%s) \
  --status SUCCESS
```

### Trader Performance Comparison
```bash
# Export all trades for a specific trader as CSV
./target/release/trade_history \
  --trader 0xYourWhaleAddress \
  --format csv > trader_analysis.csv
```

### Failed Trade Investigation
```bash
# List all failed trades for debugging
./target/release/trade_history \
  --status FAILED \
  --limit 20
```

### Token-Specific History
```bash
# Analyze trading history for a specific market
./target/release/trade_history \
  --token 0xTokenID \
  --format json > token_history.json
```

### Quick Status Check
```bash
# View the 10 most recent trades
./target/release/trade_history --limit 10
```

## Integration with Other Tools

### Pipe to jq for JSON Analysis
```bash
# Extract just the successful trades
./target/release/trade_history --format json | \
  jq '[.[] | select(.status == "SUCCESS")]'

# Calculate total volume
./target/release/trade_history --format json | \
  jq '[.[] | .our_usd // 0] | add'
```

### Import CSV into Analysis Tools
```bash
# Export for Excel/Google Sheets
./target/release/trade_history --format csv --limit 1000 > trades.csv

# Import into Python pandas
# df = pd.read_csv('trades.csv')
```

## Performance Notes

- Database queries are fast even with thousands of trades (SQLite with WAL mode)
- Filters are applied in-memory after fetching (use --limit to reduce memory usage)
- Table format includes automatic column truncation for readability
- JSON output includes both human-readable timestamps and raw millisecond values

## Error Handling

If the database doesn't exist:
```
Error: Failed to open SQLite database
```
→ Check the --db path or ensure trades.db exists

If no trades match filters:
```
No trades found.
```
→ Adjust your filter criteria or check if the database has any data

## See Also

- `position_monitor` - View current aggregated positions with P&L
- Main bot logs - Real-time trade execution monitoring
