//! Trade Aggregation Module
//!
//! This module provides functionality to aggregate multiple small trades into single orders
//! for improved efficiency and reduced fees. Trades are aggregated within time windows,
//! with configurable thresholds for bypassing aggregation on large trades.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::models::{OrderInfo, ParsedEvent};

/// Configuration for trade aggregation behavior
#[derive(Debug, Clone)]
pub struct AggregationConfig {
    /// Time window for aggregating trades (default: 800ms)
    pub window_duration: Duration,
    /// Minimum number of trades required to aggregate (default: 2)
    pub min_trades: usize,
    /// USD value threshold that forces immediate flush (default: $500)
    pub max_pending_usd: f64,
    /// Share count threshold that bypasses aggregation (default: 4000 shares)
    pub bypass_threshold: f64,
}

impl Default for AggregationConfig {
    fn default() -> Self {
        Self {
            window_duration: Duration::from_millis(800),
            min_trades: 2,
            max_pending_usd: 500.0,
            bypass_threshold: 4000.0,
        }
    }
}

/// Represents a single trade pending aggregation
#[derive(Debug, Clone)]
pub struct PendingTrade {
    /// Token ID being traded
    pub token_id: String,
    /// Trade side (BUY or SELL)
    pub side: String,
    /// Number of shares
    pub shares: f64,
    /// Price per share
    pub price: f64,
    /// Timestamp when trade was added
    pub timestamp: Instant,
    /// Trader address that initiated the trade
    pub trader: String,
}

impl PendingTrade {
    /// Create a new pending trade
    pub fn new(
        token_id: String,
        side: String,
        shares: f64,
        price: f64,
        trader: String,
    ) -> Self {
        Self {
            token_id,
            side,
            shares,
            price,
            timestamp: Instant::now(),
            trader,
        }
    }

    /// Calculate the USD value of this trade
    pub fn usd_value(&self) -> f64 {
        self.shares * self.price
    }

    /// Create aggregation key from token_id and side
    pub fn aggregation_key(&self) -> String {
        format!("{}:{}", self.token_id, self.side)
    }
}

/// Represents the result of aggregating multiple trades
#[derive(Debug, Clone)]
pub struct AggregatedTrade {
    /// Token ID being traded
    pub token_id: String,
    /// Trade side (BUY or SELL)
    pub side: String,
    /// Total shares across all aggregated trades
    pub total_shares: f64,
    /// Weighted average price
    pub avg_price: f64,
    /// Number of trades aggregated
    pub trade_count: usize,
    /// Total USD value
    pub total_usd: f64,
    /// Timestamp of first trade in the aggregation
    pub first_trade_time: Instant,
    /// List of trader addresses involved
    pub traders: Vec<String>,
}

impl AggregatedTrade {
    /// Convert this aggregated trade into a ParsedEvent for order execution
    ///
    /// Creates a synthetic event with:
    /// - block_number: 0 (synthetic)
    /// - tx_hash: "AGG_{trade_count}_{token_id_prefix}"
    /// - trader_address: first trader in the aggregation
    /// - trader_label: "AGGREGATED"
    /// - trader_min_shares: 0.0 (already passed threshold checks)
    pub fn to_parsed_event(&self) -> ParsedEvent {
        let token_id_prefix = if self.token_id.len() > 10 {
            &self.token_id[..10]
        } else {
            &self.token_id
        };

        ParsedEvent {
            block_number: 0,
            tx_hash: format!("AGG_{}_{}", self.trade_count, token_id_prefix),
            trader_address: self.traders.first().cloned().unwrap_or_default(),
            trader_label: "AGGREGATED".to_string(),
            trader_min_shares: 0.0, // Already passed min_shares checks
            order: OrderInfo {
                order_type: format!("{}_FILL", self.side),
                clob_token_id: Arc::from(self.token_id.as_str()),
                usd_value: self.total_usd,
                shares: self.total_shares,
                price_per_share: self.avg_price,
            },
        }
    }

