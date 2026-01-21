/// Live positions fetcher module for Polymarket Data API
/// Fetches actual positions held on-chain/CLOB for a wallet address
///
/// # Features
///
/// - **Live Data**: Fetches real positions from Polymarket Data API
/// - **Rich Details**: Includes P&L, current prices, market info
/// - **No Auth Required**: Data API is public (no L2 authentication needed)
///
/// # Example
///
/// ```no_run
/// use pm_whale_follower::live_positions::{fetch_live_positions, LivePosition};
///
/// let address = "0x56687bf447db6ffa42ffe2204a05edaa20f55839";
/// match fetch_live_positions(address) {
///     Ok(positions) => {
///         println!("Found {} live positions", positions.len());
///         for pos in positions {
///             println!("  {}: {} shares @ ${:.2}", pos.title, pos.size, pos.cur_price);
///         }
///     }
///     Err(e) => eprintln!("Failed to fetch positions: {}", e),
/// }
/// ```

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Base URL for Polymarket Data API
const DATA_API_BASE: &str = "https://data-api.polymarket.com";

/// Live position from Polymarket Data API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePosition {
    /// Proxy wallet address holding the position
    #[serde(default)]
    pub proxy_wallet: String,

    /// Asset type (usually "USDC")
    #[serde(default)]
    pub asset: String,

    /// Condition ID (market identifier)
    #[serde(default)]
    pub condition_id: String,

    /// Number of shares held
    #[serde(default)]
    pub size: f64,

    /// Average entry price
    #[serde(default)]
    pub avg_price: f64,

    /// Initial value when position was opened
    #[serde(default)]
    pub initial_value: f64,

    /// Current value of the position
    #[serde(default)]
    pub current_value: f64,

    /// Cash P&L (current_value - initial_value)
    #[serde(default)]
    pub cash_pnl: f64,

    /// Percentage P&L
    #[serde(default)]
    pub percent_pnl: f64,

    /// Total amount bought (cost basis)
    #[serde(default)]
    pub total_bought: f64,

    /// Realized P&L from closed portions
    #[serde(default)]
    pub realized_pnl: f64,

    /// Percentage realized P&L
    #[serde(default)]
    pub percent_realized_pnl: f64,

    /// Current market price
    #[serde(default)]
    pub cur_price: f64,

    /// Whether position can be redeemed (resolved market)
    #[serde(default)]
    pub redeemable: bool,

    /// Whether position can be merged
    #[serde(default)]
    pub mergeable: bool,

    /// Market title
    #[serde(default)]
    pub title: String,

    /// Market slug (URL-friendly name)
    #[serde(default)]
    pub slug: String,

    /// Market icon URL
    #[serde(default)]
    pub icon: String,

    /// Event slug
    #[serde(default)]
    pub event_slug: String,

    /// Outcome name (e.g., "Yes", "No")
    #[serde(default)]
    pub outcome: String,

    /// Outcome index (0 or 1 for binary markets)
    #[serde(default)]
    pub outcome_index: i32,
}

/// Options for fetching live positions
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// Minimum size threshold (default: 0.01)
    pub size_threshold: Option<f64>,
    /// Maximum number of positions to return (default: 100, max: 500)
    pub limit: Option<u32>,
    /// Number of positions to skip
    pub offset: Option<u32>,
    /// Include redeemable positions only
    pub redeemable: Option<bool>,
}

impl FetchOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit.min(500));
        self
    }

    pub fn with_size_threshold(mut self, threshold: f64) -> Self {
        self.size_threshold = Some(threshold);
        self
    }
}

/// Fetches live positions for a wallet address from Polymarket Data API
///
/// # Arguments
/// * `address` - Wallet address (with or without 0x prefix)
///
/// # Returns
/// * `Ok(Vec<LivePosition>)` - List of live positions
/// * `Err` - If API request fails
pub fn fetch_live_positions(address: &str) -> Result<Vec<LivePosition>> {
    fetch_live_positions_with_options(address, &FetchOptions::default())
}

