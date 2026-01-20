// src/types.rs
// Core types for the trading system

use serde::Deserialize;
use std::fmt;
use std::sync::Arc;
use tokio::sync::oneshot;

/// Parsed order information from blockchain events
#[derive(Debug, Clone)]
pub struct OrderInfo {
    pub order_type: String,
    pub clob_token_id: Arc<str>,  
    pub usd_value: f64,
    pub shares: f64,
    pub price_per_share: f64,
}

/// Fully parsed blockchain event ready for processing
#[derive(Debug, Clone)]
pub struct ParsedEvent {
    pub block_number: u64,
    pub tx_hash: String,
    /// Normalized 40-character hex address of the trader (lowercase, no 0x prefix)
    /// Empty string if trader is unknown or not yet populated
    pub trader_address: String,
    /// Human-friendly label for the trader (e.g., "Whale1", "TopTrader")
    /// Empty string if trader is unknown or not yet populated
    pub trader_label: String,
    pub order: OrderInfo,
}

/// Work item for the order processing queue
#[derive(Debug)]
pub struct WorkItem {
    pub event: ParsedEvent,
    pub respond_to: oneshot::Sender<String>,
    pub is_live: Option<bool>,
}

/// Size calculation result 
#[derive(Debug, Clone, Copy)]
pub enum SizeType {
    Scaled,
    ProbHit(u8),   // percentage
    ProbSkip(u8),  // percentage
}

/// Request to resubmit a failed FAK order 
/// Fields ordered to minimize padding: f64s together, then bools/u8 at end
#[derive(Debug, Clone)]
pub struct ResubmitRequest {
    pub token_id: String,       // 24 bytes
    pub whale_price: f64,       // Original whale price
    pub failed_price: f64,      // Price that failed (our limit)
    pub size: f64,              // Order size in shares
    pub whale_shares: f64,      // Whale's trade size (for tier-based max attempts)
    pub max_price: f64,         // Price ceiling (don't exceed this)
    pub cumulative_filled: f64, // Total filled before this attempt
    pub original_size: f64,     // Original order size (for final summary)
    pub side_is_buy: bool,      // Always true for now (only resubmit buys)
    pub is_live: bool,          // Market liveness (for GTD expiry calculation)
    pub attempt: u8,            // Current attempt number (1-indexed)
}

impl fmt::Display for SizeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SizeType::Scaled => f.write_str("SCALED"),
            SizeType::ProbHit(pct) => write!(f, "PROB_HIT ({}%)", pct),
            SizeType::ProbSkip(pct) => write!(f, "PROB_SKIP ({}%)", pct),
        }
    }
}

// ============================================================================
// WebSocket message types (for parsing incoming events)
// ============================================================================

#[derive(Deserialize)]
pub struct WsMessage {
    pub params: Option<WsParams>,
}

#[derive(Deserialize)]
pub struct WsParams {
    pub result: Option<LogResult>,
}

#[derive(Deserialize)]
pub struct LogResult {
    pub topics: Vec<String>,
    pub data: String,
    #[serde(rename = "blockNumber")]
    pub block_number: Option<String>,
    #[serde(rename = "transactionHash")]
    pub transaction_hash: Option<String>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that ParsedEvent can hold trader address information
    #[test]
    fn test_parsed_event_has_trader_address() {
        let event = ParsedEvent {
            block_number: 12345,
            tx_hash: "0xabc123".to_string(),
            trader_address: "abc123def456789012345678901234567890abcd".to_string(),
            trader_label: "Whale1".to_string(),
            order: OrderInfo {
                order_type: "BUY_FILL".to_string(),
                clob_token_id: Arc::from("123456"),
                usd_value: 100.0,
                shares: 500.0,
                price_per_share: 0.20,
            },
        };

        assert_eq!(event.trader_address, "abc123def456789012345678901234567890abcd");
        assert_eq!(event.trader_label, "Whale1");
    }

    /// Test that ParsedEvent trader fields can be empty strings (for unknown traders)
    #[test]
    fn test_parsed_event_trader_fields_can_be_empty() {
        let event = ParsedEvent {
            block_number: 12345,
            tx_hash: "0xabc123".to_string(),
            trader_address: String::new(),
            trader_label: String::new(),
            order: OrderInfo {
                order_type: "BUY_FILL".to_string(),
                clob_token_id: Arc::from("123456"),
                usd_value: 100.0,
                shares: 500.0,
                price_per_share: 0.20,
            },
        };

        assert_eq!(event.trader_address, "");
        assert_eq!(event.trader_label, "");
    }

    /// Test that ParsedEvent can be cloned with trader information
    #[test]
    fn test_parsed_event_clone_preserves_trader_info() {
        let event1 = ParsedEvent {
            block_number: 12345,
            tx_hash: "0xabc123".to_string(),
            trader_address: "def456def456789012345678901234567890def4".to_string(),
            trader_label: "TopTrader".to_string(),
            order: OrderInfo {
                order_type: "SELL_FILL".to_string(),
                clob_token_id: Arc::from("789012"),
                usd_value: 250.0,
                shares: 1000.0,
                price_per_share: 0.25,
            },
        };

        let event2 = event1.clone();

        assert_eq!(event2.trader_address, "def456def456789012345678901234567890def4");
        assert_eq!(event2.trader_label, "TopTrader");
        assert_eq!(event2.block_number, 12345);
    }
}