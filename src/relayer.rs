/// Builder Relayer Client for Polymarket
/// Handles gasless transactions including redemptions through Polymarket's infrastructure
///
/// Requires Builder credentials:
/// - POLY_BUILDER_API_KEY
/// - POLY_BUILDER_SECRET
/// - POLY_BUILDER_PASSPHRASE

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE;
use base64::Engine;
use hmac::{Hmac, Mac};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

const RELAYER_URL: &str = "https://relayer-v2.polymarket.com";
const USER_AGENT: &str = "pm_whale_follower";

// Contract addresses on Polygon mainnet
pub const CTF_CONTRACT: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
pub const USDC_ADDRESS: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";

/// Builder credentials for the Relayer API
#[derive(Debug, Clone)]
pub struct BuilderCreds {
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
}

impl BuilderCreds {
    /// Load credentials from environment variables
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("POLY_BUILDER_API_KEY")
            .map_err(|_| anyhow!("POLY_BUILDER_API_KEY not set"))?;
        let secret = std::env::var("POLY_BUILDER_SECRET")
            .map_err(|_| anyhow!("POLY_BUILDER_SECRET not set"))?;
        let passphrase = std::env::var("POLY_BUILDER_PASSPHRASE")
            .map_err(|_| anyhow!("POLY_BUILDER_PASSPHRASE not set"))?;

        Ok(Self {
            api_key,
            secret,
            passphrase,
        })
    }

    /// Create prepared credentials with pre-computed HMAC template
    pub fn prepare(&self) -> Result<PreparedBuilderCreds> {
        PreparedBuilderCreds::new(self)
    }
}

/// Prepared credentials with HMAC template for fast signing
#[derive(Clone)]
pub struct PreparedBuilderCreds {
    pub api_key: String,
    pub passphrase: String,
    hmac_template: HmacSha256,
}

impl PreparedBuilderCreds {
    pub fn new(creds: &BuilderCreds) -> Result<Self> {
        // The secret may be base64 encoded or raw - try both
        let secret_bytes = if creds.secret.len() == 64 && creds.secret.chars().all(|c| c.is_ascii_hexdigit()) {
            // Hex encoded secret
            hex::decode(&creds.secret)?
        } else {
            // Try URL-safe base64 first, then standard base64, then raw
            URL_SAFE
                .decode(&creds.secret)
                .or_else(|_| base64::engine::general_purpose::STANDARD.decode(&creds.secret))
                .unwrap_or_else(|_| creds.secret.as_bytes().to_vec())
        };

        let hmac_template = HmacSha256::new_from_slice(&secret_bytes)
            .map_err(|e| anyhow!("Invalid HMAC key: {}", e))?;

        Ok(Self {
            api_key: creds.api_key.clone(),
            passphrase: creds.passphrase.clone(),
            hmac_template,
        })
    }

    /// Sign a message and return base64-encoded signature
    pub fn sign(&self, message: &str) -> String {
        let mut mac = self.hmac_template.clone();
        mac.update(message.as_bytes());
        let result = mac.finalize();
        URL_SAFE.encode(result.into_bytes())
    }
}

/// A transaction to be executed through the relayer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayerTransaction {
    pub to: String,
    pub data: String,
    #[serde(default = "default_value")]
    pub value: String,
}

fn default_value() -> String {
    "0".to_string()
}

/// Request body for execute endpoint
#[derive(Debug, Serialize)]
struct ExecuteRequest {
    transactions: Vec<RelayerTransaction>,
    description: String,
}

/// Response from the relayer
#[derive(Debug, Clone, Deserialize)]
pub struct RelayerResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub state: String,
    #[serde(rename = "transactionHash", default)]
    pub transaction_hash: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Builder Relayer Client
pub struct RelayerClient {
    http: Client,
    creds: PreparedBuilderCreds,
    proxy_wallet: String,
}

