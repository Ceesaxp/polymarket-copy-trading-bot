/// Price fetcher module for Polymarket CLOB API
/// Provides caching and rate limiting for price data
///
/// # Features
///
/// - **Caching**: Price data is cached with configurable TTL (default: 30 seconds)
/// - **Rate Limiting**: Automatic rate limiting (default: 10 requests/second)
/// - **Batch Fetching**: Fetch multiple token prices efficiently
/// - **Fallback**: Option to use stale cache when API fails
///
/// # Example
///
/// ```no_run
/// use pm_whale_follower::prices::PriceCache;
///
/// // Create cache with 30-second TTL
/// let mut cache = PriceCache::new(30);
///
/// // Fetch a single price (with caching and rate limiting)
/// if let Ok(price) = cache.get_or_fetch_price("token_id_123") {
///     println!("Bid: {}, Ask: {}", price.bid_price, price.ask_price);
/// }
///
/// // Fetch multiple prices in batch
/// let tokens = vec!["token1", "token2", "token3"];
/// let prices = cache.get_or_fetch_prices_batch(&tokens);
/// println!("Fetched {} prices", prices.len());
///
/// // Use fallback to stale cache if API fails
/// if let Some(price) = cache.get_or_fetch_price_with_fallback("token_id") {
///     println!("Price (may be stale): {}", price.bid_price);
/// }
/// ```
///
/// # Custom Configuration
///
/// ```no_run
/// use pm_whale_follower::prices::PriceCache;
///
/// // Custom TTL and rate limit
/// let cache = PriceCache::new(60)  // 60 second TTL
///     .with_rate_limit(5);         // 5 requests/second
/// ```

use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Price information for a token
#[derive(Debug, Clone)]
pub struct PriceInfo {
    pub bid_price: f64,
    pub ask_price: f64,
    pub timestamp: Instant,
}

/// API response for order book
#[derive(Debug, Deserialize)]
struct BookResponse {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
}

#[derive(Debug, Deserialize)]
struct BookLevel {
    price: String,
    size: String,
}

/// Cache for price data with TTL support
pub struct PriceCache {
    ttl_seconds: u64,
    cache: HashMap<String, PriceInfo>,
    client: Client,
    host: String,
    /// Rate limiting: minimum duration between API requests
    min_request_interval: Duration,
    /// Last API request timestamp
    last_request: Option<Instant>,
}

impl PriceCache {
    /// Create a new price cache with specified TTL and default rate limiting (10 req/sec)
    pub fn new(ttl_seconds: u64) -> Self {
        Self::with_host(ttl_seconds, "https://clob.polymarket.com")
    }

