/// Portfolio value tracking for dynamic bet sizing
///
/// This module provides portfolio value calculation and caching to support
/// dynamic bet size limits based on total portfolio value (USDC + positions).
///
/// # Example
///
/// ```no_run
/// use pm_whale_follower::portfolio::{PortfolioTracker, PortfolioConfig};
///
/// let config = PortfolioConfig {
///     wallet_address: "0x1234...".to_string(),
///     cache_duration_secs: 300, // 5 minutes
///     max_bet_portfolio_percent: Some(0.02), // 2% max bet
/// };
///
/// let tracker = PortfolioTracker::new(config);
///
/// // Get max bet in USD based on portfolio value
/// if let Some(max_bet_usd) = tracker.get_max_bet_usd() {
///     println!("Max bet: ${:.2}", max_bet_usd);
/// }
/// ```

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::live_positions::{fetch_live_positions, LivePositionsSummary};

// Polygon USDC contracts
const USDC_POLYGON: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174"; // USDC.e (bridged)
const USDC_NATIVE: &str = "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"; // Native USDC
const POLYGON_RPC: &str = "https://polygon-rpc.com";

/// Configuration for portfolio tracking
#[derive(Debug, Clone)]
pub struct PortfolioConfig {
    /// Wallet address to track
    pub wallet_address: String,
    /// How long to cache portfolio value (seconds)
    pub cache_duration_secs: u64,
    /// Maximum bet as percentage of portfolio (e.g., 0.02 = 2%)
    /// None means no portfolio-based limit
    pub max_bet_portfolio_percent: Option<f64>,
}

impl Default for PortfolioConfig {
    fn default() -> Self {
        Self {
            wallet_address: String::new(),
            cache_duration_secs: 300, // 5 minutes
            max_bet_portfolio_percent: None, // Disabled by default
        }
    }
}

/// Cached portfolio value
#[derive(Debug, Clone)]
struct CachedPortfolio {
    /// Total portfolio value in USD
    total_value_usd: f64,
    /// USDC balance component
    usdc_balance: f64,
    /// Positions value component
    positions_value: f64,
    /// When this value was fetched
    fetched_at: Instant,
}

/// Portfolio tracker with caching
#[derive(Debug)]
pub struct PortfolioTracker {
    config: PortfolioConfig,
    cache: Arc<RwLock<Option<CachedPortfolio>>>,
    http_client: Client,
}