impl RelayerClient {
    /// Create a new relayer client
    pub fn new(creds: PreparedBuilderCreds, proxy_wallet: &str) -> Result<Self> {
        let http = Client::builder()
            .pool_max_idle_per_host(4)
            .pool_idle_timeout(Duration::from_secs(60))
            .tcp_keepalive(Duration::from_secs(30))
            .tcp_nodelay(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .user_agent(USER_AGENT)
            .build()?;

        Ok(Self {
            http,
            creds,
            proxy_wallet: proxy_wallet.to_lowercase(),
        })
    }

    /// Generate authentication headers for a request
    fn auth_headers(&self, method: &str, path: &str, body: Option<&str>) -> Result<HeaderMap> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs()
            .to_string();

        // Message format: timestamp + method + path + body
        let body_str = body.unwrap_or("");
        let message = format!("{}{}{}{}", timestamp, method, path, body_str);
        let signature = self.creds.sign(&message);

        let mut headers = HeaderMap::new();
        headers.insert(
            "POLY_BUILDER_API_KEY",
            HeaderValue::from_str(&self.creds.api_key)?,
        );
        headers.insert("POLY_BUILDER_TIMESTAMP", HeaderValue::from_str(&timestamp)?);
        headers.insert(
            "POLY_BUILDER_PASSPHRASE",
            HeaderValue::from_str(&self.creds.passphrase)?,
        );
        headers.insert("POLY_BUILDER_SIGNATURE", HeaderValue::from_str(&signature)?);
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));

        Ok(headers)
    }

    /// Execute transactions through the relayer
    pub fn execute(
        &self,
        transactions: Vec<RelayerTransaction>,
        description: &str,
    ) -> Result<RelayerResponse> {
        let path = format!("/execute?account={}", self.proxy_wallet);
        let url = format!("{}{}", RELAYER_URL, path);

        let request = ExecuteRequest {
            transactions,
            description: description.to_string(),
        };

        let body = serde_json::to_string(&request)?;
        let headers = self.auth_headers("POST", &path, Some(&body))?;

        let response = self.http.post(&url).headers(headers).body(body).send()?;

        let status = response.status();
        let response_text = response.text()?;

        if !status.is_success() {
            return Err(anyhow!(
                "Relayer request failed ({}): {}",
                status,
                response_text
            ));
        }

        let result: RelayerResponse = serde_json::from_str(&response_text)
            .map_err(|e| anyhow!("Failed to parse response: {} - {}", e, response_text))?;

        Ok(result)
    }

    /// Wait for a transaction to be confirmed
    pub fn wait_for_confirmation(&self, tx_id: &str, max_attempts: u32) -> Result<RelayerResponse> {
        let path = format!("/transaction/{}", tx_id);
        let url = format!("{}{}", RELAYER_URL, path);

        for attempt in 0..max_attempts {
            std::thread::sleep(Duration::from_secs(2));

            let headers = self.auth_headers("GET", &path, None)?;
            let response = self.http.get(&url).headers(headers).send()?;

            if response.status().is_success() {
                let result: RelayerResponse = response.json()?;
                match result.state.as_str() {
                    "STATE_CONFIRMED" | "STATE_MINED" => return Ok(result),
                    "STATE_FAILED" | "STATE_INVALID" => {
                        return Err(anyhow!(
                            "Transaction failed: {:?}",
                            result.error.unwrap_or_else(|| "Unknown error".to_string())
                        ));
                    }
                    _ => {
                        // Still pending, continue waiting
                        if attempt % 5 == 0 {
                            println!("  Waiting for confirmation... (state: {})", result.state);
                        }
                    }
                }
            }
        }

        Err(anyhow!(
            "Transaction not confirmed after {} attempts",
            max_attempts
        ))
    }

    /// Redeem a winning position
    ///
    /// # Arguments
    /// * `condition_id` - The market's condition ID (with or without 0x prefix)
    /// * `outcome_index` - 0 for Yes, 1 for No (or other outcome indices for multi-outcome markets)
    pub fn redeem_position(&self, condition_id: &str, outcome_index: u32) -> Result<RelayerResponse> {
        let tx = build_redeem_transaction(condition_id, outcome_index)?;
        let description = format!("Redeem position outcome {} for {}", outcome_index, condition_id);
        self.execute(vec![tx], &description)
    }

    /// Redeem multiple positions in a single transaction
    pub fn redeem_positions_batch(
        &self,
        positions: &[(String, u32)], // (condition_id, outcome_index)
    ) -> Result<RelayerResponse> {
        let transactions: Result<Vec<_>> = positions
            .iter()
            .map(|(cid, idx)| build_redeem_transaction(cid, *idx))
            .collect();

        let description = format!("Batch redeem {} positions", positions.len());
        self.execute(transactions?, &description)
    }
}

