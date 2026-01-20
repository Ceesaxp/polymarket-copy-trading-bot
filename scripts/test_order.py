#!/usr/bin/env python3
"""
Minimal Python test to verify Polymarket order signing.
Uses py_clob_client to compare with our Rust implementation.
"""

import os
import json
from dotenv import load_dotenv

load_dotenv()

from py_clob_client.client import ClobClient
from py_clob_client.clob_types import OrderArgs, OrderType
from py_clob_client.constants import POLYGON

# Load credentials
PRIVATE_KEY = os.getenv("PRIVATE_KEY")
if not PRIVATE_KEY:
    print("ERROR: PRIVATE_KEY not set in .env")
    exit(1)

# Ensure 0x prefix
if not PRIVATE_KEY.startswith("0x"):
    PRIVATE_KEY = "0x" + PRIVATE_KEY

CLOB_API_BASE = "https://clob.polymarket.com"

# The token ID from the last failed order
TOKEN_ID = "79003893007240922565581139363959835619617307306268940540301817825959399270354"

def main():
    print("=" * 60)
    print("Python CLOB Client Test")
    print("=" * 60)

    # Create client
    print("\n1. Creating client...")
    client = ClobClient(
        host=CLOB_API_BASE,
        key=PRIVATE_KEY,
        chain_id=POLYGON,
    )

    # Show derived address
    print(f"   Wallet address: {client.get_address()}")

    # Get or create API credentials
    print("\n2. Getting API credentials...")
    try:
        creds = client.derive_api_key()
        print(f"   API Key: {creds.api_key[:20]}...")
        client.set_api_creds(creds)
    except Exception as e:
        print(f"   Error deriving API key: {e}")
        # Try loading from file
        if os.path.exists(".clob_creds.json"):
            print("   Trying to load from .clob_creds.json...")
            with open(".clob_creds.json") as f:
                creds_data = json.load(f)
            from py_clob_client.clob_types import ApiCreds
            creds = ApiCreds(
                api_key=creds_data["api_key"],
                api_secret=creds_data["api_secret"],
                api_passphrase=creds_data["api_passphrase"],
            )
            client.set_api_creds(creds)
            print(f"   Loaded API Key: {creds.api_key[:20]}...")
        else:
            raise

    # Create a minimal test order
    print("\n3. Creating test order...")
    print(f"   Token ID: {TOKEN_ID[:30]}...")
    print(f"   Side: BUY")
    print(f"   Price: 0.34")
    print(f"   Size: 3.0 (=$1.02)")

    order_args = OrderArgs(
        token_id=TOKEN_ID,
        price=0.34,
        size=3.0,
        side="BUY",
    )

    # Create signed order
    print("\n4. Signing order...")
    try:
        signed_order = client.create_order(order_args)
        print(f"   Order created successfully!")
        print(f"   Signature: {signed_order.signature[:40]}...")
        # Print all attributes
        print(f"   Order dict: {signed_order.dict()}")
    except Exception as e:
        print(f"   Error creating order: {e}")
        import traceback
        traceback.print_exc()
        raise

    # Submit the order
    print("\n5. Submitting order (FAK)...")
    try:
        response = client.post_order(signed_order, OrderType.FAK)
        print(f"   Response: {response}")
    except Exception as e:
        print(f"   Error submitting order: {e}")
        # Print full error details if available
        if hasattr(e, 'response'):
            print(f"   Status: {e.response.status_code}")
            print(f"   Body: {e.response.text}")

    print("\n" + "=" * 60)
    print("Done")
    print("=" * 60)

if __name__ == "__main__":
    main()
