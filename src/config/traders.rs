/// Trader configuration structures and parsing
/// Provides functionality to load and validate trader addresses

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};

/// JSON representation of trader configuration for file parsing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraderConfigJson {
    pub address: String,
    #[serde(default = "default_label")]
    pub label: String,
    #[serde(default = "default_scaling_ratio")]
    pub scaling_ratio: f64,
    #[serde(default)]
    pub min_shares: f64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_label() -> String {
    "Trader".to_string()
}

fn default_scaling_ratio() -> f64 {
    0.02
}

fn default_enabled() -> bool {
    true
}

/// Configuration for a single trader to monitor
#[derive(Debug, Clone)]
pub struct TraderConfig {
    /// Normalized 40-character hex address (no 0x prefix, lowercase)
    pub address: String,
    /// Human-friendly label for this trader (e.g., "Whale1", "TopTrader")
    pub label: String,
    /// Zero-padded 64-character hex for WebSocket topic filtering
    pub topic_hex: String,
    /// Per-trader scaling ratio for position sizing (default: 0.02)
    pub scaling_ratio: f64,
    /// Minimum whale shares required to copy this trader's trades (default: 0.0)
    pub min_shares: f64,
    /// Whether this trader is enabled for monitoring (default: true)
    pub enabled: bool,
}

impl TraderConfig {
    /// Creates a new TraderConfig with validated address
    ///
    /// # Arguments
    /// * `address` - Ethereum address (may include 0x prefix, any case)
    /// * `label` - Human-friendly name for this trader
    ///
    /// # Returns
    /// * `Ok(TraderConfig)` - Valid trader configuration with defaults
    /// * `Err(String)` - Error message if address validation fails
    pub fn new(address: &str, label: &str) -> Result<Self, String> {
        let normalized_address = validate_and_normalize_address(address)?;
        let topic_hex = address_to_topic_hex(&normalized_address);

        Ok(Self {
            address: normalized_address,
            label: label.to_string(),
            topic_hex,
            scaling_ratio: 0.02,
            min_shares: 0.0,
            enabled: true,
        })
    }
}

/// Validates and normalizes an Ethereum address
///
/// # Arguments
/// * `input` - The address string to validate (may include 0x prefix, whitespace, mixed case)
///
/// # Returns
/// * `Ok(String)` - Normalized lowercase address without 0x prefix (40 hex chars)
/// * `Err(String)` - Error message describing validation failure
///
/// # Examples
/// ```
/// use pm_whale_follower::config::traders::validate_and_normalize_address;
///
/// let addr = validate_and_normalize_address("0xABC123def456789012345678901234567890abcd").unwrap();
/// assert_eq!(addr.len(), 40);
/// assert_eq!(addr, "abc123def456789012345678901234567890abcd");
/// ```
pub fn validate_and_normalize_address(input: &str) -> Result<String, String> {
    // Step 1: Trim whitespace
    let trimmed = input.trim();

    // Step 2: Strip 0x prefix if present (case-insensitive)
    let without_prefix = if trimmed.len() >= 2 {
        let prefix = &trimmed[..2];
        if prefix.eq_ignore_ascii_case("0x") {
            &trimmed[2..]
        } else {
            trimmed
        }
    } else {
        trimmed
    };

    // Step 3: Validate length (must be exactly 40 characters)
    if without_prefix.len() != 40 {
        return Err(format!(
            "Address must be exactly 40 characters (found {}). Addresses are 40 hex characters without 0x prefix.",
            without_prefix.len()
        ));
    }

    // Step 4: Validate all characters are hexadecimal
    if !without_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(
            "Address contains invalid characters. Must be hexadecimal (0-9, a-f, A-F).".to_string()
        );
    }

    // Step 5: Normalize to lowercase
    Ok(without_prefix.to_lowercase())
}