    /// Create a new price cache with custom host (for testing)
    pub fn with_host(ttl_seconds: u64, host: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            ttl_seconds,
            cache: HashMap::new(),
            client,
            host: host.to_string(),
            // 10 requests per second = 100ms between requests
            min_request_interval: Duration::from_millis(100),
            last_request: None,
        }
    }

    /// Set custom rate limit (requests per second)
    pub fn with_rate_limit(mut self, requests_per_second: u32) -> Self {
        if requests_per_second > 0 {
            self.min_request_interval = Duration::from_millis(1000 / requests_per_second as u64);
        }
        self
    }

    /// Apply rate limiting - sleeps if needed to maintain rate limit
    fn apply_rate_limit(&mut self) {
        if let Some(last_req) = self.last_request {
            let elapsed = last_req.elapsed();
            if elapsed < self.min_request_interval {
                let sleep_duration = self.min_request_interval - elapsed;
                std::thread::sleep(sleep_duration);
            }
        }
        self.last_request = Some(Instant::now());
    }

    /// Get cached price if still valid (within TTL)
    pub fn get_price(&self, token_id: &str) -> Option<PriceInfo> {
        if let Some(price_info) = self.cache.get(token_id) {
            let age_seconds = price_info.timestamp.elapsed().as_secs();
            if age_seconds < self.ttl_seconds {
                return Some(price_info.clone());
            }
        }
        None
    }

    /// Set price in cache
    fn set_price(&mut self, token_id: String, price_info: PriceInfo) {
        self.cache.insert(token_id, price_info);
    }

    /// Fetch price from CLOB API
    pub fn fetch_price(&mut self, token_id: &str) -> Result<PriceInfo> {
        // Apply rate limiting before making request
        self.apply_rate_limit();

        let url = format!("{}/book?token_id={}", self.host, token_id);
        let response = self.client.get(&url).send()?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to fetch price: HTTP {}",
                response.status()
            ));
        }

        let book: BookResponse = response.json()?;

        // Get best bid (highest buy price) and best ask (lowest sell price)
        let bid_price = book
            .bids
            .first()
            .and_then(|level| level.price.parse::<f64>().ok())
            .unwrap_or(0.0);

        let ask_price = book
            .asks
            .first()
            .and_then(|level| level.price.parse::<f64>().ok())
            .unwrap_or(1.0);

        let price_info = PriceInfo {
            bid_price,
            ask_price,
            timestamp: Instant::now(),
        };

        // Update cache with fresh data
        self.set_price(token_id.to_string(), price_info.clone());

        Ok(price_info)
    }

    /// Get price from cache if valid, otherwise fetch from API
    pub fn get_or_fetch_price(&mut self, token_id: &str) -> Result<PriceInfo> {
        // Try cache first
        if let Some(cached) = self.get_price(token_id) {
            return Ok(cached);
        }

        // Cache miss or expired - fetch from API
        self.fetch_price(token_id)
    }

    /// Get price from cache if valid, otherwise fetch from API
    /// Falls back to stale cache if API fails
    pub fn get_or_fetch_price_with_fallback(&mut self, token_id: &str) -> Option<PriceInfo> {
        // Try cache first (within TTL)
        if let Some(cached) = self.get_price(token_id) {
            return Some(cached);
        }

        // Try to fetch from API
        match self.fetch_price(token_id) {
            Ok(price) => Some(price),
            Err(_) => {
                // API failed - check if we have stale cached data
                self.cache.get(token_id).cloned()
            }
        }
    }

    /// Fetch prices for multiple tokens in batch
    /// Returns a HashMap of token_id -> PriceInfo for successful fetches
    /// Failed fetches are logged but don't stop the batch
    pub fn fetch_prices_batch(&mut self, token_ids: &[&str]) -> HashMap<String, PriceInfo> {
        let mut results = HashMap::new();

        for token_id in token_ids {
            match self.fetch_price(token_id) {
                Ok(price_info) => {
                    results.insert(token_id.to_string(), price_info);
                }
                Err(e) => {
                    // Log error but continue with batch
                    eprintln!("Failed to fetch price for {}: {}", token_id, e);
                }
            }
        }

        results
    }

    /// Get or fetch prices for multiple tokens in batch
    /// Uses cache when available, fetches missing/expired prices
    pub fn get_or_fetch_prices_batch(&mut self, token_ids: &[&str]) -> HashMap<String, PriceInfo> {
        let mut results = HashMap::new();

        for token_id in token_ids {
            // Try cache first
            if let Some(cached) = self.get_price(token_id) {
                results.insert(token_id.to_string(), cached);
            } else {
                // Cache miss - fetch from API
                match self.fetch_price(token_id) {
                    Ok(price_info) => {
                        results.insert(token_id.to_string(), price_info);
                    }
                    Err(e) => {
                        eprintln!("Failed to fetch price for {}: {}", token_id, e);
                    }
                }
            }
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_price_cache_new() {
        let cache = PriceCache::new(30);
        assert_eq!(cache.ttl_seconds, 30);
        assert_eq!(cache.cache.len(), 0);
    }

    #[test]
    fn test_cache_returns_valid_price() {
        let mut cache = PriceCache::new(30);
        let price_info = PriceInfo {
            bid_price: 0.45,
            ask_price: 0.46,
            timestamp: Instant::now(),
        };
        cache.set_price("token123".to_string(), price_info.clone());

        let retrieved = cache.get_price("token123");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.bid_price, 0.45);
        assert_eq!(retrieved.ask_price, 0.46);
    }

    #[test]
    fn test_cache_returns_none_for_missing_token() {
        let cache = PriceCache::new(30);
        assert!(cache.get_price("nonexistent").is_none());
    }

    #[test]
    fn test_cache_expires_after_ttl() {
        let mut cache = PriceCache::new(1); // 1 second TTL
        let price_info = PriceInfo {
            bid_price: 0.50,
            ask_price: 0.51,
            timestamp: Instant::now(),
        };
        cache.set_price("token456".to_string(), price_info);

        // Should be valid immediately
        assert!(cache.get_price("token456").is_some());

        // Wait for TTL to expire
        thread::sleep(Duration::from_secs(2));

        // Should now be expired
        assert!(cache.get_price("token456").is_none());
    }

    #[test]
    fn test_cache_with_default_ttl() {
        let cache = PriceCache::new(30);
        assert_eq!(cache.ttl_seconds, 30);
    }

    #[test]
    #[ignore] // Network test - requires active market with orderbook
    fn test_fetch_price_from_api() {
        // Note: This test requires a token with an active orderbook.
        // Markets frequently close, so you may need to update the token_id
        // to a currently active market from https://clob.polymarket.com/markets
        let mut cache = PriceCache::new(30);
        let token_id = "36161990524808999529099890841186860907449767867066339846328156147773282747583";

        let result = cache.fetch_price(token_id);

        // If the market is closed, this is expected to fail
        if result.is_err() {
            eprintln!("Warning: Market may be closed or orderbook unavailable");
            eprintln!("Error: {:?}", result);
            // Don't fail the test - just skip it
            return;
        }

        let price_info = result.unwrap();
        assert!(price_info.bid_price >= 0.0 && price_info.bid_price <= 1.0);
        assert!(price_info.ask_price >= 0.0 && price_info.ask_price <= 1.0);
        assert!(price_info.bid_price <= price_info.ask_price);
    }

    #[test]
    fn test_fetch_price_with_invalid_host() {
        let mut cache = PriceCache::with_host(30, "http://invalid-host-that-does-not-exist.example.com");
        let result = cache.fetch_price("test_token");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_or_fetch_returns_cached_value() {
        let mut cache = PriceCache::new(30);

        // Pre-populate cache
        let price_info = PriceInfo {
            bid_price: 0.55,
            ask_price: 0.56,
            timestamp: Instant::now(),
        };
        cache.set_price("token789".to_string(), price_info);

        // get_or_fetch should return cached value without hitting API
        let result = cache.get_or_fetch_price("token789");
        assert!(result.is_ok());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.bid_price, 0.55);
        assert_eq!(retrieved.ask_price, 0.56);
    }

    #[test]
    fn test_get_or_fetch_fetches_on_cache_miss() {
        let mut cache = PriceCache::with_host(30, "http://invalid-host.example.com");

        // No cached value - should attempt to fetch and fail (no network)
        let result = cache.get_or_fetch_price("new_token");
        assert!(result.is_err()); // Fails because host is invalid
    }

    #[test]
    fn test_get_or_fetch_refreshes_after_ttl() {
        let mut cache = PriceCache::with_host(1, "http://invalid-host.example.com");

        // Populate cache
        let price_info = PriceInfo {
            bid_price: 0.60,
            ask_price: 0.61,
            timestamp: Instant::now(),
        };
        cache.set_price("token_ttl".to_string(), price_info);

        // Should return cached value immediately
        let result = cache.get_or_fetch_price("token_ttl");
        assert!(result.is_ok());

        // Wait for TTL to expire
        thread::sleep(Duration::from_secs(2));

        // Should now attempt to fetch (and fail because invalid host)
        let result = cache.get_or_fetch_price("token_ttl");
        assert!(result.is_err());
    }

    #[test]
    fn test_rate_limiting_delays_requests() {
        // Set rate limit to 2 requests per second (500ms between requests)
        let mut cache = PriceCache::with_host(30, "http://invalid-host.example.com")
            .with_rate_limit(2);

        assert_eq!(cache.min_request_interval, Duration::from_millis(500));

        // First request - no delay
        let start = Instant::now();
        let _ = cache.fetch_price("token1"); // Will fail but rate limit applies

        // Second request should be delayed by ~500ms
        let _ = cache.fetch_price("token2");
        let elapsed = start.elapsed();

        // Should have taken at least 500ms for the second request
        assert!(
            elapsed >= Duration::from_millis(500),
            "Expected at least 500ms, got {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_millis(600),
            "Expected less than 600ms, got {:?}",
            elapsed
        );
    }

    #[test]
    fn test_default_rate_limit() {
        let cache = PriceCache::new(30);
        // Default should be 10 requests per second = 100ms interval
        assert_eq!(cache.min_request_interval, Duration::from_millis(100));
    }

    #[test]
    fn test_custom_rate_limit() {
        let cache = PriceCache::new(30).with_rate_limit(5);
        // 5 requests per second = 200ms interval
        assert_eq!(cache.min_request_interval, Duration::from_millis(200));
    }

    #[test]
    fn test_batch_fetch_with_rate_limiting() {
        // Use slow rate limit to verify rate limiting works in batch
        let mut cache = PriceCache::with_host(30, "http://invalid-host.example.com")
            .with_rate_limit(5); // 200ms between requests

        let tokens = vec!["token1", "token2", "token3"];

        let start = Instant::now();
        let _results = cache.fetch_prices_batch(&tokens);
        let elapsed = start.elapsed();

        // 3 requests with 200ms interval = ~400ms total (first request is immediate)
        assert!(
            elapsed >= Duration::from_millis(400),
            "Expected at least 400ms for 3 requests, got {:?}",
            elapsed
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "Expected less than 500ms, got {:?}",
            elapsed
        );
    }

    #[test]
    fn test_get_or_fetch_batch_uses_cache() {
        let mut cache = PriceCache::new(30);

        // Pre-populate cache with some tokens
        cache.set_price(
            "cached1".to_string(),
            PriceInfo {
                bid_price: 0.30,
                ask_price: 0.31,
                timestamp: Instant::now(),
            },
        );
        cache.set_price(
            "cached2".to_string(),
            PriceInfo {
                bid_price: 0.40,
                ask_price: 0.41,
                timestamp: Instant::now(),
            },
        );

        // Mix of cached and uncached tokens (uncached will fail with invalid host)
        let mut cache_with_invalid_host =
            PriceCache::with_host(30, "http://invalid-host.example.com");
        cache_with_invalid_host.cache = cache.cache.clone();

        let tokens = vec!["cached1", "cached2", "uncached"];
        let results = cache_with_invalid_host.get_or_fetch_prices_batch(&tokens);

        // Should get 2 cached results
        assert_eq!(results.len(), 2);
        assert!(results.contains_key("cached1"));
        assert!(results.contains_key("cached2"));
        assert!(!results.contains_key("uncached")); // Failed to fetch

        // Verify cached values
        assert_eq!(results.get("cached1").unwrap().bid_price, 0.30);
        assert_eq!(results.get("cached2").unwrap().ask_price, 0.41);
    }

    #[test]
    fn test_batch_fetch_empty_list() {
        let mut cache = PriceCache::new(30);
        let tokens: Vec<&str> = vec![];
        let results = cache.fetch_prices_batch(&tokens);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_batch_fetch_single_token() {
        let mut cache = PriceCache::with_host(30, "http://invalid-host.example.com");
        let tokens = vec!["token1"];
        let results = cache.fetch_prices_batch(&tokens);
        // Should fail because invalid host, but shouldn't panic
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_fallback_returns_stale_cache_on_api_error() {
        let mut cache = PriceCache::with_host(1, "http://invalid-host.example.com");

        // Populate cache with stale data
        let stale_price = PriceInfo {
            bid_price: 0.65,
            ask_price: 0.66,
            timestamp: Instant::now(),
        };
        cache.set_price("token_fallback".to_string(), stale_price);

        // Wait for TTL to expire
        thread::sleep(Duration::from_secs(2));

        // get_or_fetch_price should fail (no valid cache, API fails)
        let result = cache.get_or_fetch_price("token_fallback");
        assert!(result.is_err());

        // But get_or_fetch_price_with_fallback should return stale cache
        let fallback_result = cache.get_or_fetch_price_with_fallback("token_fallback");
        assert!(fallback_result.is_some());
        let price = fallback_result.unwrap();
        assert_eq!(price.bid_price, 0.65);
        assert_eq!(price.ask_price, 0.66);
    }

    #[test]
    fn test_fallback_returns_none_when_no_cache() {
        let mut cache = PriceCache::with_host(30, "http://invalid-host.example.com");

        // No cached data, API will fail
        let result = cache.get_or_fetch_price_with_fallback("nonexistent");
        assert!(result.is_none());
    }
}
