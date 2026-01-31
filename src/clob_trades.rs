/// CLOB Trade History Module
/// Fetches complete trade history from Polymarket CLOB API
///
/// Provides accurate cost basis, PnL calculations, and position reconciliation

use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use crate::{PreparedCreds, RustClobClient};

const CLOB_API_BASE: &str = "https://clob.polymarket.com";

/// A single trade from the CLOB API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClobTrade {
    pub id: String,
    pub asset_id: String,
    pub market: String,  // condition_id
    pub side: String,    // BUY or SELL
    pub size: f64,
    pub price: f64,
    pub match_time: i64,
    pub transaction_hash: String,
    pub status: String,
    pub trader_side: String,  // MAKER or TAKER
    pub fee_rate_bps: String,
    // Enriched fields (from position data)
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
}

impl ClobTrade {
    pub fn from_json(v: &serde_json::Value, our_address: &str) -> Option<Self> {
        // Only include trades where we are the primary trader (not counterparty)
        // The "maker_address" field shows who initiated the trade
        let maker_address = v["maker_address"].as_str().unwrap_or("");

        // Normalize addresses for comparison
        let our_addr_lower = our_address.to_lowercase();
        let maker_addr_lower = maker_address.to_lowercase();

        // Skip if we're not the maker (we were the counterparty in someone else's trade)
        if !maker_addr_lower.contains(&our_addr_lower[2..].to_lowercase())
            && !our_addr_lower.contains(&maker_addr_lower[2..].to_lowercase())
        {
            return None;
        }

        Some(Self {
            id: v["id"].as_str()?.to_string(),
            asset_id: v["asset_id"].as_str()?.to_string(),
            market: v["market"].as_str()?.to_string(),
            side: v["side"].as_str()?.to_string(),
            size: v["size"].as_str().and_then(|s| s.parse().ok())?,
            price: v["price"].as_str().and_then(|s| s.parse().ok())?,
            match_time: v["match_time"].as_str().and_then(|s| s.parse().ok())?,
            transaction_hash: v["transaction_hash"].as_str().unwrap_or("").to_string(),
            status: v["status"].as_str().unwrap_or("UNKNOWN").to_string(),
            trader_side: v["trader_side"].as_str().unwrap_or("").to_string(),
            fee_rate_bps: v["fee_rate_bps"].as_str().unwrap_or("0").to_string(),
            title: None,
            outcome: v["outcome"].as_str().map(|s| s.to_string()),
        })
    }

    pub fn cost(&self) -> f64 {
        self.size * self.price
    }
}

/// Aggregated position from trades
#[derive(Debug, Clone, Default)]
pub struct PositionFromTrades {
    pub condition_id: String,
    pub asset_id: String,
    pub title: String,
    pub outcome: String,

    // Buy side
    pub total_bought: f64,
    pub total_buy_cost: f64,
    pub buy_count: usize,

    // Sell side
    pub total_sold: f64,
    pub total_sell_revenue: f64,
    pub sell_count: usize,

    // Calculated
    pub net_shares: f64,
    pub avg_buy_price: f64,
    pub realized_pnl: f64,

    // From position API (for reconciliation)
    pub api_shares: Option<f64>,
    pub api_current_value: Option<f64>,
    pub api_avg_price: Option<f64>,
    pub current_price: Option<f64>,

    // From activity API (merges/redeems)
    pub merged_shares: f64,
    pub merged_usdc: f64,
    pub redeemed_shares: f64,
    pub redeemed_usdc: f64,
}

impl PositionFromTrades {
    pub fn new(condition_id: &str, asset_id: &str) -> Self {
        Self {
            condition_id: condition_id.to_string(),
            asset_id: asset_id.to_string(),
            ..Default::default()
        }
    }