/// Converts a normalized 40-character address to a 64-character topic hex for WebSocket filtering
///
/// # Arguments
/// * `address` - A normalized 40-character hex address (no 0x prefix, lowercase)
///
/// # Returns
/// A 64-character hex string with 24 leading zeros followed by the address
///
/// # Examples
/// ```
/// use pm_whale_follower::config::traders::address_to_topic_hex;
///
/// let topic = address_to_topic_hex("abc123def456789012345678901234567890abcd");
/// assert_eq!(topic.len(), 64);
/// assert_eq!(topic, "000000000000000000000000abc123def456789012345678901234567890abcd");
/// ```
pub fn address_to_topic_hex(address: &str) -> String {
    // Pad address to 64 characters with leading zeros
    // 64 total - 40 address = 24 zeros needed
    format!("{:0>64}", address)
}

/// Configuration for multiple traders to monitor
#[derive(Debug, Clone)]
pub struct TradersConfig {
    traders: Vec<TraderConfig>,
    topic_map: HashMap<String, usize>, // topic_hex -> index in traders vec
    address_map: HashMap<String, usize>, // address -> index in traders vec
}

impl TradersConfig {
    /// Creates a new TradersConfig from a vector of TraderConfig
    pub fn new(traders: Vec<TraderConfig>) -> Self {
        let mut topic_map = HashMap::new();
        let mut address_map = HashMap::new();

        for (idx, trader) in traders.iter().enumerate() {
            topic_map.insert(trader.topic_hex.clone(), idx);
            address_map.insert(trader.address.clone(), idx);
        }

        Self {
            traders,
            topic_map,
            address_map,
        }
    }

    /// Returns the number of configured traders
    pub fn len(&self) -> usize {
        self.traders.len()
    }

    /// Returns true if no traders are configured
    pub fn is_empty(&self) -> bool {
        self.traders.is_empty()
    }

    /// Builds a vector of topic hex strings for WebSocket subscription filtering
    /// Only includes enabled traders
    pub fn build_topic_filter(&self) -> Vec<String> {
        self.traders
            .iter()
            .filter(|t| t.enabled)
            .map(|t| t.topic_hex.clone())
            .collect()
    }

    /// Looks up a trader by topic hex (fast O(1) lookup)
    pub fn get_by_topic(&self, topic_hex: &str) -> Option<&TraderConfig> {
        self.topic_map
            .get(topic_hex)
            .and_then(|&idx| self.traders.get(idx))
    }

    /// Looks up a trader by address (supports 0x prefix)
    pub fn get_by_address(&self, address: &str) -> Option<&TraderConfig> {
        // Normalize the address for lookup
        let normalized = validate_and_normalize_address(address).ok()?;
        self.address_map
            .get(&normalized)
            .and_then(|&idx| self.traders.get(idx))
    }

    /// Returns an iterator over all traders
    pub fn iter(&self) -> impl Iterator<Item = &TraderConfig> {
        self.traders.iter()
    }

    /// Parses trader addresses from TRADER_ADDRESSES environment variable
    ///
    /// Format: comma-separated addresses (with optional 0x prefix)
    /// Example: "0xabc123...,0xdef456..."
    ///
    /// # Returns
    /// * `Ok(TradersConfig)` - Parsed configuration
    /// * `Err(String)` - Error message if env var is missing or addresses are invalid
    pub fn from_env() -> Result<Self, String> {
        let addresses_str = env::var("TRADER_ADDRESSES")
            .map_err(|_| "TRADER_ADDRESSES environment variable not set".to_string())?;

        if addresses_str.trim().is_empty() {
            return Err("TRADER_ADDRESSES is empty. Provide comma-separated addresses.".to_string());
        }

        // Parse comma-separated addresses
        let addresses: Vec<&str> = addresses_str.split(',').map(|s| s.trim()).collect();

        // Deduplicate addresses using a HashSet
        let mut seen = HashSet::new();
        let mut traders = Vec::new();

        for (idx, addr) in addresses.iter().enumerate() {
            if addr.is_empty() {
                continue; // Skip empty entries from trailing commas
            }

            // Validate and normalize address
            let normalized = validate_and_normalize_address(addr)
                .map_err(|e| format!("Invalid address at position {}: {} - {}", idx + 1, addr, e))?;

            // Skip duplicates
            if !seen.insert(normalized.clone()) {
                continue;
            }

            // Create trader config with auto-generated label
            let label = format!("Trader{}", traders.len() + 1);
            let config = TraderConfig::new(&normalized, &label)?;
            traders.push(config);
        }

        if traders.is_empty() {
            return Err("No valid trader addresses found in TRADER_ADDRESSES".to_string());
        }

        Ok(Self::new(traders))
    }

