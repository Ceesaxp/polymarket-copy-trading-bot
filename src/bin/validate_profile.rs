//! Private key and wallet validation tool
//! Run with: cargo run --bin validate_profile
//!
//! Validates that PRIVATE_KEY is correct, shows the derived wallet address,
//! and checks stored API credentials in .clob_creds.json.
//! This helps debug "invalid signature" errors.

use anyhow::Result;
use dotenvy::dotenv;
use alloy::signers::local::PrivateKeySigner;
use pm_whale_follower::{ApiCreds, RustClobClient, PreparedCreds};
use std::env;
use std::path::Path;

const CLOB_API_BASE: &str = "https://clob.polymarket.com";
const CREDS_PATH: &str = ".clob_creds.json";

fn main() -> Result<()> {
    dotenv().ok();

    println!("🔐 Validating Profile...\n");

    // Get PRIVATE_KEY
    let private_key = match env::var("PRIVATE_KEY") {
        Ok(k) => k,
        Err(_) => {
            println!("❌ PRIVATE_KEY not set in environment");
            return Ok(());
        }
    };

    // Clean up the key (remove 0x prefix, whitespace)
    let key_clean = private_key.trim().trim_start_matches("0x");

    // Validate format
    if key_clean.len() != 64 {
        println!("❌ PRIVATE_KEY invalid length: {} chars (expected 64)", key_clean.len());
        return Ok(());
    }

    if !key_clean.chars().all(|c| c.is_ascii_hexdigit()) {
        println!("❌ PRIVATE_KEY contains non-hex characters");
        return Ok(());
    }

    // Try to parse as a wallet
    let key_with_prefix = format!("0x{}", key_clean);
    let signer: PrivateKeySigner = match key_with_prefix.parse() {
        Ok(s) => s,
        Err(e) => {
            println!("❌ PRIVATE_KEY failed to parse as valid secp256k1 key: {}", e);
            return Ok(());
        }
    };

    let wallet_address = format!("{}", signer.address());

    println!("✅ PRIVATE_KEY is valid\n");
    println!("📍 Derived wallet address: {}", wallet_address);
    println!("   (lowercase): {}\n", wallet_address.to_lowercase());

    // Check USE_SEPARATE_FUNDER and FUNDER_ADDRESS
    let use_separate_funder = env::var("USE_SEPARATE_FUNDER").is_ok();
    let funder_for_client: Option<String>;

    if use_separate_funder {
        println!("⚠️  USE_SEPARATE_FUNDER is set - using separate funder mode\n");

        match env::var("FUNDER_ADDRESS") {
            Ok(funder) => {
                let funder_clean = funder.trim().to_string();
                println!("📍 FUNDER_ADDRESS: {}", funder_clean);

                // Compare addresses (case-insensitive)
                if wallet_address.to_lowercase() == funder_clean.to_lowercase() {
                    println!("   ✅ FUNDER_ADDRESS matches derived wallet");
                    println!("   ℹ️  Consider removing USE_SEPARATE_FUNDER since addresses match\n");
                } else {
                    println!("   ⚠️  FUNDER_ADDRESS differs from derived wallet!");
                    println!("   This is valid if you're using a proxy/funder pattern,");
                    println!("   but will cause 'invalid signature' if the funder hasn't");
                    println!("   authorized this wallet as a signer.\n");
                }
                funder_for_client = Some(funder_clean);
            }
            Err(_) => {
                println!("❌ USE_SEPARATE_FUNDER is set but FUNDER_ADDRESS is missing!");
                funder_for_client = None;
            }
        }
    } else {
        println!("ℹ️  USE_SEPARATE_FUNDER not set - funder derived from PRIVATE_KEY (recommended)");
        println!("   The bot will use {} as both signer and funder.\n", wallet_address);
        funder_for_client = None;
    }

    // Check stored API credentials
    println!("{}", "=".repeat(60));
    println!("🔑 Checking API Credentials ({})...\n", CREDS_PATH);

    if !Path::new(CREDS_PATH).exists() {
        println!("   ℹ️  {} not found", CREDS_PATH);
        println!("   This is normal for first run - credentials will be derived automatically.\n");
        println!("   To pre-derive credentials, run the main bot once or use derive_api_key().\n");
    } else {
        match std::fs::read_to_string(CREDS_PATH) {
            Ok(data) => {
                match serde_json::from_str::<ApiCreds>(&data) {
                    Ok(creds) => {
                        println!("   ✅ {} found and parsed\n", CREDS_PATH);

                        // Mask the sensitive values
                        let api_key_masked = mask_string(&creds.api_key, 8);
                        let secret_masked = mask_string(&creds.api_secret, 8);
                        let passphrase_masked = mask_string(&creds.api_passphrase, 4);

                        println!("   API Key:      {}", api_key_masked);
                        println!("   Secret:       {}", secret_masked);
                        println!("   Passphrase:   {}\n", passphrase_masked);

                        // Now validate the credentials against the API
                        println!("   🔄 Validating credentials against CLOB API...\n");

                        match validate_api_creds(&private_key, funder_for_client.as_deref(), &creds) {
                            Ok(info) => {
                                println!("   ✅ API credentials are VALID\n");
                                if let Some(i) = info {
                                    println!("   {}", i);
                                }
                            }
                            Err(e) => {
                                println!("   ❌ API credentials are INVALID\n");
                                println!("   Error: {}\n", e);
                                println!("   💡 Suggested fix: Delete {} and restart the bot", CREDS_PATH);
                                println!("      to derive fresh credentials.\n");
                                println!("      $ rm {}", CREDS_PATH);
                            }
                        }
                    }
                    Err(e) => {
                        println!("   ❌ Failed to parse {}: {}", CREDS_PATH, e);
                        println!("   💡 Delete the file and let the bot re-derive credentials.\n");
                    }
                }
            }
            Err(e) => {
                println!("   ❌ Failed to read {}: {}", CREDS_PATH, e);
            }
        }
    }

    // Summary
    println!("{}", "=".repeat(60));
    println!("📋 Summary for Polymarket CLOB API:\n");
    println!("   Signer (from PRIVATE_KEY): {}", wallet_address);

    if use_separate_funder {
        if let Some(ref funder) = funder_for_client {
            println!("   Maker/Funder (FUNDER_ADDRESS): {}", funder);
        }
    } else {
        println!("   Maker/Funder (derived): {}", wallet_address);
    }

    println!("\n   For valid signatures, 'signer' and 'maker' addresses in the");
    println!("   signed order must match, OR the maker must have authorized");
    println!("   the signer via Polymarket's operator approval system.\n");

    Ok(())
}