    pub fn add_trade(&mut self, trade: &ClobTrade) {
        if trade.side == "BUY" {
            self.total_bought += trade.size;
            self.total_buy_cost += trade.cost();
            self.buy_count += 1;
        } else if trade.side == "SELL" {
            self.total_sold += trade.size;
            self.total_sell_revenue += trade.cost();
            self.sell_count += 1;
        }

        // Update calculated fields
        self.net_shares = self.total_bought - self.total_sold;
        self.avg_buy_price = if self.total_bought > 0.0 {
            self.total_buy_cost / self.total_bought
        } else {
            0.0
        };

        // Realized PnL = revenue from sells - cost basis of sold shares
        // Using FIFO: sold shares came from buys at avg_buy_price
        self.realized_pnl = self.total_sell_revenue - (self.total_sold * self.avg_buy_price);

        // Update title/outcome from trade if not set
        if self.title.is_empty() {
            if let Some(ref t) = trade.title {
                self.title = t.clone();
            }
        }
        if self.outcome.is_empty() {
            if let Some(ref o) = trade.outcome {
                self.outcome = o.clone();
            }
        }
    }

    /// Unrealized PnL based on current price
    pub fn unrealized_pnl(&self) -> f64 {
        if let Some(cur_price) = self.current_price {
            let current_value = self.net_shares * cur_price;
            let cost_basis = self.net_shares * self.avg_buy_price;
            current_value - cost_basis
        } else {
            0.0
        }
    }

    /// Current value of position
    pub fn current_value(&self) -> f64 {
        if let Some(cur_price) = self.current_price {
            self.net_shares * cur_price
        } else {
            self.api_current_value.unwrap_or(0.0)
        }
    }

    /// Total shares explained by trades + merges - redeems
    pub fn explained_shares(&self) -> f64 {
        self.net_shares + self.merged_shares - self.redeemed_shares
    }

    /// Shares not accounted for by trades or activities (truly unexplained)
    pub fn unexplained_shares(&self) -> f64 {
        if let Some(api) = self.api_shares {
            api - self.explained_shares()
        } else {
            0.0
        }
    }

    /// Shares not accounted for by trades alone (before activities)
    pub fn unexplained_by_trades_only(&self) -> f64 {
        if let Some(api) = self.api_shares {
            api - self.net_shares
        } else {
            0.0
        }
    }

    /// Total PnL (realized + unrealized)
    pub fn total_pnl(&self) -> f64 {
        self.realized_pnl + self.unrealized_pnl()
    }
}

/// Summary of all positions
#[derive(Debug, Clone, Default)]
pub struct TradeSummary {
    pub total_trades: usize,
    pub total_positions: usize,
    pub total_buy_volume: f64,
    pub total_sell_volume: f64,
    pub total_realized_pnl: f64,
    pub total_unrealized_pnl: f64,
    pub total_current_value: f64,
    pub positions_with_unexplained: usize,
    pub total_unexplained_shares: f64,
}

/// Fetches all trades from CLOB API
pub fn fetch_all_clob_trades(
    client: &RustClobClient,
    creds: &PreparedCreds,
    wallet_address: &str,
) -> Result<Vec<ClobTrade>> {
    let mut all_trades = Vec::new();
    let mut cursor: Option<String> = None;
    let limit = 500;

    loop {
        let path = "/trades";
        let query = match &cursor {
            Some(c) => format!("?limit={}&cursor={}", limit, c),
            None => format!("?limit={}", limit),
        };
        let url = format!("{}{}{}", CLOB_API_BASE, path, query);

        let headers = client.l2_headers_fast("GET", path, None, creds)?;

        let response = client
            .http_client()
            .get(&url)
            .headers(headers)
            .send()?;

        if !response.status().is_success() {
            return Err(anyhow!("CLOB API error: {}", response.text()?));
        }

        let json: serde_json::Value = response.json()?;
        let data = json["data"].as_array();

        if let Some(trades) = data {
            for trade_json in trades {
                if let Some(trade) = ClobTrade::from_json(trade_json, wallet_address) {
                    all_trades.push(trade);
                }
            }

            // Check for next cursor
            if let Some(next) = json.get("next_cursor").and_then(|c| c.as_str()) {
                if !next.is_empty() && trades.len() >= limit {
                    cursor = Some(next.to_string());
                    continue;
                }
            }
        }

        break;
    }

    // Sort by match_time ascending (oldest first)
    all_trades.sort_by_key(|t| t.match_time);

    Ok(all_trades)
}

