//! Approve USDC allowances for Polymarket exchange contracts
//! Run with: cargo run --bin approve_allowances

use alloy::primitives::U256;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use anyhow::Result;
use dotenvy::dotenv;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::env;

// Polygon USDC.e contract (bridged USDC - what Polymarket uses)
const USDC_POLYGON: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";

// Polymarket exchange contracts
const CTF_EXCHANGE: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
const NEG_RISK_CTF_EXCHANGE: &str = "0xC5d563A36AE78145C45a50134d48A1215220f80a";

// Polygon RPC endpoint
const POLYGON_RPC: &str = "https://polygon-rpc.com";

// Chain ID for Polygon
const CHAIN_ID: u64 = 137;

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    params: Vec<serde_json::Value>,
    id: u32,
}

#[derive(Deserialize, Debug)]
struct JsonRpcResponse {
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}

fn main() -> Result<()> {
    dotenv().ok();

    println!("{}", "=".repeat(70));
    println!("Approve USDC Allowances for Polymarket");
    println!("{}", "=".repeat(70));

    // Get wallet from PRIVATE_KEY
    let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set");
    let key_clean = private_key.trim().trim_start_matches("0x");
    let key_with_prefix = format!("0x{}", key_clean);
    let wallet: PrivateKeySigner = key_with_prefix.parse()?;
    let wallet_address = wallet.address();

    println!("\nWallet: {}", wallet_address);

    let client = Client::new();

    // Check current allowances first
    println!("\n{}", "-".repeat(70));
    println!("Current Allowances:");
    println!("{}", "-".repeat(70));

    let ctf_allowance = get_allowance(&client, USDC_POLYGON, &format!("{}", wallet_address), CTF_EXCHANGE)?;
    let neg_risk_allowance = get_allowance(&client, USDC_POLYGON, &format!("{}", wallet_address), NEG_RISK_CTF_EXCHANGE)?;

    println!("  CTF Exchange:          {}", format_allowance(ctf_allowance));
    println!("  Neg Risk CTF Exchange: {}", format_allowance(neg_risk_allowance));

    // Determine which approvals are needed
    let max_allowance = U256::MAX;
    let needs_ctf = ctf_allowance < U256::from(1_000_000_000_000u64); // Less than $1M
    let needs_neg_risk = neg_risk_allowance < U256::from(1_000_000_000_000u64);

    if !needs_ctf && !needs_neg_risk {
        println!("\n✅ Allowances already set! No action needed.");
        return Ok(());
    }

    println!("\n{}", "-".repeat(70));
    println!("Setting Allowances:");
    println!("{}", "-".repeat(70));

    // Get current nonce
    let nonce = get_nonce(&client, &format!("{}", wallet_address))?;
    println!("  Current nonce: {}", nonce);

    // Get gas prices
    let (max_fee, priority_fee) = get_gas_prices(&client)?;
    println!("  Max fee: {} gwei, Priority fee: {} gwei",
             max_fee / 1_000_000_000, priority_fee / 1_000_000_000);

    let mut current_nonce = nonce;

    // Approve CTF Exchange if needed
    if needs_ctf {
        println!("\n  Approving CTF Exchange...");
        let tx_hash = send_approve_tx(
            &client,
            &wallet,
            USDC_POLYGON,
            CTF_EXCHANGE,
            max_allowance,
            current_nonce,
            max_fee,
            priority_fee,
        )?;
        println!("    TX: {}", tx_hash);
        println!("    Waiting for confirmation...");
        wait_for_tx(&client, &tx_hash)?;
        println!("    ✅ Confirmed!");
        current_nonce += 1;
    }

    // Approve Neg Risk CTF Exchange if needed
    if needs_neg_risk {
        println!("\n  Approving Neg Risk CTF Exchange...");
        let tx_hash = send_approve_tx(
            &client,
            &wallet,
            USDC_POLYGON,
            NEG_RISK_CTF_EXCHANGE,
            max_allowance,
            current_nonce,
            max_fee,
            priority_fee,
        )?;
        println!("    TX: {}", tx_hash);
        println!("    Waiting for confirmation...");
        wait_for_tx(&client, &tx_hash)?;
        println!("    ✅ Confirmed!");
    }

    // Verify new allowances
    println!("\n{}", "-".repeat(70));
    println!("New Allowances:");
    println!("{}", "-".repeat(70));

    let new_ctf = get_allowance(&client, USDC_POLYGON, &format!("{}", wallet_address), CTF_EXCHANGE)?;
    let new_neg_risk = get_allowance(&client, USDC_POLYGON, &format!("{}", wallet_address), NEG_RISK_CTF_EXCHANGE)?;

    println!("  CTF Exchange:          {}", format_allowance(new_ctf));
    println!("  Neg Risk CTF Exchange: {}", format_allowance(new_neg_risk));

    println!("\n{}", "=".repeat(70));
    println!("✅ Allowances set successfully! You can now trade on Polymarket.");
    println!("{}", "=".repeat(70));

    Ok(())
}

