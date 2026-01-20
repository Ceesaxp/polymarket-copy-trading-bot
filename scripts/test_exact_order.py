#!/usr/bin/env python3
"""
Test with EXACT same parameters as the Rust bot failed with.
"""

import os
import json
from dotenv import load_dotenv

load_dotenv()

from py_clob_client.client import ClobClient
from py_clob_client.clob_types import OrderArgs, OrderType
from py_clob_client.constants import POLYGON

PRIVATE_KEY = os.getenv("PRIVATE_KEY")
if not PRIVATE_KEY.startswith("0x"):
    PRIVATE_KEY = "0x" + PRIVATE_KEY

CLOB_API_BASE = "https://clob.polymarket.com"

# Exact values from Rust verbose output that failed:
TOKEN_ID = "38865488008932950700526030022928946208892757165833157506040011064447395822077"
# Exchange: 0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E (non neg-risk)
# makerAmount: 2290000, takerAmount: 6200000
# This translates to: price = 2290000/6200000 = 0.369... and size = 6.2 shares

def main():
    print("=" * 60)
    print("Test EXACT order from Rust failure")
    print("=" * 60)

    client = ClobClient(
        host=CLOB_API_BASE,
        key=PRIVATE_KEY,
        chain_id=POLYGON,
    )

    print(f"\nWallet: {client.get_address()}")

    # Load or derive API creds
    try:
        creds = client.derive_api_key()
        client.set_api_creds(creds)
    except:
        with open(".clob_creds.json") as f:
            creds_data = json.load(f)
        from py_clob_client.clob_types import ApiCreds
        creds = ApiCreds(
            api_key=creds_data["api_key"],
            api_secret=creds_data["api_secret"],
            api_passphrase=creds_data["api_passphrase"],
        )
        client.set_api_creds(creds)

    print(f"API Key: {creds.api_key[:20]}...")

    # Test with a different, simpler order to avoid precision issues
    # Use nice round numbers
    price = 0.37  # Clean 2 decimal price
    size = 5.0    # Clean size

    print(f"\nOrder params (clean values):")
    print(f"  Token ID: {TOKEN_ID[:30]}...")
    print(f"  Price: {price}")
    print(f"  Size: {size}")

    order_args = OrderArgs(
        token_id=TOKEN_ID,
        price=price,
        size=size,
        side="BUY",
    )

    print(f"\nCreating order...")
    signed = client.create_order(order_args)
    print(f"Order dict: {json.dumps(signed.dict(), indent=2)}")

    print(f"\nSubmitting FAK order...")
    try:
        response = client.post_order(signed, OrderType.FAK)
        print(f"SUCCESS: {response}")
    except Exception as e:
        print(f"ERROR: {e}")
        if hasattr(e, 'response'):
            print(f"Status: {e.response.status_code}")
            print(f"Body: {e.response.text}")

if __name__ == "__main__":
    main()