/// Groups trades into positions and calculates PnL
pub fn build_positions_from_trades(trades: &[ClobTrade]) -> HashMap<String, PositionFromTrades> {
    let mut positions: HashMap<String, PositionFromTrades> = HashMap::new();

    for trade in trades {
        let key = format!("{}:{}", trade.market, trade.asset_id);

        let position = positions
            .entry(key)
            .or_insert_with(|| PositionFromTrades::new(&trade.market, &trade.asset_id));

        position.add_trade(trade);
    }

    positions
}

/// Enriches positions with data from the position API
pub fn enrich_with_position_api(
    positions: &mut HashMap<String, PositionFromTrades>,
    api_positions: &[crate::live_positions::LivePosition],
) {
    // Build lookup by condition_id:asset_id (same key format as positions HashMap)
    // This ensures we match Yes/No outcomes correctly
    let mut api_lookup: HashMap<String, &crate::live_positions::LivePosition> = HashMap::new();
    for pos in api_positions {
        // The asset field contains the token ID (same as asset_id in trades)
        let key = format!("{}:{}", pos.condition_id, pos.asset);
        api_lookup.insert(key, pos);
    }

    for (key, position) in positions.iter_mut() {
        if let Some(api_pos) = api_lookup.get(key) {
            position.api_shares = Some(api_pos.size);
            position.api_current_value = Some(api_pos.current_value);
            position.api_avg_price = Some(api_pos.avg_price);
            position.current_price = Some(api_pos.cur_price);
            position.title = api_pos.title.clone();
            position.outcome = api_pos.outcome.clone();
        }
    }
}

/// Enriches positions with merge/redeem data from activities
pub fn enrich_with_activities(
    positions: &mut HashMap<String, PositionFromTrades>,
    activities: &[Activity],
) {
    for activity in activities {
        // Build key matching format: condition_id:asset_id
        let key = format!("{}:{}", activity.condition_id, activity.asset);

        // Find matching position by condition_id AND asset_id
        if let Some(position) = positions.get_mut(&key) {
            match activity.activity_type {
                ActivityType::Merge => {
                    position.merged_shares += activity.size;
                    position.merged_usdc += activity.usdc_size;
                }
                ActivityType::Redeem => {
                    position.redeemed_shares += activity.size;
                    position.redeemed_usdc += activity.usdc_size;
                }
                ActivityType::Trade => {
                    // Trades are already tracked via CLOB API
                }
            }
        }
    }
}

/// Calculates summary statistics
pub fn calculate_summary(positions: &HashMap<String, PositionFromTrades>, total_trades: usize) -> TradeSummary {
    let mut summary = TradeSummary {
        total_trades,
        total_positions: positions.len(),
        ..Default::default()
    };

    for pos in positions.values() {
        summary.total_buy_volume += pos.total_buy_cost;
        summary.total_sell_volume += pos.total_sell_revenue;
        summary.total_realized_pnl += pos.realized_pnl;
        summary.total_unrealized_pnl += pos.unrealized_pnl();
        summary.total_current_value += pos.current_value();

        let unexplained = pos.unexplained_shares();
        if unexplained.abs() > 0.01 {
            summary.positions_with_unexplained += 1;
            summary.total_unexplained_shares += unexplained;
        }
    }

    summary
}

// ============================================================================
// Activity Types (MERGE, REDEEM, TRADE) from Data API
// ============================================================================