    /// Create an aggregated trade from a collection of pending trades
    pub fn from_trades(trades: Vec<PendingTrade>) -> Option<Self> {
        if trades.is_empty() {
            return None;
        }

        let token_id = trades[0].token_id.clone();
        let side = trades[0].side.clone();
        let first_trade_time = trades[0].timestamp;

        let mut total_value = 0.0;
        let mut total_shares = 0.0;
        let mut traders = Vec::new();

        for trade in &trades {
            total_value += trade.usd_value();
            total_shares += trade.shares;
            if !traders.contains(&trade.trader) {
                traders.push(trade.trader.clone());
            }
        }

        let avg_price = if total_shares > 0.0 {
            total_value / total_shares
        } else {
            0.0
        };

        Some(Self {
            token_id,
            side,
            total_shares,
            avg_price,
            trade_count: trades.len(),
            total_usd: total_value,
            first_trade_time,
            traders,
        })
    }
}

/// Main aggregator that manages pending trades and produces aggregated trades
pub struct TradeAggregator {
    config: AggregationConfig,
    /// Pending trades grouped by (token_id, side)
    pending: HashMap<String, Vec<PendingTrade>>,
}

impl TradeAggregator {
    /// Create a new trade aggregator with the given configuration
    pub fn new(config: AggregationConfig) -> Self {
        Self {
            config,
            pending: HashMap::new(),
        }
    }

    /// Add a trade to the aggregator
    /// Returns Some(AggregatedTrade) if the trade should be executed immediately
    /// Returns None if the trade is added to the pending window
    pub fn add_trade(
        &mut self,
        token_id: String,
        side: String,
        shares: f64,
        price: f64,
        trader: String,
    ) -> Option<AggregatedTrade> {
        // Check bypass threshold - large trades execute immediately
        if shares >= self.config.bypass_threshold {
            let trade = PendingTrade::new(token_id, side, shares, price, trader);
            return AggregatedTrade::from_trades(vec![trade]);
        }

        // Add to pending trades
        let trade = PendingTrade::new(token_id, side, shares, price, trader);
        let key = trade.aggregation_key();

        let pending_trades = self.pending.entry(key.clone()).or_insert_with(Vec::new);
        pending_trades.push(trade);

        // Check if we should flush due to max_pending_usd
        let total_usd: f64 = pending_trades.iter().map(|t| t.usd_value()).sum();
        if total_usd >= self.config.max_pending_usd {
            return self.flush_key(&key);
        }

        None
    }

    /// Flush all pending trades for a specific key
    fn flush_key(&mut self, key: &str) -> Option<AggregatedTrade> {
        if let Some(trades) = self.pending.remove(key) {
            if trades.len() >= self.config.min_trades {
                return AggregatedTrade::from_trades(trades);
            } else {
                // Put back if not enough trades
                self.pending.insert(key.to_string(), trades);
            }
        }
        None
    }

    /// Check and flush expired windows
    /// Returns a vector of aggregated trades ready for execution
    pub fn flush_expired(&mut self) -> Vec<AggregatedTrade> {
        let now = Instant::now();
        let mut to_flush = Vec::new();

        // Find keys with expired windows
        for (key, trades) in &self.pending {
            if let Some(first_trade) = trades.first() {
                if now.duration_since(first_trade.timestamp) >= self.config.window_duration {
                    to_flush.push(key.clone());
                }
            }
        }

        // Flush expired keys
        let mut aggregated = Vec::new();
        for key in to_flush {
            if let Some(agg) = self.flush_key(&key) {
                aggregated.push(agg);
            }
        }

        aggregated
    }

    /// Flush all pending trades (used during shutdown)
    pub fn flush_all(&mut self) -> Vec<AggregatedTrade> {
        let mut aggregated = Vec::new();

        for (_key, trades) in self.pending.drain() {
            if trades.len() >= self.config.min_trades {
                if let Some(agg) = AggregatedTrade::from_trades(trades) {
                    aggregated.push(agg);
                }
            }
        }

        aggregated
    }

