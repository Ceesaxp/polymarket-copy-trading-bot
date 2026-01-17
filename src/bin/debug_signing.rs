//! Debug script to compare EIP-712 signing with Python implementation
//! Run with: cargo run --bin debug_signing

use alloy::dyn_abi::eip712::TypedData;
use alloy::primitives::U256;
use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use anyhow::Result;
use dotenvy::dotenv;
use std::env;

fn main() -> Result<()> {
    dotenv().ok();
    println!("{}", "=".repeat(70));
    println!("Rust EIP-712 Signing Debug");
    println!("{}", "=".repeat(70));

    // Constants matching Python test case
    let chain_id: u64 = 137;
    let exchange = "0xC5d563A36AE78145C45a50134d48A1215220f80a";
    let maker = "0x83845c1ABC2594Fc28Dfc8A19510EEc03630dFCF";
    let signer = maker;
    let taker = "0x0000000000000000000000000000000000000000";
    let token_id = "79003893007240922565581139363959835619617307306268940540301817825959399270354";

    // Fixed values matching Python
    let salt: u64 = 124398945;
    let maker_amount: u64 = 1000000;
    let taker_amount: u64 = 2970000;
    let expiration: u64 = 0;
    let nonce: u64 = 0;
    let fee_rate_bps: u64 = 0;
    let side: u8 = 0; // BUY
    let signature_type: u8 = 0; // EOA

    // Convert to U256 for display
    let token_id_u256: U256 = token_id.parse().unwrap();

    println!("\nOrder Data:");
    println!("{}", "-".repeat(70));
    println!("  salt:          {}", salt);
    println!("  maker:         {}", maker);
    println!("  signer:        {}", signer);
    println!("  taker:         {}", taker);
    println!("  tokenId:       {}", token_id_u256);
    println!("  makerAmount:   {}", maker_amount);
    println!("  takerAmount:   {}", taker_amount);
    println!("  expiration:    {}", expiration);
    println!("  nonce:         {}", nonce);
    println!("  feeRateBps:    {}", fee_rate_bps);
    println!("  side:          {}", side);
    println!("  signatureType: {}", signature_type);

    println!("\nDomain:");
    println!("{}", "-".repeat(70));
    println!("  name:              Polymarket CTF Exchange");
    println!("  version:           1");
    println!("  chainId:           {}", chain_id);
    println!("  verifyingContract: {}", exchange);

    // Build the EIP-712 TypedData JSON - matching our lib.rs implementation
    let json_str = format!(
        concat!(
            r#"{{"types":{{"EIP712Domain":["#,
            r#"{{"name":"name","type":"string"}},"#,
            r#"{{"name":"version","type":"string"}},"#,
            r#"{{"name":"chainId","type":"uint256"}},"#,
            r#"{{"name":"verifyingContract","type":"address"}}"#,
            r#"],"Order":["#,
            r#"{{"name":"salt","type":"uint256"}},"#,
            r#"{{"name":"maker","type":"address"}},"#,
            r#"{{"name":"signer","type":"address"}},"#,
            r#"{{"name":"taker","type":"address"}},"#,
            r#"{{"name":"tokenId","type":"uint256"}},"#,
            r#"{{"name":"makerAmount","type":"uint256"}},"#,
            r#"{{"name":"takerAmount","type":"uint256"}},"#,
            r#"{{"name":"expiration","type":"uint256"}},"#,
            r#"{{"name":"nonce","type":"uint256"}},"#,
            r#"{{"name":"feeRateBps","type":"uint256"}},"#,
            r#"{{"name":"side","type":"uint8"}},"#,
            r#"{{"name":"signatureType","type":"uint8"}}"#,
            r#"]}},"primaryType":"Order","#,
            r#""domain":{{"name":"Polymarket CTF Exchange","version":"1","chainId":{},"verifyingContract":"{}"}},"#,
            r#""message":{{"salt":"{}","maker":"{}","signer":"{}","taker":"{}","tokenId":"{}","makerAmount":"{}","takerAmount":"{}","expiration":"{}","nonce":"{}","feeRateBps":"0","side":{},"signatureType":{}}}}}"#
        ),
        chain_id,
        exchange,
        salt,
        maker,
        signer,
        taker,
        token_id_u256,
        maker_amount,
        taker_amount,
        expiration,
        nonce,
        side,
        signature_type
    );

    println!("\nTypedData JSON:");
    println!("{}", "-".repeat(70));
    // Pretty print JSON
    let json_val: serde_json::Value = serde_json::from_str(&json_str)?;
    println!("{}", serde_json::to_string_pretty(&json_val)?);

    // Parse and compute hash
    let typed: TypedData = serde_json::from_str(&json_str)?;
    let hash = typed.eip712_signing_hash()?;

    println!("\nHashes:");
    println!("{}", "-".repeat(70));
    println!("  Struct hash (Rust): 0x{}", hex::encode(hash.as_slice()));

    // Python's expected hash for comparison
    let python_hash = "fdfe0dfe96ac85387202046e3dc500babf224fd9d471c1e3a93207aee10115d7";
    println!("  Struct hash (Python): 0x{}", python_hash);

    if hex::encode(hash.as_slice()) == python_hash {
        println!("\n  ✅ HASHES MATCH!");
    } else {
        println!("\n  ❌ HASHES DO NOT MATCH!");
    }

    // Now sign with the private key
    let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set");
    let key_clean = private_key.trim().trim_start_matches("0x");
    let key_with_prefix = format!("0x{}", key_clean);
    let wallet: PrivateKeySigner = key_with_prefix.parse()?;

    println!("\n  Wallet address: {}", wallet.address());

    let sig = wallet.sign_hash_sync(&hash)?;
    let sig_hex = format!("0x{}", sig);

    println!("  Signature (Rust):   {}", sig_hex);

    // Python's expected signature
    let python_sig = "0xa0f60ecabcdf2ed8e4cb3b6049cfc641d7be9c7241b195c5e20f149f27fec6823197e5fefbfb4b2ba71c50a52826f56ef90b2694a151b76958664764655fb3e01c";
    println!("  Signature (Python): {}", python_sig);

    if sig_hex.to_lowercase() == python_sig.to_lowercase() {
        println!("\n  ✅ SIGNATURES MATCH!");
    } else {
        println!("\n  ❌ SIGNATURES DO NOT MATCH!");
    }

    println!("\n{}", "=".repeat(70));

    Ok(())
}
