#!/usr/bin/env python3
"""
Transfer USDC.e from EOA wallet to Polymarket proxy wallet.
"""

import os
import sys
from dotenv import load_dotenv
from eth_account import Account
from web3 import Web3

load_dotenv()

# Polygon RPCs (fallbacks)
POLYGON_RPCS = [
    "https://polygon.llamarpc.com",
    "https://polygon-bor-rpc.publicnode.com",
    "https://polygon-rpc.com",
]

# USDC.e contract on Polygon
USDC_ADDRESS = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174"

# ERC20 ABI (minimal - just what we need)
ERC20_ABI = [
    {
        "constant": True,
        "inputs": [{"name": "_owner", "type": "address"}],
        "name": "balanceOf",
        "outputs": [{"name": "balance", "type": "uint256"}],
        "type": "function"
    },
    {
        "constant": False,
        "inputs": [
            {"name": "_to", "type": "address"},
            {"name": "_value", "type": "uint256"}
        ],
        "name": "transfer",
        "outputs": [{"name": "", "type": "bool"}],
        "type": "function"
    },
    {
        "constant": True,
        "inputs": [],
        "name": "decimals",
        "outputs": [{"name": "", "type": "uint8"}],
        "type": "function"
    }
]

def main():
    # Get private key
    private_key = os.getenv("PRIVATE_KEY")
    if not private_key:
        print("ERROR: PRIVATE_KEY not set in .env")
        sys.exit(1)

    if not private_key.startswith("0x"):
        private_key = "0x" + private_key

    # Polymarket proxy wallet (destination)
    PROXY_WALLET = "0xec1813ed7b60a9b5880e546b444cbfed51428eb4"

    # Connect to Polygon (try multiple RPCs)
    w3 = None
    for rpc in POLYGON_RPCS:
        try:
            w3 = Web3(Web3.HTTPProvider(rpc, request_kwargs={'timeout': 30}))
            if w3.is_connected():
                print(f"Connected to: {rpc}")
                break
        except Exception:
            continue

    if not w3 or not w3.is_connected():
        print("ERROR: Cannot connect to any Polygon RPC")
        sys.exit(1)

    print("=" * 60)
    print("USDC.e Transfer Tool")
    print("=" * 60)

    # Get account from private key
    account = Account.from_key(private_key)
    eoa_address = account.address

    print(f"\nFrom (EOA):  {eoa_address}")
    print(f"To (Proxy):  {PROXY_WALLET}")

    # Get USDC contract
    usdc = w3.eth.contract(address=Web3.to_checksum_address(USDC_ADDRESS), abi=ERC20_ABI)

    # Check balances
    print("\n" + "-" * 60)
    print("Balances:")
    print("-" * 60)

    # USDC balance
    usdc_balance = usdc.functions.balanceOf(eoa_address).call()
    usdc_human = usdc_balance / 1_000_000  # USDC has 6 decimals
    print(f"  USDC.e balance: ${usdc_human:.2f} ({usdc_balance} raw)")

    # MATIC balance (for gas)
    matic_balance = w3.eth.get_balance(eoa_address)
    matic_human = w3.from_wei(matic_balance, 'ether')
    print(f"  MATIC balance:  {matic_human:.4f} MATIC")

    if usdc_balance == 0:
        print("\n❌ No USDC.e to transfer!")
        sys.exit(0)

    if matic_balance < w3.to_wei(0.01, 'ether'):
        print("\n❌ Not enough MATIC for gas! Need at least 0.01 MATIC.")
        sys.exit(1)

    # Ask for confirmation
    print("\n" + "-" * 60)
    print(f"Transfer ${usdc_human:.2f} USDC.e to Polymarket proxy wallet?")
    print("-" * 60)

    if "--yes" not in sys.argv:
        try:
            confirm = input("\nType 'yes' to confirm (or run with --yes flag): ").strip().lower()
            if confirm != 'yes':
                print("Cancelled.")
                sys.exit(0)
        except EOFError:
            print("\nRun with --yes flag to auto-confirm:")
            print("  python3 transfer_usdc.py --yes")
            sys.exit(0)
    else:
        print("\nAuto-confirmed with --yes flag")

    # Build transaction
    print("\nBuilding transaction...")

    nonce = w3.eth.get_transaction_count(eoa_address)
    gas_price = w3.eth.gas_price

    # Estimate gas for transfer
    transfer_tx = usdc.functions.transfer(
        Web3.to_checksum_address(PROXY_WALLET),
        usdc_balance
    )

    gas_estimate = transfer_tx.estimate_gas({'from': eoa_address})
    gas_limit = int(gas_estimate * 1.2)  # 20% buffer

    print(f"  Nonce: {nonce}")
    print(f"  Gas price: {w3.from_wei(gas_price, 'gwei'):.1f} gwei")
    print(f"  Gas limit: {gas_limit}")

    # Build the transaction
    tx = transfer_tx.build_transaction({
        'chainId': 137,  # Polygon
        'gas': gas_limit,
        'maxFeePerGas': gas_price * 2,
        'maxPriorityFeePerGas': w3.to_wei(30, 'gwei'),
        'nonce': nonce,
    })

    # Sign transaction
    print("\nSigning transaction...")
    signed_tx = w3.eth.account.sign_transaction(tx, private_key)

    # Send transaction
    print("Sending transaction...")
    tx_hash = w3.eth.send_raw_transaction(signed_tx.raw_transaction)
    print(f"  TX Hash: {tx_hash.hex()}")

    # Wait for confirmation
    print("Waiting for confirmation...")
    receipt = w3.eth.wait_for_transaction_receipt(tx_hash, timeout=120)

    if receipt.status == 1:
        print(f"\n✅ Transfer successful!")
        print(f"   TX: https://polygonscan.com/tx/{tx_hash.hex()}")

        # Check new balance
        new_balance = usdc.functions.balanceOf(Web3.to_checksum_address(PROXY_WALLET)).call()
        print(f"   Proxy wallet new USDC.e balance: ${new_balance / 1_000_000:.2f}")
    else:
        print(f"\n❌ Transaction failed!")
        sys.exit(1)

if __name__ == "__main__":
    main()
