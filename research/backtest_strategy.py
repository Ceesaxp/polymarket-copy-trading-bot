#!/usr/bin/env python3
"""
Backtest copy trading strategies on historical data.

This script simulates copy trading strategies using historical trade data
from the SQLite database to evaluate potential performance.

Usage:
    python backtest_strategy.py --db ../trades.db
    python backtest_strategy.py --scale 0.5 --min-shares 10
"""

import argparse
import json
import sqlite3
import sys
from datetime import datetime
from pathlib import Path

try:
    import pandas as pd
    import numpy as np
except ImportError:
    print("Please install pandas and numpy: pip install pandas numpy", file=sys.stderr)
    sys.exit(1)


class BacktestEngine:
    """Simple backtesting engine for copy trading strategies."""

    def __init__(
        self,
        scale_ratio: float = 1.0,
        min_shares: float = 10.0,
        max_position_usd: float = 1000.0,
        copy_probability: float = 1.0,
    ):
        self.scale_ratio = scale_ratio
        self.min_shares = min_shares
        self.max_position_usd = max_position_usd
        self.copy_probability = copy_probability

        # State
        self.positions: dict[str, float] = {}  # token_id -> net_shares
        self.trades_executed = 0
        self.trades_skipped = 0
        self.total_volume_usd = 0.0
        self.entry_prices: dict[str, list] = {}  # token_id -> [(shares, price), ...]

    def should_copy(self, whale_shares: float, whale_usd: float) -> tuple[bool, str]:
        """Determine if a trade should be copied."""
        # Scale the trade
        our_shares = whale_shares * self.scale_ratio

        # Check minimum shares
        if our_shares < self.min_shares:
            return False, f"SKIP: {our_shares:.2f} shares < {self.min_shares} minimum"

        # Check max position
        if whale_usd * self.scale_ratio > self.max_position_usd:
            return False, f"SKIP: ${whale_usd * self.scale_ratio:.2f} > ${self.max_position_usd} max"

        # Random probability check
        if np.random.random() > self.copy_probability:
            return False, f"SKIP: Random probability ({self.copy_probability*100:.0f}%)"

        return True, "COPY"

    def execute_trade(self, token_id: str, side: str, shares: float, price: float, usd: float):
        """Execute a simulated trade."""
        scaled_shares = shares * self.scale_ratio
        scaled_usd = usd * self.scale_ratio

        if side == "BUY":
            self.positions[token_id] = self.positions.get(token_id, 0) + scaled_shares
            # Track entry price
            if token_id not in self.entry_prices:
                self.entry_prices[token_id] = []
            self.entry_prices[token_id].append((scaled_shares, price))
        else:  # SELL
            self.positions[token_id] = self.positions.get(token_id, 0) - scaled_shares

        self.trades_executed += 1
        self.total_volume_usd += scaled_usd

    def run_backtest(self, trades_df: pd.DataFrame) -> dict:
        """Run backtest on historical trades."""
        # Sort by timestamp
        trades_df = trades_df.sort_values("timestamp_ms")

        execution_log = []

        for _, row in trades_df.iterrows():
            whale_shares = row["whale_shares"]
            whale_usd = row["whale_usd"]
            whale_price = row["whale_price"]
            token_id = row["token_id"]
            side = row["side"]

            should_copy, reason = self.should_copy(whale_shares, whale_usd)

            if should_copy:
                self.execute_trade(token_id, side, whale_shares, whale_price, whale_usd)
                execution_log.append({
                    "timestamp_ms": row["timestamp_ms"],
                    "token_id": token_id[:20] + "...",
                    "side": side,
                    "whale_shares": whale_shares,
                    "our_shares": whale_shares * self.scale_ratio,
                    "price": whale_price,
                    "action": "EXECUTED"
                })
            else:
                self.trades_skipped += 1
                execution_log.append({
                    "timestamp_ms": row["timestamp_ms"],
                    "token_id": token_id[:20] + "...",
                    "side": side,
                    "whale_shares": whale_shares,
                    "our_shares": 0,
                    "price": whale_price,
                    "action": reason
                })

        return {
            "total_whale_trades": len(trades_df),
            "trades_executed": self.trades_executed,
            "trades_skipped": self.trades_skipped,
            "execution_rate": self.trades_executed / len(trades_df) * 100 if len(trades_df) > 0 else 0,
            "total_volume_usd": round(self.total_volume_usd, 2),
            "open_positions": len([p for p in self.positions.values() if abs(p) > 0.0001]),
            "positions": {k: round(v, 4) for k, v in self.positions.items() if abs(v) > 0.0001},
            "strategy_params": {
                "scale_ratio": self.scale_ratio,
                "min_shares": self.min_shares,
                "max_position_usd": self.max_position_usd,
                "copy_probability": self.copy_probability,
            },
            "execution_log": execution_log[-20:],  # Last 20 trades
        }


