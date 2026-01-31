/// Reloadable configuration wrapper
/// Provides thread-safe access to TradersConfig with hot-reload support

use std::sync::Arc;
use tokio::sync::{RwLock, watch};
use super::traders::TradersConfig;

/// Shared, reloadable traders configuration
///
/// This wrapper allows the configuration to be reloaded at runtime
/// without restarting the application. Components can subscribe to
/// configuration changes via the watch channel.
#[derive(Clone)]
pub struct ReloadableTraders {
    config: Arc<RwLock<TradersConfig>>,
    /// Sender for notifying subscribers of config changes
    /// The value is a generation counter that increments on each reload
    change_tx: watch::Sender<u64>,
    /// Receiver for config change notifications
    change_rx: watch::Receiver<u64>,
}

impl ReloadableTraders {
    /// Creates a new ReloadableTraders from an existing TradersConfig
    pub fn new(config: TradersConfig) -> Self {
        let (change_tx, change_rx) = watch::channel(0u64);
        Self {
            config: Arc::new(RwLock::new(config)),
            change_tx,
            change_rx,
        }
    }

    /// Gets a read lock on the current configuration
    pub async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, TradersConfig> {
        self.config.read().await
    }

    /// Reloads the configuration from disk/environment
    /// Returns Ok(true) if the configuration changed, Ok(false) if unchanged
    pub async fn reload(&self) -> Result<bool, String> {
        let current = self.config.read().await;
        let (new_config, changed) = current.reload()?;
        drop(current);

        if changed {
            let mut write_guard = self.config.write().await;
            *write_guard = new_config;
            drop(write_guard);

            // Notify subscribers of the change
            let new_gen = *self.change_rx.borrow() + 1;
            let _ = self.change_tx.send(new_gen);

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Returns a receiver that notifies when config changes
    /// The receiver yields generation counters (incrementing on each change)
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.change_rx.clone()
    }

    /// Gets the current generation counter
    pub fn generation(&self) -> u64 {
        *self.change_rx.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::traders::TraderConfig;

    fn create_test_config() -> TradersConfig {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "TestTrader"
        ).unwrap();
        TradersConfig::new(vec![trader])
    }

    #[tokio::test]
    async fn test_reloadable_traders_creation() {
        let config = create_test_config();
        let reloadable = ReloadableTraders::new(config);

        let read = reloadable.read().await;
        assert_eq!(read.len(), 1);
        assert_eq!(reloadable.generation(), 0);
    }

    #[tokio::test]
    async fn test_subscribe_returns_receiver() {
        let config = create_test_config();
        let reloadable = ReloadableTraders::new(config);

        let mut rx = reloadable.subscribe();
        assert_eq!(*rx.borrow(), 0);
    }
}
