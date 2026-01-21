#!/usr/bin/env python3
"""
Fetch Polymarket leaderboard data for trader discovery.

This script fetches top traders from Polymarket's leaderboard API
and outputs their addresses and statistics for potential copy trading.

Usage:
    python fetch_leaderboard.py --top 50
    python fetch_leaderboard.py --top 100 --output traders.csv
"""

import argparse
import csv
import json
import sys
from datetime import datetime

import httpx

# Polymarket leaderboard API endpoint
LEADERBOARD_API = "https://polymarket.com/api/leaderboard"


def fetch_leaderboard(limit: int = 50) -> list[dict]:
    """
    Fetch top traders from Polymarket leaderboard.

    Args:
        limit: Number of top traders to fetch

    Returns:
        List of trader dictionaries with address and stats
    """
    try:
        with httpx.Client(timeout=30) as client:
            # Try the leaderboard endpoint
            response = client.get(
                LEADERBOARD_API,
                params={"limit": limit, "period": "all"}
            )
            response.raise_for_status()
            data = response.json()

            traders = []
            for rank, trader in enumerate(data.get("leaderboard", [])[:limit], 1):
                traders.append({
                    "rank": rank,
                    "address": trader.get("address", ""),
                    "username": trader.get("username", ""),
                    "profit_loss": trader.get("profitLoss", 0),
                    "volume": trader.get("volume", 0),
                    "positions": trader.get("positions", 0),
                    "win_rate": trader.get("winRate", 0),
                })
            return traders

    except httpx.HTTPStatusError as e:
        print(f"Error fetching leaderboard: HTTP {e.response.status_code}", file=sys.stderr)
        print("Note: Polymarket may require authentication or have rate limits.", file=sys.stderr)
        return []
    except Exception as e:
        print(f"Error fetching leaderboard: {e}", file=sys.stderr)
        return []


def output_csv(traders: list[dict], file=sys.stdout):
    """Output traders as CSV."""
    if not traders:
        return

    writer = csv.DictWriter(file, fieldnames=traders[0].keys())
    writer.writeheader()
    writer.writerows(traders)


def output_json(traders: list[dict], file=sys.stdout):
    """Output traders as JSON."""
    json.dump(traders, file, indent=2)
    file.write("\n")


def output_table(traders: list[dict]):
    """Output traders as formatted table."""
    try:
        from tabulate import tabulate
        print(tabulate(traders, headers="keys", tablefmt="simple"))
    except ImportError:
        # Fallback to simple format
        if not traders:
            return
        headers = list(traders[0].keys())
        print(" | ".join(headers))
        print("-" * (len(" | ".join(headers))))
        for trader in traders:
            print(" | ".join(str(trader[h]) for h in headers))


def main():
    parser = argparse.ArgumentParser(
        description="Fetch Polymarket leaderboard for trader discovery"
    )
    parser.add_argument(
        "--top", "-n",
        type=int,
        default=50,
        help="Number of top traders to fetch (default: 50)"
    )
    parser.add_argument(
        "--output", "-o",
        type=str,
        help="Output file path (default: stdout)"
    )
    parser.add_argument(
        "--format", "-f",
        choices=["csv", "json", "table"],
        default="table",
        help="Output format (default: table)"
    )

    args = parser.parse_args()

    print(f"Fetching top {args.top} traders from Polymarket...", file=sys.stderr)
    traders = fetch_leaderboard(args.top)

    if not traders:
        print("No traders found. API may be unavailable or rate-limited.", file=sys.stderr)
        sys.exit(1)

    print(f"Found {len(traders)} traders", file=sys.stderr)

    # Output results
    output_file = open(args.output, "w") if args.output else sys.stdout

    try:
        if args.format == "csv":
            output_csv(traders, output_file)
        elif args.format == "json":
            output_json(traders, output_file)
        else:
            output_table(traders)
    finally:
        if args.output:
            output_file.close()
            print(f"Output written to {args.output}", file=sys.stderr)


if __name__ == "__main__":
    main()