fn format_allowance(amount: U256) -> String {
    if amount > U256::from(u128::MAX) {
        "unlimited".to_string()
    } else {
        let val: u128 = amount.try_into().unwrap_or(0);
        if val > 1_000_000_000_000 {
            "unlimited".to_string()
        } else {
            format!("${:.2}", val as f64 / 1_000_000.0)
        }
    }
}

fn get_allowance(client: &Client, token: &str, owner: &str, spender: &str) -> Result<U256> {
    let owner_padded = format!("{:0>64}", owner.trim_start_matches("0x").to_lowercase());
    let spender_padded = format!("{:0>64}", spender.trim_start_matches("0x").to_lowercase());
    let data = format!("0xdd62ed3e{}{}", owner_padded, spender_padded);

    let result = eth_call(client, token, &data)?;
    parse_uint256(&result)
}

fn get_nonce(client: &Client, address: &str) -> Result<u64> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "eth_getTransactionCount",
        params: vec![
            serde_json::json!(address),
            serde_json::json!("latest"),
        ],
        id: 1,
    };

    let response: JsonRpcResponse = client.post(POLYGON_RPC).json(&request).send()?.json()?;

    if let Some(error) = response.error {
        anyhow::bail!("RPC error: {:?}", error);
    }

    let hex = response.result
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "0x0".to_string());

    Ok(u64::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap_or(0))
}

fn get_gas_prices(client: &Client) -> Result<(u64, u64)> {
    // Get base fee from latest block
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "eth_gasPrice",
        params: vec![],
        id: 1,
    };

    let response: JsonRpcResponse = client.post(POLYGON_RPC).json(&request).send()?.json()?;
    let gas_price_hex = response.result
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "0x0".to_string());
    let gas_price = u64::from_str_radix(gas_price_hex.trim_start_matches("0x"), 16).unwrap_or(30_000_000_000);

    // Use reasonable values for Polygon
    let max_fee = gas_price.max(50_000_000_000); // At least 50 gwei
    let priority_fee = 30_000_000_000u64; // 30 gwei priority

    Ok((max_fee, priority_fee))
}