const DATA_API_BASE: &str = "https://data-api.polymarket.com";

/// Activity type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ActivityType {
    Trade,
    Merge,
    Redeem,
}

impl std::fmt::Display for ActivityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivityType::Trade => write!(f, "TRADE"),
            ActivityType::Merge => write!(f, "MERGE"),
            ActivityType::Redeem => write!(f, "REDEEM"),
        }
    }
}

/// An activity from the Data API (includes TRADE, MERGE, REDEEM)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// Activity type: TRADE, MERGE, or REDEEM
    #[serde(rename = "type")]
    pub activity_type: ActivityType,

    /// Proxy wallet address
    #[serde(default)]
    pub proxy_wallet: String,

    /// Condition ID (market identifier)
    #[serde(default)]
    pub condition_id: String,

    /// Asset ID (token identifier)
    #[serde(default)]
    pub asset: String,

    /// Transaction hash
    #[serde(default)]
    pub transaction_hash: String,

    /// Side (BUY/SELL for trades)
    #[serde(default)]
    pub side: String,

    /// Number of shares
    #[serde(default)]
    pub size: f64,

    /// Price per share
    #[serde(default)]
    pub price: f64,

    /// Fee amount
    #[serde(default)]
    pub fee: f64,

    /// USDC amount (for merges/redeems)
    #[serde(default)]
    pub usdc_size: f64,

    /// Market title
    #[serde(default)]
    pub title: String,

    /// Market slug
    #[serde(default)]
    pub slug: String,

    /// Market icon
    #[serde(default)]
    pub icon: String,

    /// Outcome name
    #[serde(default)]
    pub outcome: String,

    /// Timestamp
    #[serde(default)]
    pub timestamp: i64,
}

impl Activity {
    /// Returns the effective value of this activity
    pub fn value(&self) -> f64 {
        match self.activity_type {
            ActivityType::Trade => self.size * self.price,
            ActivityType::Merge | ActivityType::Redeem => self.usdc_size,
        }
    }

    /// Returns a description of this activity for display
    pub fn description(&self) -> String {
        match self.activity_type {
            ActivityType::Trade => {
                format!("{} {:.2} shares @ ${:.4}", self.side, self.size, self.price)
            }
            ActivityType::Merge => {
                format!("Merged {:.2} shares → ${:.2}", self.size, self.usdc_size)
            }
            ActivityType::Redeem => {
                format!("Redeemed {:.2} shares → ${:.2}", self.size, self.usdc_size)
            }
        }
    }
}

/// Fetches all activities from Data API (includes TRADE, MERGE, REDEEM)
pub fn fetch_all_activities(wallet_address: &str) -> Result<Vec<Activity>> {
    let mut all_activities = Vec::new();
    let limit = 500;
    let mut offset = 0;

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    loop {
        let url = format!(
            "{}/activity?user={}&limit={}&offset={}",
            DATA_API_BASE, wallet_address, limit, offset
        );

        let response = client
            .get(&url)
            .header("Accept", "application/json")
            .send()?;

        if !response.status().is_success() {
            return Err(anyhow!("Data API error: {}", response.text()?));
        }

        let activities: Vec<Activity> = response.json()?;
        let count = activities.len();
        all_activities.extend(activities);

        if count < limit {
            break;
        }

        offset += limit;

        // Safety limit
        if offset > 10000 {
            break;
        }
    }

    // Sort by timestamp (oldest first)
    all_activities.sort_by_key(|a| a.timestamp);

    Ok(all_activities)
}

/// Fetches only MERGE and REDEEM activities
pub fn fetch_merge_and_redeem_activities(wallet_address: &str) -> Result<Vec<Activity>> {
    let all = fetch_all_activities(wallet_address)?;
    Ok(all
        .into_iter()
        .filter(|a| a.activity_type == ActivityType::Merge || a.activity_type == ActivityType::Redeem)
        .collect())
}