impl PortfolioTracker {
    /// Create a new portfolio tracker
    pub fn new(config: PortfolioConfig) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            cache: Arc::new(RwLock::new(None)),
            http_client,
        }
    }

    /// Get the maximum bet size in USD based on portfolio value
    /// Returns None if portfolio-based limiting is disabled or if fetch fails
    pub fn get_max_bet_usd(&self) -> Option<f64> {
        let percent = self.config.max_bet_portfolio_percent?;

        let portfolio_value = self.get_portfolio_value().ok()?;

        Some(portfolio_value * percent)
    }

    /// Get the maximum bet size in shares based on portfolio value and price
    /// Returns None if portfolio-based limiting is disabled or if fetch fails
    pub fn get_max_bet_shares(&self, price: f64) -> Option<f64> {
        let max_usd = self.get_max_bet_usd()?;
        let safe_price = price.max(0.01);
        Some(max_usd / safe_price)
    }

    /// Get current portfolio value, using cache if valid
    pub fn get_portfolio_value(&self) -> Result<f64> {
        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(ref cached) = *cache {
                let age = cached.fetched_at.elapsed();
                if age < Duration::from_secs(self.config.cache_duration_secs) {
                    return Ok(cached.total_value_usd);
                }
            }
        }

        // Cache miss or expired - fetch fresh value
        self.refresh_portfolio_value()
    }

    /// Force refresh portfolio value
    pub fn refresh_portfolio_value(&self) -> Result<f64> {
        let usdc_balance = self.fetch_usdc_balance()?;
        let positions_value = self.fetch_positions_value()?;
        let total_value = usdc_balance + positions_value;

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            *cache = Some(CachedPortfolio {
                total_value_usd: total_value,
                usdc_balance,
                positions_value,
                fetched_at: Instant::now(),
            });
        }

        Ok(total_value)
    }

    /// Get detailed portfolio breakdown (fetches fresh if cache expired)
    pub fn get_portfolio_details(&self) -> Result<PortfolioDetails> {
        // Ensure cache is fresh
        let _ = self.get_portfolio_value()?;

        let cache = self.cache.read().unwrap();
        let cached = cache.as_ref().context("No cached portfolio value")?;

        Ok(PortfolioDetails {
            total_value_usd: cached.total_value_usd,
            usdc_balance: cached.usdc_balance,
            positions_value: cached.positions_value,
            max_bet_usd: self.config.max_bet_portfolio_percent
                .map(|p| cached.total_value_usd * p),
            max_bet_percent: self.config.max_bet_portfolio_percent,
            cache_age_secs: cached.fetched_at.elapsed().as_secs(),
        })
    }

    /// Fetch USDC balance from blockchain
    fn fetch_usdc_balance(&self) -> Result<f64> {
        let usdc_e = self.get_token_balance(USDC_POLYGON)?;
        let usdc_native = self.get_token_balance(USDC_NATIVE)?;

        // Both have 6 decimals
        let total = (usdc_e + usdc_native) as f64 / 1_000_000.0;
        Ok(total)
    }

    /// Fetch positions value from Polymarket Data API
    fn fetch_positions_value(&self) -> Result<f64> {
        let positions = fetch_live_positions(&self.config.wallet_address)?;
        let summary = LivePositionsSummary::from_positions(&positions);
        Ok(summary.total_value)
    }

    /// Get ERC20 token balance via RPC
    fn get_token_balance(&self, token: &str) -> Result<u128> {
        let owner = &self.config.wallet_address;
        let owner_padded = format!("{:0>64}", owner.trim_start_matches("0x").to_lowercase());
        let data = format!("0x70a08231{}", owner_padded);

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method: "eth_call",
            params: vec![
                serde_json::json!({
                    "to": token,
                    "data": data
                }),
                serde_json::json!("latest"),
            ],
            id: 1,
        };

        let response: JsonRpcResponse = self.http_client
            .post(POLYGON_RPC)
            .json(&request)
            .send()
            .context("Failed to send RPC request")?
            .json()
            .context("Failed to parse RPC response")?;

        if let Some(error) = response.error {
            anyhow::bail!("RPC error: {:?}", error);
        }

        let hex = response.result.unwrap_or_else(|| "0x0".to_string());
        let hex_clean = hex.trim_start_matches("0x");
        if hex_clean.is_empty() || hex_clean == "0" {
            return Ok(0);
        }

        Ok(u128::from_str_radix(hex_clean, 16).unwrap_or(0))
    }
}

/// Detailed portfolio breakdown
#[derive(Debug, Clone, Serialize)]
pub struct PortfolioDetails {
    /// Total portfolio value in USD
    pub total_value_usd: f64,
    /// USDC balance component
    pub usdc_balance: f64,
    /// Positions value component
    pub positions_value: f64,
    /// Maximum bet size in USD (if limit enabled)
    pub max_bet_usd: Option<f64>,
    /// Maximum bet as percentage of portfolio
    pub max_bet_percent: Option<f64>,
    /// How old the cached value is (seconds)
    pub cache_age_secs: u64,
}

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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_portfolio_config_default() {
        let config = PortfolioConfig::default();
        assert_eq!(config.cache_duration_secs, 300);
        assert!(config.max_bet_portfolio_percent.is_none());
    }

    #[test]
    fn test_max_bet_calculation() {
        // Test the math: 2% of $1000 = $20
        let portfolio_value: f64 = 1000.0;
        let percent: f64 = 0.02;
        let max_bet: f64 = portfolio_value * percent;
        assert!((max_bet - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_max_bet_shares_calculation() {
        // $20 max bet at $0.50 price = 40 shares
        let max_bet_usd: f64 = 20.0;
        let price: f64 = 0.50;
        let max_shares: f64 = max_bet_usd / price;
        assert!((max_shares - 40.0).abs() < 0.001);
    }

    #[test]
    fn test_portfolio_config_with_limit() {
        let config = PortfolioConfig {
            wallet_address: "0x1234".to_string(),
            cache_duration_secs: 60,
            max_bet_portfolio_percent: Some(0.05), // 5%
        };
        assert_eq!(config.max_bet_portfolio_percent, Some(0.05));
    }

    #[test]
    fn test_disabled_returns_none() {
        let config = PortfolioConfig {
            wallet_address: String::new(),
            cache_duration_secs: 300,
            max_bet_portfolio_percent: None, // Disabled
        };
        let tracker = PortfolioTracker::new(config);

        // Should return None when disabled
        assert!(tracker.get_max_bet_usd().is_none());
    }
}
