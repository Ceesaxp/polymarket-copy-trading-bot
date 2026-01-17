//! Check USDC balance and token allowances for Polymarket trading
//! Run with: cargo run --bin check_balance

use alloy::signers::local::PrivateKeySigner;
use anyhow::Result;
use dotenvy::dotenv;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::env;

// Polygon USDC contracts
const USDC_POLYGON: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174"; // USDC.e (bridged)
const USDC_NATIVE: &str = "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"; // Native USDC

// Polymarket exchange contracts
const CTF_EXCHANGE: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
const NEG_RISK_CTF_EXCHANGE: &str = "0xC5d563A36AE78145C45a50134d48A1215220f80a";

// Polygon RPC endpoint
const POLYGON_RPC: &str = "https://polygon-rpc.com";

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    method: &'static str,
    params: Vec<serde_json::Value>,
    id: u32,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<String>,
    error: Option<serde_json::Value>,
}

fn main() -> Result<()> {
    dotenv().ok();

    println!("{}", "=".repeat(70));
    println!("Polymarket Wallet Balance & Allowance Check");
    println!("{}", "=".repeat(70));

    // Get wallet address from PRIVATE_KEY
    let private_key = env::var("PRIVATE_KEY").expect("PRIVATE_KEY not set");
    let key_clean = private_key.trim().trim_start_matches("0x");
    let key_with_prefix = format!("0x{}", key_clean);
    let wallet: PrivateKeySigner = key_with_prefix.parse()?;
    let wallet_address = format!("{}", wallet.address());

    println!("\nWallet: {}", wallet_address);

    // Check if using separate funder
    let funder_address = if env::var("USE_SEPARATE_FUNDER").is_ok() {
        env::var("FUNDER_ADDRESS").ok()
    } else {
        None
    };

    let check_address = funder_address.as_ref().unwrap_or(&wallet_address);
    if funder_address.is_some() {
        println!("Funder: {} (checking this address)", check_address);
    }

    let client = Client::new();

    println!("\n{}", "-".repeat(70));
    println!("USDC Balances:");
    println!("{}", "-".repeat(70));

    // Check USDC.e balance (bridged USDC - most common)
    let usdc_e_balance = get_balance(&client, USDC_POLYGON, check_address)?;
    let usdc_e_human = usdc_e_balance as f64 / 1_000_000.0; // USDC has 6 decimals
    println!("  USDC.e (bridged):  ${:.2} ({} raw)", usdc_e_human, usdc_e_balance);

    // Check native USDC balance
    let usdc_native_balance = get_balance(&client, USDC_NATIVE, check_address)?;
    let usdc_native_human = usdc_native_balance as f64 / 1_000_000.0;
    println!("  USDC (native):     ${:.2} ({} raw)", usdc_native_human, usdc_native_balance);

    let total_usdc = usdc_e_human + usdc_native_human;
    println!("  Total USDC:        ${:.2}", total_usdc);

    if total_usdc < 1.0 {
        println!("\n  ⚠️  Low balance! You need USDC on Polygon to place orders.");
        println!("     Send USDC to: {}", check_address);
    }

    println!("\n{}", "-".repeat(70));
    println!("Token Allowances (USDC.e -> Exchanges):");
    println!("{}", "-".repeat(70));

    // Check allowance for CTF Exchange
    let ctf_allowance = get_allowance(&client, USDC_POLYGON, check_address, CTF_EXCHANGE)?;
    let ctf_allowance_human = if ctf_allowance > u64::MAX as u128 {
        "unlimited".to_string()
    } else {
        format!("${:.2}", ctf_allowance as f64 / 1_000_000.0)
    };
    println!("  CTF Exchange:          {}", ctf_allowance_human);
    if ctf_allowance == 0 {
        println!("     ❌ No allowance set for regular markets!");
    } else {
        println!("     ✅ Allowance OK");
    }

    // Check allowance for Neg Risk CTF Exchange
    let neg_risk_allowance = get_allowance(&client, USDC_POLYGON, check_address, NEG_RISK_CTF_EXCHANGE)?;
    let neg_risk_allowance_human = if neg_risk_allowance > u64::MAX as u128 {
        "unlimited".to_string()
    } else {
        format!("${:.2}", neg_risk_allowance as f64 / 1_000_000.0)
    };
    println!("  Neg Risk CTF Exchange: {}", neg_risk_allowance_human);
    if neg_risk_allowance == 0 {
        println!("     ❌ No allowance set for neg-risk markets!");
    } else {
        println!("     ✅ Allowance OK");
    }

    // Also check native USDC allowances if there's a balance
    if usdc_native_balance > 0 {
        println!("\n{}", "-".repeat(70));
        println!("Token Allowances (Native USDC -> Exchanges):");
        println!("{}", "-".repeat(70));

        let ctf_native = get_allowance(&client, USDC_NATIVE, check_address, CTF_EXCHANGE)?;
        let ctf_native_human = if ctf_native > u64::MAX as u128 {
            "unlimited".to_string()
        } else {
            format!("${:.2}", ctf_native as f64 / 1_000_000.0)
        };
        println!("  CTF Exchange:          {}", ctf_native_human);

        let neg_native = get_allowance(&client, USDC_NATIVE, check_address, NEG_RISK_CTF_EXCHANGE)?;
        let neg_native_human = if neg_native > u64::MAX as u128 {
            "unlimited".to_string()
        } else {
            format!("${:.2}", neg_native as f64 / 1_000_000.0)
        };
        println!("  Neg Risk CTF Exchange: {}", neg_native_human);
    }

    // Summary
    println!("\n{}", "=".repeat(70));
    println!("Summary:");
    println!("{}", "=".repeat(70));

    let has_balance = total_usdc >= 1.0;
    let has_allowances = ctf_allowance > 0 && neg_risk_allowance > 0;

    if has_balance && has_allowances {
        println!("  ✅ Wallet is ready for trading!");
    } else {
        println!("  ❌ Wallet needs setup:");
        if !has_balance {
            println!("     - Send USDC to {} on Polygon", check_address);
        }
        if ctf_allowance == 0 || neg_risk_allowance == 0 {
            println!("     - Set token allowances (easiest via Polymarket web interface)");
            println!("     - Or approve USDC spending for:");
            if ctf_allowance == 0 {
                println!("       * CTF Exchange: {}", CTF_EXCHANGE);
            }
            if neg_risk_allowance == 0 {
                println!("       * Neg Risk Exchange: {}", NEG_RISK_CTF_EXCHANGE);
            }
        }
    }

    println!();
    Ok(())
}