/// Summary of merge/redeem activities
#[derive(Debug, Clone, Default)]
pub struct ActivitySummary {
    pub total_activities: usize,
    pub trade_count: usize,
    pub merge_count: usize,
    pub redeem_count: usize,
    pub total_merged_usdc: f64,
    pub total_redeemed_usdc: f64,
}

impl ActivitySummary {
    pub fn from_activities(activities: &[Activity]) -> Self {
        let mut summary = Self {
            total_activities: activities.len(),
            ..Default::default()
        };

        for activity in activities {
            match activity.activity_type {
                ActivityType::Trade => summary.trade_count += 1,
                ActivityType::Merge => {
                    summary.merge_count += 1;
                    summary.total_merged_usdc += activity.usdc_size;
                }
                ActivityType::Redeem => {
                    summary.redeem_count += 1;
                    summary.total_redeemed_usdc += activity.usdc_size;
                }
            }
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_from_trades_buy() {
        let mut pos = PositionFromTrades::new("cond1", "asset1");

        let trade = ClobTrade {
            id: "t1".to_string(),
            asset_id: "asset1".to_string(),
            market: "cond1".to_string(),
            side: "BUY".to_string(),
            size: 100.0,
            price: 0.50,
            match_time: 1000,
            transaction_hash: "0x123".to_string(),
            status: "CONFIRMED".to_string(),
            trader_side: "TAKER".to_string(),
            fee_rate_bps: "0".to_string(),
            title: Some("Test".to_string()),
            outcome: Some("Yes".to_string()),
        };

        pos.add_trade(&trade);

        assert_eq!(pos.total_bought, 100.0);
        assert_eq!(pos.total_buy_cost, 50.0);
        assert_eq!(pos.net_shares, 100.0);
        assert_eq!(pos.avg_buy_price, 0.50);
    }

    #[test]
    fn test_position_from_trades_buy_sell() {
        let mut pos = PositionFromTrades::new("cond1", "asset1");

        // Buy 100 @ 0.50
        pos.add_trade(&ClobTrade {
            id: "t1".to_string(),
            asset_id: "asset1".to_string(),
            market: "cond1".to_string(),
            side: "BUY".to_string(),
            size: 100.0,
            price: 0.50,
            match_time: 1000,
            transaction_hash: "".to_string(),
            status: "CONFIRMED".to_string(),
            trader_side: "".to_string(),
            fee_rate_bps: "0".to_string(),
            title: None,
            outcome: None,
        });

        // Sell 50 @ 0.70
        pos.add_trade(&ClobTrade {
            id: "t2".to_string(),
            asset_id: "asset1".to_string(),
            market: "cond1".to_string(),
            side: "SELL".to_string(),
            size: 50.0,
            price: 0.70,
            match_time: 2000,
            transaction_hash: "".to_string(),
            status: "CONFIRMED".to_string(),
            trader_side: "".to_string(),
            fee_rate_bps: "0".to_string(),
            title: None,
            outcome: None,
        });

        assert_eq!(pos.total_bought, 100.0);
        assert_eq!(pos.total_sold, 50.0);
        assert_eq!(pos.net_shares, 50.0);
        assert_eq!(pos.avg_buy_price, 0.50);
        // Realized PnL = 50 * 0.70 - 50 * 0.50 = 35 - 25 = 10
        assert!((pos.realized_pnl - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_unrealized_pnl() {
        let mut pos = PositionFromTrades::new("cond1", "asset1");
        pos.total_bought = 100.0;
        pos.total_buy_cost = 50.0;
        pos.net_shares = 100.0;
        pos.avg_buy_price = 0.50;
        pos.current_price = Some(0.80);

        // Unrealized = 100 * 0.80 - 100 * 0.50 = 80 - 50 = 30
        assert!((pos.unrealized_pnl() - 30.0).abs() < 0.001);
    }
}