fn mask_string(s: &str, visible_chars: usize) -> String {
    if s.len() <= visible_chars {
        return s.to_string();
    }
    let visible = &s[..visible_chars];
    let masked_len = s.len() - visible_chars;
    format!("{}...{} chars hidden", visible, masked_len)
}

fn validate_api_creds(private_key: &str, funder: Option<&str>, creds: &ApiCreds) -> Result<Option<String>> {
    // Create client
    let client = RustClobClient::new(CLOB_API_BASE, 137, private_key, funder)?;
    let prepared = PreparedCreds::from_api_creds(creds)?;

    // Show the addresses that will be used in orders
    println!("   📋 Order address configuration:");
    println!("      signer (wallet):  {}", client.wallet_address());
    println!("      maker (funder):   {}", client.funder_address());

    if client.wallet_address() == client.funder_address() {
        println!("      ✅ Addresses match exactly");
    } else {
        println!("      ⚠️  Addresses DIFFER - checking case...");
        if client.wallet_address().to_lowercase() == client.funder_address().to_lowercase() {
            println!("      ⚠️  Same address but different case - may cause signature issues!");
        } else {
            println!("      ❌ Different addresses - requires delegation to be configured!");
        }
    }
    println!();

    // Try to fetch open orders (authenticated endpoint)
    // GET /data/orders requires L2 authentication
    let path = "/data/orders";
    let url = format!("{}{}", CLOB_API_BASE, path);

    let headers = client.l2_headers_fast("GET", path, None, &prepared)?;

    let resp = client.http_client()
        .get(&url)
        .headers(headers)
        .timeout(std::time::Duration::from_secs(10))
        .send()?;

    let status = resp.status();
    let body = resp.text().unwrap_or_default();

    if status.is_success() {
        // Try to count orders
        if let Ok(orders) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
            return Ok(Some(format!("Found {} open orders for this account", orders.len())));
        }
        return Ok(Some("Response OK".to_string()));
    }

    // Check specific error cases
    if status.as_u16() == 401 {
        if body.contains("invalid api key") || body.contains("INVALID_API_KEY") {
            anyhow::bail!("Invalid API key - credentials may have been revoked or are for a different wallet");
        }
        if body.contains("invalid signature") || body.contains("INVALID_SIGNATURE") {
            anyhow::bail!("Invalid signature - HMAC secret may be corrupted or wallet address mismatch");
        }
        anyhow::bail!("401 Unauthorized: {}", body);
    }

    if status.as_u16() == 400 {
        anyhow::bail!("400 Bad Request: {}", body);
    }

    anyhow::bail!("HTTP {}: {}", status, body);
}
