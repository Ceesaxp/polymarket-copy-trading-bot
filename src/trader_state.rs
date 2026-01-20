/// Per-trader state management
/// Tracks trading activity, success rates, and daily statistics for each monitored trader

use std::collections::HashMap;
use std::time::Instant;
use chrono::{DateTime, Utc};
use crate::config::traders::TradersConfig;

/// Status of a trade execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeStatus {
    Success,
    Failed,
    Partial,
    Skipped,
}

/// State tracking for a single trader
#[derive(Debug, Clone)]
pub struct TraderState {
    pub address: String,
    pub label: String,
    pub total_copied_usd: f64,
    pub trades_today: u32,
    pub successful_trades: u32,
    pub failed_trades: u32,
    pub partial_trades: u32,
    pub last_trade_ts: Option<Instant>,
    pub daily_reset_ts: DateTime<Utc>,
}

impl TraderState {
    /// Creates a new TraderState with default values
    pub fn new(address: String, label: String) -> Self {
        Self {
            address,
            label,
            total_copied_usd: 0.0,
            trades_today: 0,
            successful_trades: 0,
            failed_trades: 0,
            partial_trades: 0,
            last_trade_ts: None,
            daily_reset_ts: Utc::now(),
        }
    }
}

/// Manager for all trader states
pub struct TraderManager {
    states: HashMap<String, TraderState>,
}

impl TraderManager {
    /// Creates a new TraderManager initialized with traders from config
    pub fn new(traders: &TradersConfig) -> Self {
        let mut states = HashMap::new();

        for trader in traders.iter() {
            let state = TraderState::new(
                trader.address.clone(),
                trader.label.clone(),
            );
            states.insert(trader.address.clone(), state);
        }

        Self { states }
    }

    /// Records a trade execution and updates stats
    pub fn record_trade(&mut self, address: &str, usd_amount: f64, status: TradeStatus) {
        if let Some(state) = self.states.get_mut(address) {
            state.last_trade_ts = Some(Instant::now());
            state.trades_today += 1;

            match status {
                TradeStatus::Success => {
                    state.successful_trades += 1;
                    state.total_copied_usd += usd_amount;
                }
                TradeStatus::Failed => {
                    state.failed_trades += 1;
                }
                TradeStatus::Partial => {
                    state.partial_trades += 1;
                    state.total_copied_usd += usd_amount;
                }
                TradeStatus::Skipped => {
                    // Skipped trades don't increment any counter except trades_today
                }
            }
        }
    }

    /// Gets state for a specific trader
    pub fn get_state(&self, address: &str) -> Option<&TraderState> {
        self.states.get(address)
    }

    /// Gets all trader states
    pub fn get_all_states(&self) -> Vec<&TraderState> {
        self.states.values().collect()
    }

    /// Resets daily statistics for all traders
    pub fn check_daily_reset(&mut self) {
        let now = Utc::now();

        for state in self.states.values_mut() {
            // Check if we've crossed midnight UTC
            if now.date_naive() > state.daily_reset_ts.date_naive() {
                state.trades_today = 0;
                state.daily_reset_ts = now;
            }
        }
    }
}

/// Aggregate statistics across all traders
#[derive(Debug, Clone)]
pub struct ManagerStats {
    pub total_traders: usize,
    pub total_trades: u32,
    pub total_successful: u32,
    pub total_failed: u32,
    pub total_partial: u32,
    pub total_copied_usd: f64,
}

impl TraderManager {
    /// Gets summary statistics across all traders
    pub fn get_summary_stats(&self) -> ManagerStats {
        let mut stats = ManagerStats {
            total_traders: self.states.len(),
            total_trades: 0,
            total_successful: 0,
            total_failed: 0,
            total_partial: 0,
            total_copied_usd: 0.0,
        };

        for state in self.states.values() {
            stats.total_trades += state.trades_today;
            stats.total_successful += state.successful_trades;
            stats.total_failed += state.failed_trades;
            stats.total_partial += state.partial_trades;
            stats.total_copied_usd += state.total_copied_usd;
        }

        stats
    }