fn get_balance(client: &Client, token: &str, owner: &str) -> Result<u128> {
    // balanceOf(address) selector: 0x70a08231
    let owner_padded = format!("{:0>64}", owner.trim_start_matches("0x").to_lowercase());
    let data = format!("0x70a08231{}", owner_padded);

    let result = eth_call(client, token, &data)?;
    parse_uint256(&result)
}

fn get_allowance(client: &Client, token: &str, owner: &str, spender: &str) -> Result<u128> {
    // allowance(address,address) selector: 0xdd62ed3e
    let owner_padded = format!("{:0>64}", owner.trim_start_matches("0x").to_lowercase());
    let spender_padded = format!("{:0>64}", spender.trim_start_matches("0x").to_lowercase());
    let data = format!("0xdd62ed3e{}{}", owner_padded, spender_padded);

    let result = eth_call(client, token, &data)?;
    parse_uint256(&result)
}

fn eth_call(client: &Client, to: &str, data: &str) -> Result<String> {
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "eth_call",
        params: vec![
            serde_json::json!({
                "to": to,
                "data": data
            }),
            serde_json::json!("latest"),
        ],
        id: 1,
    };

    let response: JsonRpcResponse = client
        .post(POLYGON_RPC)
        .json(&request)
        .send()?
        .json()?;

    if let Some(error) = response.error {
        anyhow::bail!("RPC error: {:?}", error);
    }

    Ok(response.result.unwrap_or_else(|| "0x0".to_string()))
}

fn parse_uint256(hex: &str) -> Result<u128> {
    let hex_clean = hex.trim_start_matches("0x");
    if hex_clean.is_empty() || hex_clean == "0" {
        return Ok(0);
    }
    // Parse as u128 (sufficient for USDC amounts)
    Ok(u128::from_str_radix(hex_clean, 16).unwrap_or(0))
}
