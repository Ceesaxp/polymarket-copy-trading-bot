#!/usr/bin/env python3
"""
Debug script to output exact EIP-712 signing data for comparison with Rust.
"""

import os
import json
from dotenv import load_dotenv

load_dotenv()

from eth_account import Account
from py_order_utils.builders import OrderBuilder
from py_order_utils.signer import Signer
from py_order_utils.model.order import Order, OrderData
from py_order_utils.model.sides import BUY
from py_order_utils.model.signatures import EOA
from poly_eip712_structs import make_domain
from eth_utils import keccak

PRIVATE_KEY = os.getenv("PRIVATE_KEY")
if not PRIVATE_KEY.startswith("0x"):
    PRIVATE_KEY = "0x" + PRIVATE_KEY

# Constants matching our Rust test case
CHAIN_ID = 137
EXCHANGE = "0xC5d563A36AE78145C45a50134d48A1215220f80a"  # Neg risk exchange
TOKEN_ID = "79003893007240922565581139363959835619617307306268940540301817825959399270354"

# Fixed values for reproducible comparison
SALT = 124398945
MAKER_AMOUNT = 1000000
TAKER_AMOUNT = 2970000

def main():
    print("=" * 70)
    print("EIP-712 Signing Debug")
    print("=" * 70)

    # Create signer
    signer = Signer(PRIVATE_KEY)
    maker_address = signer.address()

    print(f"\nWallet address: {maker_address}")
    print(f"Chain ID: {CHAIN_ID}")
    print(f"Exchange: {EXCHANGE}")

    # Create order with fixed values
    order = Order(
        salt=SALT,
        maker=maker_address,
        signer=maker_address,
        taker="0x0000000000000000000000000000000000000000",
        tokenId=int(TOKEN_ID),
        makerAmount=MAKER_AMOUNT,
        takerAmount=TAKER_AMOUNT,
        expiration=0,
        nonce=0,
        feeRateBps=0,
        side=0,  # BUY
        signatureType=0,  # EOA
    )

    print("\n" + "-" * 70)
    print("Order Data:")
    print("-" * 70)
    print(f"  salt:          {order['salt']}")
    print(f"  maker:         {order['maker']}")
    print(f"  signer:        {order['signer']}")
    print(f"  taker:         {order['taker']}")
    print(f"  tokenId:       {order['tokenId']}")
    print(f"  makerAmount:   {order['makerAmount']}")
    print(f"  takerAmount:   {order['takerAmount']}")
    print(f"  expiration:    {order['expiration']}")
    print(f"  nonce:         {order['nonce']}")
    print(f"  feeRateBps:    {order['feeRateBps']}")
    print(f"  side:          {order['side']}")
    print(f"  signatureType: {order['signatureType']}")

    # Create domain separator (matching Python library)
    domain = make_domain(
        name="Polymarket CTF Exchange",
        version="1",
        chainId=str(CHAIN_ID),
        verifyingContract=EXCHANGE,
    )

    print("\n" + "-" * 70)
    print("Domain:")
    print("-" * 70)
    print(f"  name:              Polymarket CTF Exchange")
    print(f"  version:           1")
    print(f"  chainId:           {CHAIN_ID}")
    print(f"  verifyingContract: {EXCHANGE}")

    # Get signable bytes and hash
    signable = order.signable_bytes(domain=domain)
    struct_hash = keccak(signable)

    print("\n" + "-" * 70)
    print("Hashes:")
    print("-" * 70)
    print(f"  Signable bytes (hex): {signable.hex()}")
    print(f"  Struct hash (hex):    {struct_hash.hex()}")
    print(f"  Struct hash (0x):     0x{struct_hash.hex()}")

    # Sign it
    signature = signer.sign("0x" + struct_hash.hex())
    print(f"\n  Signature: 0x{signature}")

    # Also show the type hash for Order
    order_type_str = "Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)"
    order_type_hash = keccak(order_type_str.encode())
    print(f"\n  Order type hash: 0x{order_type_hash.hex()}")

    # Domain type hash
    domain_type_str = "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
    domain_type_hash = keccak(domain_type_str.encode())
    print(f"  Domain type hash: 0x{domain_type_hash.hex()}")

    print("\n" + "=" * 70)
    print("Use these values to compare with Rust implementation")
    print("=" * 70)

if __name__ == "__main__":
    main()
