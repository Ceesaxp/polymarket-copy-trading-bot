#!/usr/bin/env python3
"""
Analyze a trader's historical performance from the local database.

This script analyzes trades for a specific trader address from the
SQLite database, providing statistics and visualizations.

Usage:
    python analyze_trader.py LEGACY_CSV_IMPORT --days 30
    python analyze_trader.py LEGACY_CSV_IMPORT --db ../trades.db
    python analyze_trader.py --api http://127.0.0.1:8080
"""

import argparse
import json
import sqlite3
import sys
from datetime import datetime, timedelta
from pathlib import Path

try:
    import pandas as pd
    import numpy as np
except ImportError:
    print("Please install pandas and numpy: pip install pandas numpy", file=sys.stderr)
    sys.exit(1)


def load_trades_from_db(db_path: str, trader_address: str = None, days: int = None) -> pd.DataFrame:
    """Load trades from SQLite database."""
    conn = sqlite3.connect(db_path)

    query = "SELECT * FROM trades"
    conditions = []
    params = []

    if trader_address:
        conditions.append("trader_address = ?")
        params.append(trader_address)

    if days:
        since_ts = int((datetime.now() - timedelta(days=days)).timestamp() * 1000)
        conditions.append("timestamp_ms >= ?")
        params.append(since_ts)

    if conditions:
        query += " WHERE " + " AND ".join(conditions)

    query += " ORDER BY timestamp_ms DESC"

    df = pd.read_sql_query(query, conn, params=params)
    conn.close()
    return df


def load_trades_from_api(api_url: str, limit: int = 1000) -> pd.DataFrame:
    """Load trades from HTTP API."""
    import httpx

    response = httpx.get(f"{api_url}/trades", params={"limit": limit})
    response.raise_for_status()
    trades = response.json()
    return pd.DataFrame(trades)


def analyze_trades(df: pd.DataFrame) -> dict:
    """Analyze trade data and return statistics."""
    if df.empty:
        return {"error": "No trades found"}

    # Convert timestamp to datetime
    df["datetime"] = pd.to_datetime(df["timestamp_ms"], unit="ms")

    # Basic stats
    total_trades = len(df)
    unique_tokens = df["token_id"].nunique()

    # Side distribution
    buy_trades = len(df[df["side"] == "BUY"])
    sell_trades = len(df[df["side"] == "SELL"])

    # Volume stats (using whale data as that's what we observe)
    total_volume_usd = df["whale_usd"].sum()
    avg_trade_size = df["whale_usd"].mean()

    # Status distribution
    status_counts = df["status"].value_counts().to_dict()

    # Execution stats (trades where our_shares is not null)
    executed_df = df[df["our_shares"].notna()]
    executed_count = len(executed_df)
    execution_rate = (executed_count / total_trades * 100) if total_trades > 0 else 0

    # Time analysis
    if not df.empty:
        first_trade = df["datetime"].min()
        last_trade = df["datetime"].max()
        trading_days = (last_trade - first_trade).days + 1
        trades_per_day = total_trades / trading_days if trading_days > 0 else 0
    else:
        first_trade = last_trade = None
        trading_days = trades_per_day = 0

    return {
        "total_trades": total_trades,
        "unique_tokens": unique_tokens,
        "buy_trades": buy_trades,
        "sell_trades": sell_trades,
        "buy_sell_ratio": buy_trades / sell_trades if sell_trades > 0 else float("inf"),
        "total_volume_usd": round(total_volume_usd, 2),
        "avg_trade_size_usd": round(avg_trade_size, 2),
        "executed_trades": executed_count,
        "execution_rate_pct": round(execution_rate, 2),
        "status_breakdown": status_counts,
        "first_trade": str(first_trade) if first_trade else None,
        "last_trade": str(last_trade) if last_trade else None,
        "trading_days": trading_days,
        "trades_per_day": round(trades_per_day, 2),
    }


def print_analysis(stats: dict, verbose: bool = False):
    """Print analysis results in a readable format."""
    if "error" in stats:
        print(f"Error: {stats['error']}")
        return

    print("\n" + "=" * 50)
    print("TRADER ANALYSIS")
    print("=" * 50)

    print(f"\nTrading Activity:")
    print(f"  Total Trades:     {stats['total_trades']:,}")
    print(f"  Unique Tokens:    {stats['unique_tokens']:,}")
    print(f"  Trading Days:     {stats['trading_days']}")
    print(f"  Trades/Day:       {stats['trades_per_day']:.1f}")

    print(f"\nTrade Distribution:")
    print(f"  Buy Trades:       {stats['buy_trades']:,}")
    print(f"  Sell Trades:      {stats['sell_trades']:,}")
    print(f"  Buy/Sell Ratio:   {stats['buy_sell_ratio']:.2f}")

    print(f"\nVolume:")
    print(f"  Total USD:        ${stats['total_volume_usd']:,.2f}")
    print(f"  Avg Trade Size:   ${stats['avg_trade_size_usd']:.2f}")

    print(f"\nExecution:")
    print(f"  Executed Trades:  {stats['executed_trades']:,}")
    print(f"  Execution Rate:   {stats['execution_rate_pct']:.1f}%")

    print(f"\nTime Range:")
    print(f"  First Trade:      {stats['first_trade']}")
    print(f"  Last Trade:       {stats['last_trade']}")

    if verbose and stats.get("status_breakdown"):
        print(f"\nStatus Breakdown:")
        for status, count in sorted(stats["status_breakdown"].items(), key=lambda x: -x[1])[:10]:
            pct = count / stats['total_trades'] * 100
            # Truncate long status strings
            status_short = status[:40] + "..." if len(status) > 40 else status
            print(f"  {status_short}: {count} ({pct:.1f}%)")


def main():
    parser = argparse.ArgumentParser(
        description="Analyze trader performance from local database"
    )
    parser.add_argument(
        "trader",
        nargs="?",
        default=None,
        help="Trader address to analyze (optional, analyzes all if not specified)"
    )
    parser.add_argument(
        "--db",
        type=str,
        default="../trades.db",
        help="Path to SQLite database (default: ../trades.db)"
    )
    parser.add_argument(
        "--api",
        type=str,
        help="Use HTTP API instead of database (e.g., http://127.0.0.1:8080)"
    )
    parser.add_argument(
        "--days", "-d",
        type=int,
        help="Analyze only last N days"
    )
    parser.add_argument(
        "--format", "-f",
        choices=["text", "json"],
        default="text",
        help="Output format (default: text)"
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Show detailed status breakdown"
    )

    args = parser.parse_args()

    # Load trades
    if args.api:
        print(f"Loading trades from API: {args.api}", file=sys.stderr)
        df = load_trades_from_api(args.api)
        if args.trader:
            df = df[df["trader_address"] == args.trader]
    else:
        db_path = Path(args.db)
        if not db_path.exists():
            print(f"Database not found: {db_path}", file=sys.stderr)
            print("Run the bot first or specify correct path with --db", file=sys.stderr)
            sys.exit(1)

        print(f"Loading trades from: {db_path}", file=sys.stderr)
        df = load_trades_from_db(str(db_path), args.trader, args.days)

    print(f"Loaded {len(df)} trades", file=sys.stderr)

    # Analyze
    stats = analyze_trades(df)

    # Output
    if args.format == "json":
        print(json.dumps(stats, indent=2))
    else:
        print_analysis(stats, args.verbose)


if __name__ == "__main__":
    main()
