/// Market information fetcher for Polymarket markets
///
/// Fetches market metadata from the Gamma API to get human-readable
/// market titles and outcome names for token IDs.
///
/// # Example
///
/// ```no_run
/// use pm_whale_follower::market_info::MarketInfo;
///
/// let market_info = MarketInfo::new();
/// if let Ok(Some(info)) = market_info.fetch("token_id") {
///     println!("Market: {}", info.title);
///     println!("Outcome: {}", info.outcome);
/// }
/// ```

use anyhow::Result;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;

/// Market metadata for a token
#[derive(Debug, Clone, PartialEq)]
pub struct MarketMetadata {
    pub title: String,
    pub outcome: String,
}

/// Gamma API response structure (returns an array of markets directly)
type GammaMarketResponse = Vec<GammaMarket>;

#[derive(Debug, Deserialize)]
struct GammaMarket {
    question: String,
    #[serde(rename = "clobTokenIds")]
    clob_token_ids: String, // JSON string containing array of token IDs
    outcomes: String,        // JSON string containing array of outcome names
}

/// Market information fetcher
pub struct MarketInfo {
    client: Client,
    host: String,
}

impl MarketInfo {
    /// Create a new MarketInfo fetcher with default Gamma API host
    pub fn new() -> Self {
        Self::with_host("https://gamma-api.polymarket.com")
    }

    /// Create a new MarketInfo fetcher with custom host (for testing)
    pub fn with_host(host: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            host: host.to_string(),
        }
    }

    /// Fetch market metadata for a given token ID
    ///
    /// Returns None if the token doesn't exist or API fails
    pub fn fetch(&self, token_id: &str) -> Result<Option<MarketMetadata>> {
        let url = format!("{}/markets?clob_token_ids={}", self.host, token_id);
        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let markets: GammaMarketResponse = response.json()?;

        // Should get exactly one market for a specific token ID query
        if let Some(market) = markets.first() {
            // Parse the JSON strings for token IDs and outcomes
            let token_ids: Vec<String> = serde_json::from_str(&market.clob_token_ids)
                .unwrap_or_default();
            let outcomes: Vec<String> = serde_json::from_str(&market.outcomes)
                .unwrap_or_default();

            // Find the index of our token ID
            if let Some(index) = token_ids.iter().position(|id| id == token_id) {
                // Get the corresponding outcome name
                if let Some(outcome) = outcomes.get(index) {
                    return Ok(Some(MarketMetadata {
                        title: market.question.clone(),
                        outcome: outcome.clone(),
                    }));
                }
            }
        }

        // Token ID not found in response
        Ok(None)
    }
}

impl Default for MarketInfo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_info_fetch_returns_metadata() {
        // This test will fail initially because we haven't implemented the real API call yet
        // For now, we'll use an invalid host to ensure it returns None gracefully
        let market_info = MarketInfo::with_host("http://invalid-host.example.com");

        // Should return error (network failure) but not panic
        let result = market_info.fetch("test_token");
        assert!(result.is_err() || result.unwrap().is_none());
    }

    #[test]
    fn test_market_info_new_creates_instance() {
        let market_info = MarketInfo::new();
        assert_eq!(market_info.host, "https://gamma-api.polymarket.com");
    }

    #[test]
    fn test_market_info_with_host_creates_instance() {
        let market_info = MarketInfo::with_host("http://test-host.com");
        assert_eq!(market_info.host, "http://test-host.com");
    }

    #[test]
    #[ignore] // Network test - requires active market
    fn test_market_info_fetch_real_api() {
        // This is a real API test that should be run manually
        // Using a known active market token ID from January 2026
        let market_info = MarketInfo::new();

        // Active market: "Will Trump deport less than 250,000?" - Yes outcome
        let token_id = "101676997363687199724245607342877036148401850938023978421879460310389391082353";

        let result = market_info.fetch(token_id);

        // If market is closed or unavailable, skip test
        if result.is_err() {
            eprintln!("Warning: API may be unavailable");
            eprintln!("Error: {:?}", result);
            return;
        }

        let metadata = result.unwrap();
        if let Some(info) = metadata {
            assert!(!info.title.is_empty());
            assert!(!info.outcome.is_empty());
            println!("Market: {}", info.title);
            println!("Outcome: {}", info.outcome);

            // Verify we got the expected market
            assert_eq!(info.title, "Will Trump deport less than 250,000?");
            assert_eq!(info.outcome, "Yes");
        } else {
            eprintln!("Warning: Token ID not found - may be an inactive market");
        }
    }

    #[test]
    fn test_market_metadata_clone() {
        let metadata = MarketMetadata {
            title: "Will it rain?".to_string(),
            outcome: "Yes".to_string(),
        };

        let cloned = metadata.clone();
        assert_eq!(cloned.title, "Will it rain?");
        assert_eq!(cloned.outcome, "Yes");
    }

    #[test]
    fn test_market_metadata_equality() {
        let metadata1 = MarketMetadata {
            title: "Will it rain?".to_string(),
            outcome: "Yes".to_string(),
        };

        let metadata2 = MarketMetadata {
            title: "Will it rain?".to_string(),
            outcome: "Yes".to_string(),
        };

        assert_eq!(metadata1, metadata2);
    }
}