/// Fetches live positions with custom options
///
/// # Arguments
/// * `address` - Wallet address (with or without 0x prefix)
/// * `options` - Fetch options (limit, threshold, etc.)
///
/// # Returns
/// * `Ok(Vec<LivePosition>)` - List of live positions
/// * `Err` - If API request fails
pub fn fetch_live_positions_with_options(
    address: &str,
    options: &FetchOptions,
) -> Result<Vec<LivePosition>> {
    // Normalize address (ensure 0x prefix)
    let normalized_address = normalize_address(address)?;

    // Build URL with query parameters
    let mut url = format!("{}/positions?user={}", DATA_API_BASE, normalized_address);

    if let Some(threshold) = options.size_threshold {
        url.push_str(&format!("&sizeThreshold={}", threshold));
    }
    if let Some(limit) = options.limit {
        url.push_str(&format!("&limit={}", limit));
    }
    if let Some(offset) = options.offset {
        url.push_str(&format!("&offset={}", offset));
    }
    if let Some(redeemable) = options.redeemable {
        url.push_str(&format!("&redeemable={}", redeemable));
    }

    // Create HTTP client with timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    // Make request
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .context("Failed to send request to Data API")?;

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().unwrap_or_default();
        return Err(anyhow!(
            "Data API returned error {}: {}",
            status,
            error_body
        ));
    }

    // Parse response
    let positions: Vec<LivePosition> = response
        .json()
        .context("Failed to parse positions response")?;

    Ok(positions)
}

/// Normalizes an address to have 0x prefix
fn normalize_address(address: &str) -> Result<String> {
    let trimmed = address.trim();

    // Handle 0x prefix
    let without_prefix = if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        &trimmed[2..]
    } else {
        trimmed
    };

    // Validate length
    if without_prefix.len() != 40 {
        return Err(anyhow!(
            "Invalid address length: {} (expected 40 hex chars)",
            without_prefix.len()
        ));
    }

    // Validate hex
    if !without_prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!("Address contains non-hex characters"));
    }

    Ok(format!("0x{}", without_prefix.to_lowercase()))
}

/// Summary statistics for live positions
#[derive(Debug, Clone, Default)]
pub struct LivePositionsSummary {
    /// Total number of positions
    pub position_count: usize,
    /// Total current value across all positions
    pub total_value: f64,
    /// Total unrealized P&L
    pub total_pnl: f64,
    /// Total realized P&L
    pub total_realized_pnl: f64,
    /// Number of profitable positions
    pub profitable_count: usize,
    /// Number of losing positions
    pub losing_count: usize,
}