fn send_approve_tx(
    client: &Client,
    wallet: &PrivateKeySigner,
    token: &str,
    spender: &str,
    amount: U256,
    nonce: u64,
    max_fee: u64,
    priority_fee: u64,
) -> Result<String> {
    // Build approve(address,uint256) calldata
    // Function selector: 0x095ea7b3
    let spender_padded = format!("{:0>64}", spender.trim_start_matches("0x").to_lowercase());
    let amount_hex = format!("{:0>64x}", amount);
    let data = format!("0x095ea7b3{}{}", spender_padded, amount_hex);

    // Gas limit for approve is typically ~50k, use 60k to be safe
    let gas_limit = 60_000u64;

    // Build EIP-1559 transaction
    // Type 2 transaction format
    let tx_fields = rlp_encode_eip1559_tx(
        CHAIN_ID,
        nonce,
        priority_fee,
        max_fee,
        gas_limit,
        token,
        0, // value = 0
        &data,
    );

    // Sign the transaction
    let tx_hash = alloy::primitives::keccak256(&tx_fields);
    let signature = wallet.sign_hash_sync(&tx_hash)?;

    // Extract signature components
    // For EIP-1559, v() returns y_parity as bool directly
    let y_parity = signature.v();
    let r = signature.r();
    let s = signature.s();

    // Encode signed transaction
    let signed_tx = rlp_encode_signed_eip1559_tx(
        CHAIN_ID,
        nonce,
        priority_fee,
        max_fee,
        gas_limit,
        token,
        0,
        &data,
        y_parity,
        r,
        s,
    );

    let signed_tx_hex = format!("0x02{}", hex::encode(&signed_tx));

    // Send transaction
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "eth_sendRawTransaction",
        params: vec![serde_json::json!(signed_tx_hex)],
        id: 1,
    };

    let response: JsonRpcResponse = client.post(POLYGON_RPC).json(&request).send()?.json()?;

    if let Some(error) = response.error {
        anyhow::bail!("Transaction failed: {:?}", error);
    }

    let tx_hash = response.result
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| anyhow::anyhow!("No transaction hash returned"))?;

    Ok(tx_hash)
}

fn wait_for_tx(client: &Client, tx_hash: &str) -> Result<()> {
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_secs(2));

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method: "eth_getTransactionReceipt",
            params: vec![serde_json::json!(tx_hash)],
            id: 1,
        };

        let response: JsonRpcResponse = client.post(POLYGON_RPC).json(&request).send()?.json()?;

        if let Some(result) = response.result {
            if !result.is_null() {
                // Check status
                if let Some(status) = result.get("status").and_then(|s| s.as_str()) {
                    if status == "0x1" {
                        return Ok(());
                    } else {
                        anyhow::bail!("Transaction reverted");
                    }
                }
                return Ok(());
            }
        }
    }

    anyhow::bail!("Transaction not confirmed after 120 seconds")
}

fn eth_call(client: &Client, to: &str, data: &str) -> Result<String> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "eth_call",
        params: vec![
            serde_json::json!({"to": to, "data": data}),
            serde_json::json!("latest"),
        ],
        id: 1,
    };

    let response: JsonRpcResponse = client.post(POLYGON_RPC).json(&request).send()?.json()?;

    if let Some(error) = response.error {
        anyhow::bail!("RPC error: {:?}", error);
    }

    Ok(response.result
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "0x0".to_string()))
}

fn parse_uint256(hex: &str) -> Result<U256> {
    let hex_clean = hex.trim_start_matches("0x");
    if hex_clean.is_empty() || hex_clean == "0" {
        return Ok(U256::ZERO);
    }
    Ok(U256::from_str_radix(hex_clean, 16).unwrap_or(U256::ZERO))
}

// RLP encoding helpers for EIP-1559 transactions
fn rlp_encode_eip1559_tx(
    chain_id: u64,
    nonce: u64,
    max_priority_fee: u64,
    max_fee: u64,
    gas_limit: u64,
    to: &str,
    value: u64,
    data: &str,
) -> Vec<u8> {
    let mut items: Vec<Vec<u8>> = Vec::new();

    items.push(rlp_encode_uint(chain_id));
    items.push(rlp_encode_uint(nonce));
    items.push(rlp_encode_uint(max_priority_fee));
    items.push(rlp_encode_uint(max_fee));
    items.push(rlp_encode_uint(gas_limit));
    items.push(rlp_encode_address(to));
    items.push(rlp_encode_uint(value));
    items.push(rlp_encode_bytes(&hex::decode(data.trim_start_matches("0x")).unwrap_or_default()));
    items.push(rlp_encode_list(&[])); // access list (empty)

    let mut result = vec![0x02]; // EIP-1559 type
    result.extend(rlp_encode_list_from_items(&items));
    result
}