    /// Get count of pending trades
    pub fn pending_count(&self) -> usize {
        self.pending.values().map(|v| v.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregation_config_defaults() {
        let config = AggregationConfig::default();

        assert_eq!(config.window_duration, Duration::from_millis(800));
        assert_eq!(config.min_trades, 2);
        assert_eq!(config.max_pending_usd, 500.0);
        assert_eq!(config.bypass_threshold, 4000.0);
    }

    #[test]
    fn test_aggregation_config_custom_values() {
        let config = AggregationConfig {
            window_duration: Duration::from_millis(1000),
            min_trades: 3,
            max_pending_usd: 1000.0,
            bypass_threshold: 5000.0,
        };

        assert_eq!(config.window_duration, Duration::from_millis(1000));
        assert_eq!(config.min_trades, 3);
        assert_eq!(config.max_pending_usd, 1000.0);
        assert_eq!(config.bypass_threshold, 5000.0);
    }

    #[test]
    fn test_pending_trade_creation() {
        let trade = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        assert_eq!(trade.token_id, "0xabc123");
        assert_eq!(trade.side, "BUY");
        assert_eq!(trade.shares, 100.0);
        assert_eq!(trade.price, 0.50);
        assert_eq!(trade.trader, "0xtrader");
    }

    #[test]
    fn test_pending_trade_usd_value() {
        let trade = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            150.0,
            0.45,
            "0xtrader".to_string(),
        );

        assert_eq!(trade.usd_value(), 67.5); // 150 * 0.45
    }

    #[test]
    fn test_pending_trade_aggregation_key() {
        let trade = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        assert_eq!(trade.aggregation_key(), "0xabc123:BUY");
    }

    #[test]
    fn test_pending_trade_different_keys_for_different_sides() {
        let buy_trade = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        let sell_trade = PendingTrade::new(
            "0xabc123".to_string(),
            "SELL".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        assert_ne!(buy_trade.aggregation_key(), sell_trade.aggregation_key());
    }

    #[test]
    fn test_aggregated_trade_from_empty_trades() {
        let trades: Vec<PendingTrade> = vec![];
        let result = AggregatedTrade::from_trades(trades);
        assert!(result.is_none());
    }

    #[test]
    fn test_aggregated_trade_from_single_trade() {
        let trade = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        let aggregated = AggregatedTrade::from_trades(vec![trade]).unwrap();

        assert_eq!(aggregated.token_id, "0xabc123");
        assert_eq!(aggregated.side, "BUY");
        assert_eq!(aggregated.total_shares, 100.0);
        assert_eq!(aggregated.avg_price, 0.50);
        assert_eq!(aggregated.trade_count, 1);
        assert_eq!(aggregated.total_usd, 50.0);
        assert_eq!(aggregated.traders.len(), 1);
    }

    #[test]
    fn test_aggregated_trade_weighted_average_price() {
        let trade1 = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.40,
            "0xtrader1".to_string(),
        );

        let trade2 = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            200.0,
            0.50,
            "0xtrader2".to_string(),
        );

        let aggregated = AggregatedTrade::from_trades(vec![trade1, trade2]).unwrap();

        // Weighted avg: (100*0.40 + 200*0.50) / 300 = 140/300 = 0.4666...
        assert_eq!(aggregated.total_shares, 300.0);
        assert!((aggregated.avg_price - 0.4666666666666667).abs() < 0.0001);
        assert_eq!(aggregated.trade_count, 2);
        assert_eq!(aggregated.total_usd, 140.0);
    }

    #[test]
    fn test_aggregated_trade_multiple_traders() {
        let trade1 = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            50.0,
            0.45,
            "0xtrader1".to_string(),
        );