    /// Loads trader configuration with fallback chain:
    /// 1. Try traders.json file (highest priority)
    /// 2. Try TRADER_ADDRESSES env var (multi-trader format)
    /// 3. Fall back to TARGET_WHALE_ADDRESS (legacy single trader)
    /// 4. Error if none found
    ///
    /// # Returns
    /// * `Ok(TradersConfig)` - Loaded configuration
    /// * `Err(String)` - Error if no valid configuration source found
    pub fn load() -> Result<Self, String> {
        // 1. Try traders.json file first (highest priority)
        if Path::new("traders.json").exists() {
            return Self::from_file("traders.json");
        }

        // 2. Try TRADER_ADDRESSES env var
        if env::var("TRADER_ADDRESSES").is_ok() {
            return Self::from_env();
        }

        // 3. Fall back to legacy TARGET_WHALE_ADDRESS
        if let Ok(legacy_address) = env::var("TARGET_WHALE_ADDRESS") {
            let normalized = validate_and_normalize_address(&legacy_address)
                .map_err(|e| format!("Invalid TARGET_WHALE_ADDRESS: {}", e))?;

            let config = TraderConfig::new(&normalized, "Legacy")?;
            return Ok(Self::new(vec![config]));
        }

        Err(
            "No trader configuration found. Create traders.json or set TRADER_ADDRESSES/TARGET_WHALE_ADDRESS environment variable.".to_string()
        )
    }

    /// Loads trader configuration from a JSON file
    ///
    /// Expected JSON format:
    /// ```json
    /// [
    ///   {
    ///     "address": "abc123...",
    ///     "label": "Whale1",
    ///     "scaling_ratio": 0.02,
    ///     "min_shares": 100.0,
    ///     "enabled": true
    ///   }
    /// ]
    /// ```
    ///
    /// # Arguments
    /// * `path` - Path to the JSON file
    ///
    /// # Returns
    /// * `Ok(TradersConfig)` - Loaded configuration
    /// * `Err(String)` - Error if file cannot be read or parsed
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path = path.as_ref();

        // Read file contents
        let contents = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read file {}: {}", path.display(), e))?;

        // Parse JSON
        let json_configs: Vec<TraderConfigJson> = serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;

        if json_configs.is_empty() {
            return Err("JSON file contains no trader configurations".to_string());
        }

        // Convert JSON configs to TraderConfig, validating addresses and deduplicating
        let mut seen = HashSet::new();
        let mut traders = Vec::new();

        for (idx, json_config) in json_configs.iter().enumerate() {
            // Validate and normalize address
            let normalized = validate_and_normalize_address(&json_config.address)
                .map_err(|e| format!("Invalid address at index {}: {} - {}", idx, json_config.address, e))?;

            // Skip duplicates
            if !seen.insert(normalized.clone()) {
                continue;
            }

            // Build TraderConfig
            let mut config = TraderConfig::new(&normalized, &json_config.label)?;
            config.scaling_ratio = json_config.scaling_ratio;
            config.min_shares = json_config.min_shares;
            config.enabled = json_config.enabled;

            traders.push(config);
        }

        if traders.is_empty() {
            return Err("No valid trader configurations after deduplication".to_string());
        }

        Ok(Self::new(traders))
    }
}