    /// Persists trader stats to database
    ///
    /// # Arguments
    /// * `store` - TradeStore instance to persist to
    ///
    /// # Returns
    /// * `Result<()>` - Ok if all stats persisted successfully
    #[cfg(not(test))]
    pub fn persist_to_db(&self, store: &crate::persistence::TradeStore) -> anyhow::Result<()> {
        for state in self.states.values() {
            let last_trade_ts = state.last_trade_ts.map(|_| {
                // Convert Instant to timestamp - we can't convert directly,
                // so we just use current time if last_trade_ts is set
                chrono::Utc::now().timestamp_millis()
            });

            let daily_reset_ts = state.daily_reset_ts.timestamp_millis();

            store.upsert_trader_stats(
                &state.address,
                &state.label,
                state.trades_today,
                state.successful_trades,
                state.failed_trades,
                state.total_copied_usd,
                last_trade_ts,
                daily_reset_ts,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::traders::TraderConfig;

    #[test]
    fn test_trader_state_new_initializes_with_defaults() {
        let address = "abc123def456789012345678901234567890abcd".to_string();
        let label = "Whale1".to_string();

        let state = TraderState::new(address.clone(), label.clone());

        assert_eq!(state.address, address);
        assert_eq!(state.label, label);
        assert_eq!(state.total_copied_usd, 0.0);
        assert_eq!(state.trades_today, 0);
        assert_eq!(state.successful_trades, 0);
        assert_eq!(state.failed_trades, 0);
        assert_eq!(state.partial_trades, 0);
        assert!(state.last_trade_ts.is_none());
    }

    #[test]
    fn test_trader_state_daily_reset_ts_initialized() {
        let state = TraderState::new(
            "abc123def456789012345678901234567890abcd".to_string(),
            "Test".to_string(),
        );

        // daily_reset_ts should be initialized to current time
        let now = Utc::now();
        let diff = (now - state.daily_reset_ts).num_seconds().abs();
        assert!(diff < 5, "daily_reset_ts should be initialized to current time");
    }

    #[test]
    fn test_trader_manager_new_initializes_from_config() {
        let trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();

        let trader2 = TraderConfig::new(
            "def456def456789012345678901234567890def4",
            "Whale2",
        ).unwrap();

        let config = TradersConfig::new(vec![trader1, trader2]);
        let manager = TraderManager::new(&config);

        assert_eq!(manager.states.len(), 2);

        let state1 = manager.get_state("abc123def456789012345678901234567890abcd");
        assert!(state1.is_some());
        assert_eq!(state1.unwrap().label, "Whale1");

        let state2 = manager.get_state("def456def456789012345678901234567890def4");
        assert!(state2.is_some());
        assert_eq!(state2.unwrap().label, "Whale2");
    }

    #[test]
    fn test_record_trade_increments_success_counter() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        manager.record_trade("abc123def456789012345678901234567890abcd", 100.0, TradeStatus::Success);

        let state = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state.successful_trades, 1);
        assert_eq!(state.failed_trades, 0);
        assert_eq!(state.partial_trades, 0);
        assert_eq!(state.trades_today, 1);
    }

    #[test]
    fn test_record_trade_increments_failed_counter() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        manager.record_trade("abc123def456789012345678901234567890abcd", 0.0, TradeStatus::Failed);

        let state = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state.successful_trades, 0);
        assert_eq!(state.failed_trades, 1);
        assert_eq!(state.partial_trades, 0);
        assert_eq!(state.trades_today, 1);
    }

    #[test]
    fn test_record_trade_increments_partial_counter() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        manager.record_trade("abc123def456789012345678901234567890abcd", 50.0, TradeStatus::Partial);

        let state = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state.successful_trades, 0);
        assert_eq!(state.failed_trades, 0);
        assert_eq!(state.partial_trades, 1);
        assert_eq!(state.trades_today, 1);
    }

    #[test]
    fn test_record_trade_accumulates_usd_on_success() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        manager.record_trade("abc123def456789012345678901234567890abcd", 100.0, TradeStatus::Success);
        manager.record_trade("abc123def456789012345678901234567890abcd", 50.0, TradeStatus::Success);

        let state = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state.total_copied_usd, 150.0);
        assert_eq!(state.successful_trades, 2);
    }

    #[test]
    fn test_record_trade_accumulates_usd_on_partial() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        manager.record_trade("abc123def456789012345678901234567890abcd", 75.0, TradeStatus::Partial);

        let state = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state.total_copied_usd, 75.0);
        assert_eq!(state.partial_trades, 1);
    }

    #[test]
    fn test_record_trade_does_not_accumulate_usd_on_failure() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        manager.record_trade("abc123def456789012345678901234567890abcd", 0.0, TradeStatus::Failed);

        let state = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state.total_copied_usd, 0.0);
        assert_eq!(state.failed_trades, 1);
    }

    #[test]
    fn test_record_trade_updates_last_trade_timestamp() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        let state_before = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert!(state_before.last_trade_ts.is_none());

        manager.record_trade("abc123def456789012345678901234567890abcd", 100.0, TradeStatus::Success);

        let state_after = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert!(state_after.last_trade_ts.is_some());
    }

    #[test]
    fn test_record_trade_skipped_increments_only_trades_today() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        manager.record_trade("abc123def456789012345678901234567890abcd", 0.0, TradeStatus::Skipped);

        let state = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state.successful_trades, 0);
        assert_eq!(state.failed_trades, 0);
        assert_eq!(state.partial_trades, 0);
        assert_eq!(state.trades_today, 1);
        assert_eq!(state.total_copied_usd, 0.0);
    }

    #[test]
    fn test_get_state_returns_none_for_unknown_trader() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let manager = TraderManager::new(&config);

        let state = manager.get_state("unknown000000000000000000000000000000000");
        assert!(state.is_none());
    }

    #[test]
    fn test_get_all_states_returns_all_traders() {
        let trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();

        let trader2 = TraderConfig::new(
            "def456def456789012345678901234567890def4",
            "Whale2",
        ).unwrap();

        let config = TradersConfig::new(vec![trader1, trader2]);
        let manager = TraderManager::new(&config);

        let all_states = manager.get_all_states();
        assert_eq!(all_states.len(), 2);
    }

    #[test]
    fn test_daily_reset_clears_trades_today() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        // Record some trades
        manager.record_trade("abc123def456789012345678901234567890abcd", 100.0, TradeStatus::Success);
        manager.record_trade("abc123def456789012345678901234567890abcd", 50.0, TradeStatus::Failed);

        // Manually set daily_reset_ts to yesterday
        if let Some(state) = manager.states.get_mut("abc123def456789012345678901234567890abcd") {
            state.daily_reset_ts = Utc::now() - chrono::Duration::days(1);
        }

        // Verify trades_today is set
        let state_before = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state_before.trades_today, 2);

        // Trigger daily reset
        manager.check_daily_reset();

        // Verify trades_today is reset but totals remain
        let state_after = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state_after.trades_today, 0);
        assert_eq!(state_after.successful_trades, 1);
        assert_eq!(state_after.failed_trades, 1);
        assert_eq!(state_after.total_copied_usd, 100.0);
    }

    #[test]
    fn test_daily_reset_does_not_reset_if_same_day() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Test",
        ).unwrap();

        let config = TradersConfig::new(vec![trader]);
        let mut manager = TraderManager::new(&config);

        // Record a trade
        manager.record_trade("abc123def456789012345678901234567890abcd", 100.0, TradeStatus::Success);

        // Trigger daily reset (should not reset since same day)
        manager.check_daily_reset();

        let state = manager.get_state("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(state.trades_today, 1);
    }

    #[test]
    fn test_get_summary_stats_aggregates_correctly() {
        let trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();

        let trader2 = TraderConfig::new(
            "def456def456789012345678901234567890def4",
            "Whale2",
        ).unwrap();

        let config = TradersConfig::new(vec![trader1, trader2]);
        let mut manager = TraderManager::new(&config);

        // Record trades for trader1
        manager.record_trade("abc123def456789012345678901234567890abcd", 100.0, TradeStatus::Success);
        manager.record_trade("abc123def456789012345678901234567890abcd", 50.0, TradeStatus::Failed);

        // Record trades for trader2
        manager.record_trade("def456def456789012345678901234567890def4", 200.0, TradeStatus::Success);
        manager.record_trade("def456def456789012345678901234567890def4", 75.0, TradeStatus::Partial);

        let stats = manager.get_summary_stats();

        assert_eq!(stats.total_traders, 2);
        assert_eq!(stats.total_trades, 4);
        assert_eq!(stats.total_successful, 2);
        assert_eq!(stats.total_failed, 1);
        assert_eq!(stats.total_partial, 1);
        assert_eq!(stats.total_copied_usd, 375.0);
    }
}