fn rlp_encode_signed_eip1559_tx(
    chain_id: u64,
    nonce: u64,
    max_priority_fee: u64,
    max_fee: u64,
    gas_limit: u64,
    to: &str,
    value: u64,
    data: &str,
    y_parity: bool,
    r: U256,
    s: U256,
) -> Vec<u8> {
    let mut items: Vec<Vec<u8>> = Vec::new();

    items.push(rlp_encode_uint(chain_id));
    items.push(rlp_encode_uint(nonce));
    items.push(rlp_encode_uint(max_priority_fee));
    items.push(rlp_encode_uint(max_fee));
    items.push(rlp_encode_uint(gas_limit));
    items.push(rlp_encode_address(to));
    items.push(rlp_encode_uint(value));
    items.push(rlp_encode_bytes(&hex::decode(data.trim_start_matches("0x")).unwrap_or_default()));
    items.push(rlp_encode_list(&[])); // access list (empty)
    items.push(rlp_encode_uint(if y_parity { 1u64 } else { 0u64 }));
    items.push(rlp_encode_u256(r));
    items.push(rlp_encode_u256(s));

    rlp_encode_list_from_items(&items)
}

fn rlp_encode_uint(value: u64) -> Vec<u8> {
    if value == 0 {
        return vec![0x80];
    }
    let bytes = value.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(8);
    let trimmed = &bytes[start..];

    if trimmed.len() == 1 && trimmed[0] < 0x80 {
        trimmed.to_vec()
    } else {
        let mut result = vec![0x80 + trimmed.len() as u8];
        result.extend(trimmed);
        result
    }
}

fn rlp_encode_u256(value: U256) -> Vec<u8> {
    if value.is_zero() {
        return vec![0x80];
    }
    let bytes: [u8; 32] = value.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(32);
    let trimmed = &bytes[start..];

    if trimmed.len() == 1 && trimmed[0] < 0x80 {
        trimmed.to_vec()
    } else {
        let mut result = vec![0x80 + trimmed.len() as u8];
        result.extend(trimmed);
        result
    }
}

fn rlp_encode_address(addr: &str) -> Vec<u8> {
    let bytes = hex::decode(addr.trim_start_matches("0x")).unwrap_or_default();
    let mut result = vec![0x80 + bytes.len() as u8];
    result.extend(bytes);
    result
}

fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.is_empty() {
        return vec![0x80];
    }
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return bytes.to_vec();
    }
    if bytes.len() < 56 {
        let mut result = vec![0x80 + bytes.len() as u8];
        result.extend(bytes);
        result
    } else {
        let len_bytes = (bytes.len() as u64).to_be_bytes();
        let len_start = len_bytes.iter().position(|&b| b != 0).unwrap_or(8);
        let len_trimmed = &len_bytes[len_start..];

        let mut result = vec![0xb7 + len_trimmed.len() as u8];
        result.extend(len_trimmed);
        result.extend(bytes);
        result
    }
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let content: Vec<u8> = items.iter().flatten().cloned().collect();
    rlp_encode_list_raw(&content)
}

fn rlp_encode_list_from_items(items: &[Vec<u8>]) -> Vec<u8> {
    let content: Vec<u8> = items.iter().flatten().cloned().collect();
    rlp_encode_list_raw(&content)
}

fn rlp_encode_list_raw(content: &[u8]) -> Vec<u8> {
    if content.len() < 56 {
        let mut result = vec![0xc0 + content.len() as u8];
        result.extend(content);
        result
    } else {
        let len_bytes = (content.len() as u64).to_be_bytes();
        let len_start = len_bytes.iter().position(|&b| b != 0).unwrap_or(8);
        let len_trimmed = &len_bytes[len_start..];

        let mut result = vec![0xf7 + len_trimmed.len() as u8];
        result.extend(len_trimmed);
        result.extend(content);
        result
    }
}