/// Build a redeem transaction for the CTF contract
pub fn build_redeem_transaction(condition_id: &str, outcome_index: u32) -> Result<RelayerTransaction> {
    // Normalize condition_id (ensure 0x prefix, lowercase)
    let condition_id = if condition_id.starts_with("0x") {
        condition_id.to_lowercase()
    } else {
        format!("0x{}", condition_id.to_lowercase())
    };

    // Validate condition_id length (should be 66 chars: 0x + 64 hex)
    if condition_id.len() != 66 {
        return Err(anyhow!(
            "Invalid condition_id length: {} (expected 66)",
            condition_id.len()
        ));
    }

    // indexSet: 1 << outcome_index
    // For binary markets: 1 for Yes (index 0), 2 for No (index 1)
    let index_set = 1u64 << outcome_index;

    // Build calldata for redeemPositions(address, bytes32, bytes32, uint256[])
    // Function selector: 0x01a9505f (first 4 bytes of keccak256("redeemPositions(address,bytes32,bytes32,uint256[])"))
    let data = encode_redeem_positions(USDC_ADDRESS, &condition_id, index_set)?;

    Ok(RelayerTransaction {
        to: CTF_CONTRACT.to_string(),
        data,
        value: "0".to_string(),
    })
}

/// Encode the redeemPositions function call
fn encode_redeem_positions(
    collateral_token: &str,
    condition_id: &str,
    index_set: u64,
) -> Result<String> {
    // Function selector for redeemPositions(address,bytes32,bytes32,uint256[])
    let selector = "0e6d1de9"; // keccak256("redeemPositions(address,bytes32,bytes32,uint256[])")[:4]

    // Pad address to 32 bytes (remove 0x, pad left with zeros)
    let collateral_padded = format!("{:0>64}", &collateral_token[2..]);

    // Parent collection ID is always zero for Polymarket
    let parent_collection_id = "0".repeat(64);

    // Condition ID (remove 0x prefix)
    let condition_id_hex = &condition_id[2..];

    // For dynamic array (uint256[]), we need:
    // - Offset to array data (0x80 = 128, which is after the 4 fixed params)
    // - Array length
    // - Array elements
    let array_offset = format!("{:0>64}", "80"); // Offset in hex
    let array_length = format!("{:0>64}", "1"); // 1 element
    let index_set_hex = format!("{:0>64x}", index_set);

    let data = format!(
        "0x{}{}{}{}{}{}{}",
        selector,
        collateral_padded,
        parent_collection_id,
        condition_id_hex,
        array_offset,
        array_length,
        index_set_hex
    );

    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_redeem_positions() {
        let result = encode_redeem_positions(
            "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174",
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            1,
        );
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(data.starts_with("0x0e6d1de9")); // Function selector
    }

    #[test]
    fn test_build_redeem_transaction() {
        let tx = build_redeem_transaction(
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            0,
        );
        assert!(tx.is_ok());
        let tx = tx.unwrap();
        assert_eq!(tx.to, CTF_CONTRACT);
        assert_eq!(tx.value, "0");
    }

    #[test]
    fn test_index_set_calculation() {
        // Yes (index 0) -> indexSet = 1
        assert_eq!(1u64 << 0, 1);
        // No (index 1) -> indexSet = 2
        assert_eq!(1u64 << 1, 2);
    }
}