        let trade2 = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            50.0,
            0.45,
            "0xtrader2".to_string(),
        );

        let trade3 = PendingTrade::new(
            "0xabc123".to_string(),
            "BUY".to_string(),
            50.0,
            0.45,
            "0xtrader1".to_string(), // Duplicate trader
        );

        let aggregated = AggregatedTrade::from_trades(vec![trade1, trade2, trade3]).unwrap();

        assert_eq!(aggregated.trade_count, 3);
        assert_eq!(aggregated.total_shares, 150.0);
        assert_eq!(aggregated.traders.len(), 2); // Only unique traders
        assert!(aggregated.traders.contains(&"0xtrader1".to_string()));
        assert!(aggregated.traders.contains(&"0xtrader2".to_string()));
    }

    // TradeAggregator Tests

    #[test]
    fn test_aggregator_creation() {
        let config = AggregationConfig::default();
        let aggregator = TradeAggregator::new(config);
        assert_eq!(aggregator.pending_count(), 0);
    }

    #[test]
    fn test_aggregator_bypass_large_trade() {
        let config = AggregationConfig::default();
        let mut aggregator = TradeAggregator::new(config);

        // Large trade bypasses aggregation
        let result = aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            5000.0, // Above bypass threshold of 4000
            0.50,
            "0xtrader".to_string(),
        );

        assert!(result.is_some());
        let agg = result.unwrap();
        assert_eq!(agg.total_shares, 5000.0);
        assert_eq!(agg.trade_count, 1);
        assert_eq!(aggregator.pending_count(), 0);
    }

    #[test]
    fn test_aggregator_small_trade_goes_pending() {
        let config = AggregationConfig::default();
        let mut aggregator = TradeAggregator::new(config);

        // Small trade goes to pending
        let result = aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        assert!(result.is_none());
        assert_eq!(aggregator.pending_count(), 1);
    }

    #[test]
    fn test_aggregator_max_usd_triggers_flush() {
        let config = AggregationConfig {
            max_pending_usd: 100.0, // Low threshold for testing
            ..Default::default()
        };
        let mut aggregator = TradeAggregator::new(config);

        // First trade: 50 shares * $0.50 = $25 - goes to pending
        let result1 = aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            50.0,
            0.50,
            "0xtrader1".to_string(),
        );
        assert!(result1.is_none());
        assert_eq!(aggregator.pending_count(), 1);

        // Second trade: 200 shares * $0.50 = $100, total = $125 - triggers flush
        let result2 = aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            200.0,
            0.50,
            "0xtrader2".to_string(),
        );

        assert!(result2.is_some());
        let agg = result2.unwrap();
        assert_eq!(agg.trade_count, 2);
        assert_eq!(agg.total_shares, 250.0);
        assert_eq!(aggregator.pending_count(), 0);
    }

    #[test]
    fn test_aggregator_different_tokens_separate() {
        let config = AggregationConfig::default();
        let mut aggregator = TradeAggregator::new(config);

        // Add trades for different tokens
        aggregator.add_trade(
            "0xtoken1".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        aggregator.add_trade(
            "0xtoken2".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        assert_eq!(aggregator.pending_count(), 2);
    }

    #[test]
    fn test_aggregator_different_sides_separate() {
        let config = AggregationConfig::default();
        let mut aggregator = TradeAggregator::new(config);

        // Add BUY and SELL for same token
        aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        aggregator.add_trade(
            "0xabc123".to_string(),
            "SELL".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        assert_eq!(aggregator.pending_count(), 2);
    }

    #[test]
    fn test_aggregator_flush_expired() {
        let config = AggregationConfig {
            window_duration: Duration::from_millis(50), // Short window for testing
            ..Default::default()
        };
        let mut aggregator = TradeAggregator::new(config);

        // Add two trades
        aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.40,
            "0xtrader1".to_string(),
        );

        aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            200.0,
            0.50,
            "0xtrader2".to_string(),
        );

        assert_eq!(aggregator.pending_count(), 2);

        // Wait for window to expire
        std::thread::sleep(Duration::from_millis(100));

        // Flush expired
        let expired = aggregator.flush_expired();

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].trade_count, 2);
        assert_eq!(expired[0].total_shares, 300.0);
        assert_eq!(aggregator.pending_count(), 0);
    }

    #[test]
    fn test_aggregator_flush_all() {
        let config = AggregationConfig::default();
        let mut aggregator = TradeAggregator::new(config);

        // Add trades for different tokens
        aggregator.add_trade(
            "0xtoken1".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        aggregator.add_trade(
            "0xtoken1".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        aggregator.add_trade(
            "0xtoken2".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader".to_string(),
        );

        assert_eq!(aggregator.pending_count(), 3);

        // Flush all
        let all = aggregator.flush_all();

        // Should get 1 aggregated trade (token1 with 2 trades)
        // token2 only has 1 trade, below min_trades
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].trade_count, 2);
        assert_eq!(aggregator.pending_count(), 0);
    }

    #[test]
    fn test_aggregator_min_trades_requirement() {
        let config = AggregationConfig {
            min_trades: 3,
            ..Default::default()
        };
        let mut aggregator = TradeAggregator::new(config);

        // Add 2 trades
        aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader1".to_string(),
        );

        aggregator.add_trade(
            "0xabc123".to_string(),
            "BUY".to_string(),
            100.0,
            0.50,
            "0xtrader2".to_string(),
        );

        // Flush all - should not aggregate because min_trades = 3
        let all = aggregator.flush_all();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_aggregator_performance_add_trade() {
        use std::time::Instant;

        let config = AggregationConfig::default();
        let mut aggregator = TradeAggregator::new(config);

        // Warm-up
        for _ in 0..10 {
            aggregator.add_trade(
                "0xwarmup".to_string(),
                "BUY".to_string(),
                100.0,
                0.50,
                "0xtrader".to_string(),
            );
        }
        aggregator.flush_all();

        // Benchmark add_trade performance
        let iterations = 1000;
        let start = Instant::now();

        for i in 0..iterations {
            aggregator.add_trade(
                format!("0xtoken{}", i % 10), // 10 different tokens
                "BUY".to_string(),
                100.0,
                0.50,
                "0xtrader".to_string(),
            );
        }

        let elapsed = start.elapsed();
        let avg_per_call = elapsed.as_nanos() / iterations;

        // Should be less than 100 microseconds (100,000 nanoseconds)
        println!("Average add_trade time: {}ns ({}us)", avg_per_call, avg_per_call / 1000);
        assert!(avg_per_call < 100_000, "add_trade took {}ns, expected < 100,000ns", avg_per_call);
    }

    #[test]
    fn test_aggregated_trade_to_parsed_event() {
        let trade1 = PendingTrade::new(
            "0xabc123456789".to_string(),
            "BUY".to_string(),
            100.0,
            0.40,
            "0xtrader1".to_string(),
        );

        let trade2 = PendingTrade::new(
            "0xabc123456789".to_string(),
            "BUY".to_string(),
            200.0,
            0.50,
            "0xtrader2".to_string(),
        );

        let aggregated = AggregatedTrade::from_trades(vec![trade1, trade2]).unwrap();
        let event = aggregated.to_parsed_event();

        // Verify event fields
        assert_eq!(event.block_number, 0);
        assert!(event.tx_hash.starts_with("AGG_2_"));
        assert_eq!(event.trader_address, "0xtrader1"); // First trader
        assert_eq!(event.trader_label, "AGGREGATED");
        assert_eq!(event.trader_min_shares, 0.0);

        // Verify order info
        assert_eq!(event.order.order_type, "BUY_FILL");
        assert_eq!(event.order.clob_token_id.as_ref(), "0xabc123456789");
        assert_eq!(event.order.shares, 300.0);
        // Weighted avg: (100*0.40 + 200*0.50) / 300 = 140/300 = 0.4666...
        assert!((event.order.price_per_share - 0.4666666666666667).abs() < 0.0001);
        assert_eq!(event.order.usd_value, 140.0);
    }

    #[test]
    fn test_aggregated_trade_to_parsed_event_sell() {
        let trade = PendingTrade::new(
            "0xdef456".to_string(),
            "SELL".to_string(),
            500.0,
            0.75,
            "0xseller".to_string(),
        );

        let aggregated = AggregatedTrade::from_trades(vec![trade]).unwrap();
        let event = aggregated.to_parsed_event();

        assert_eq!(event.order.order_type, "SELL_FILL");
        assert_eq!(event.order.shares, 500.0);
        assert_eq!(event.order.price_per_share, 0.75);
        assert_eq!(event.order.usd_value, 375.0);
    }
}