def load_trades(db_path: str, days: int = None) -> pd.DataFrame:
    """Load trades from SQLite database."""
    conn = sqlite3.connect(db_path)

    query = "SELECT * FROM trades"
    params = []

    if days:
        from datetime import timedelta
        since_ts = int((datetime.now() - timedelta(days=days)).timestamp() * 1000)
        query += " WHERE timestamp_ms >= ?"
        params.append(since_ts)

    query += " ORDER BY timestamp_ms ASC"

    df = pd.read_sql_query(query, conn, params=params)
    conn.close()
    return df


def print_results(results: dict, verbose: bool = False):
    """Print backtest results."""
    print("\n" + "=" * 60)
    print("BACKTEST RESULTS")
    print("=" * 60)

    params = results["strategy_params"]
    print(f"\nStrategy Parameters:")
    print(f"  Scale Ratio:      {params['scale_ratio']:.2f}x")
    print(f"  Min Shares:       {params['min_shares']}")
    print(f"  Max Position:     ${params['max_position_usd']}")
    print(f"  Copy Probability: {params['copy_probability']*100:.0f}%")

    print(f"\nExecution Stats:")
    print(f"  Whale Trades:     {results['total_whale_trades']:,}")
    print(f"  Executed:         {results['trades_executed']:,}")
    print(f"  Skipped:          {results['trades_skipped']:,}")
    print(f"  Execution Rate:   {results['execution_rate']:.1f}%")

    print(f"\nVolume:")
    print(f"  Total USD:        ${results['total_volume_usd']:,.2f}")

    print(f"\nPositions:")
    print(f"  Open Positions:   {results['open_positions']}")

    if verbose and results.get("positions"):
        print(f"\n  Position Details:")
        for token_id, shares in list(results["positions"].items())[:10]:
            token_short = token_id[:20] + "..."
            print(f"    {token_short}: {shares:,.4f} shares")

    if verbose and results.get("execution_log"):
        print(f"\n  Recent Trades (last 10):")
        for trade in results["execution_log"][-10:]:
            action = trade["action"]
            if action == "EXECUTED":
                print(f"    [{trade['side']}] {trade['our_shares']:.2f} @ {trade['price']:.4f}")
            else:
                print(f"    [{trade['side']}] {action}")


def main():
    parser = argparse.ArgumentParser(
        description="Backtest copy trading strategies on historical data"
    )
    parser.add_argument(
        "--db",
        type=str,
        default="../trades.db",
        help="Path to SQLite database (default: ../trades.db)"
    )
    parser.add_argument(
        "--days", "-d",
        type=int,
        help="Backtest only last N days"
    )
    parser.add_argument(
        "--scale", "-s",
        type=float,
        default=1.0,
        help="Scale ratio for trade sizes (default: 1.0)"
    )
    parser.add_argument(
        "--min-shares",
        type=float,
        default=10.0,
        help="Minimum shares to copy a trade (default: 10)"
    )
    parser.add_argument(
        "--max-position",
        type=float,
        default=1000.0,
        help="Maximum position size in USD (default: 1000)"
    )
    parser.add_argument(
        "--probability", "-p",
        type=float,
        default=1.0,
        help="Probability of copying each trade (default: 1.0 = 100%%)"
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
        help="Show detailed position and trade info"
    )

    args = parser.parse_args()

    # Load trades
    db_path = Path(args.db)
    if not db_path.exists():
        print(f"Database not found: {db_path}", file=sys.stderr)
        print("Run the bot first or import CSV data.", file=sys.stderr)
        sys.exit(1)

    print(f"Loading trades from: {db_path}", file=sys.stderr)
    df = load_trades(str(db_path), args.days)
    print(f"Loaded {len(df)} trades", file=sys.stderr)

    if df.empty:
        print("No trades found for backtest.", file=sys.stderr)
        sys.exit(1)

    # Run backtest
    engine = BacktestEngine(
        scale_ratio=args.scale,
        min_shares=args.min_shares,
        max_position_usd=args.max_position,
        copy_probability=args.probability,
    )

    print("Running backtest...", file=sys.stderr)
    results = engine.run_backtest(df)

    # Output
    if args.format == "json":
        # Remove execution_log for cleaner JSON output unless verbose
        if not args.verbose:
            results.pop("execution_log", None)
        print(json.dumps(results, indent=2))
    else:
        print_results(results, args.verbose)


if __name__ == "__main__":
    main()