impl LivePositionsSummary {
    /// Calculate summary from a list of positions
    pub fn from_positions(positions: &[LivePosition]) -> Self {
        let mut summary = Self::default();

        summary.position_count = positions.len();

        for pos in positions {
            summary.total_value += pos.current_value;
            summary.total_pnl += pos.cash_pnl;
            summary.total_realized_pnl += pos.realized_pnl;

            if pos.cash_pnl > 0.0 {
                summary.profitable_count += 1;
            } else if pos.cash_pnl < 0.0 {
                summary.losing_count += 1;
            }
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_address_with_0x() {
        let result = normalize_address("0xabc123def456789012345678901234567890abcd");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0xabc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_normalize_address_without_0x() {
        let result = normalize_address("abc123def456789012345678901234567890abcd");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0xabc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_normalize_address_uppercase() {
        let result = normalize_address("0xABC123DEF456789012345678901234567890ABCD");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "0xabc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_normalize_address_too_short() {
        let result = normalize_address("0xabc123");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("length"));
    }

    #[test]
    fn test_normalize_address_invalid_chars() {
        let result = normalize_address("0xabc123def456789012345678901234567890abcg");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("non-hex"));
    }

    #[test]
    fn test_fetch_options_builder() {
        let options = FetchOptions::new()
            .with_limit(50)
            .with_size_threshold(0.1);

        assert_eq!(options.limit, Some(50));
        assert_eq!(options.size_threshold, Some(0.1));
    }

    #[test]
    fn test_fetch_options_limit_capped() {
        let options = FetchOptions::new().with_limit(1000);
        assert_eq!(options.limit, Some(500)); // Should be capped at 500
    }

    #[test]
    fn test_live_position_deserialization() {
        let json = r#"{
            "proxyWallet": "0x123",
            "asset": "USDC",
            "conditionId": "0xabc",
            "size": 100.5,
            "avgPrice": 0.45,
            "initialValue": 45.225,
            "currentValue": 50.25,
            "cashPnl": 5.025,
            "percentPnl": 11.11,
            "totalBought": 45.225,
            "realizedPnl": 0.0,
            "percentRealizedPnl": 0.0,
            "curPrice": 0.50,
            "redeemable": false,
            "mergeable": true,
            "title": "Test Market",
            "slug": "test-market",
            "icon": "https://example.com/icon.png",
            "eventSlug": "test-event",
            "outcome": "Yes",
            "outcomeIndex": 1
        }"#;

        let pos: LivePosition = serde_json::from_str(json).unwrap();
        assert_eq!(pos.size, 100.5);
        assert_eq!(pos.avg_price, 0.45);
        assert_eq!(pos.cash_pnl, 5.025);
        assert_eq!(pos.outcome, "Yes");
    }

    #[test]
    fn test_live_position_partial_deserialization() {
        // Test with minimal fields (API might not return all fields)
        let json = r#"{
            "size": 50.0,
            "curPrice": 0.75,
            "title": "Partial Position"
        }"#;

        let pos: LivePosition = serde_json::from_str(json).unwrap();
        assert_eq!(pos.size, 50.0);
        assert_eq!(pos.cur_price, 0.75);
        assert_eq!(pos.title, "Partial Position");
        // Default values for missing fields
        assert_eq!(pos.avg_price, 0.0);
        assert_eq!(pos.cash_pnl, 0.0);
    }

    #[test]
    fn test_summary_from_positions() {
        let positions = vec![
            LivePosition {
                size: 100.0,
                current_value: 50.0,
                cash_pnl: 10.0,
                realized_pnl: 5.0,
                ..Default::default()
            },
            LivePosition {
                size: 200.0,
                current_value: 80.0,
                cash_pnl: -15.0,
                realized_pnl: 0.0,
                ..Default::default()
            },
            LivePosition {
                size: 50.0,
                current_value: 25.0,
                cash_pnl: 0.0,
                realized_pnl: 2.5,
                ..Default::default()
            },
        ];

        let summary = LivePositionsSummary::from_positions(&positions);

        assert_eq!(summary.position_count, 3);
        assert!((summary.total_value - 155.0).abs() < 0.01);
        assert!((summary.total_pnl - (-5.0)).abs() < 0.01);
        assert!((summary.total_realized_pnl - 7.5).abs() < 0.01);
        assert_eq!(summary.profitable_count, 1);
        assert_eq!(summary.losing_count, 1);
    }

    #[test]
    fn test_summary_empty_positions() {
        let positions: Vec<LivePosition> = vec![];
        let summary = LivePositionsSummary::from_positions(&positions);

        assert_eq!(summary.position_count, 0);
        assert_eq!(summary.total_value, 0.0);
        assert_eq!(summary.total_pnl, 0.0);
    }
}

// Implement Default for LivePosition for testing
impl Default for LivePosition {
    fn default() -> Self {
        Self {
            proxy_wallet: String::new(),
            asset: String::new(),
            condition_id: String::new(),
            size: 0.0,
            avg_price: 0.0,
            initial_value: 0.0,
            current_value: 0.0,
            cash_pnl: 0.0,
            percent_pnl: 0.0,
            total_bought: 0.0,
            realized_pnl: 0.0,
            percent_realized_pnl: 0.0,
            cur_price: 0.0,
            redeemable: false,
            mergeable: false,
            title: String::new(),
            slug: String::new(),
            icon: String::new(),
            event_slug: String::new(),
            outcome: String::new(),
            outcome_index: 0,
        }
    }
}
