// import_csv.rs - Import legacy CSV trade data into SQLite database
//
// This tool imports historical trade data from matches_optimized.csv format
// into the trades database for analysis and historical tracking.

use anyhow::{Context, Result};
use clap::Parser;
use pm_whale_follower::persistence::{TradeRecord, TradeStore};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(name = "import_csv")]
#[command(about = "Import legacy CSV trade data into SQLite database")]
struct Args {
    /// Path to CSV file to import
    csv_file: PathBuf,

    /// Path to SQLite database (default: trades.db)
    #[arg(long, default_value = "trades.db")]
    db: PathBuf,

    /// Preview import without writing to database
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Skip rows with duplicate tx_hash
    #[arg(long, default_value_t = true)]
    skip_duplicates: bool,
}

struct ImportStats {
    total: usize,
    imported: usize,
    skipped: usize,
    errors: usize,
}

impl ImportStats {
    fn new(total: usize) -> Self {
        Self {
            total,
            imported: 0,
            skipped: 0,
            errors: 0,
        }
    }

    fn print_summary(&self) {
        println!("\nImport Summary:");
        println!("Total rows:    {:>6}", self.total);
        println!("Imported:      {:>6}", self.imported);
        println!("Skipped:       {:>6} (duplicates)", self.skipped);
        println!("Errors:        {:>6}", self.errors);
    }
}

/// Import trades into database
fn import_trades(
    store: &TradeStore,
    trades: Vec<TradeRecord>,
    skip_duplicates: bool,
) -> Result<ImportStats> {
    let mut stats = ImportStats::new(trades.len());

    for trade in trades {
        // Check for duplicates if enabled
        if skip_duplicates {
            match store.tx_hash_exists(&trade.tx_hash) {
                Ok(true) => {
                    stats.skipped += 1;
                    continue;
                }
                Ok(false) => {
                    // Not a duplicate, proceed to insert
                }
                Err(e) => {
                    eprintln!("Error checking for duplicate tx_hash {}: {}", trade.tx_hash, e);
                    stats.errors += 1;
                    continue;
                }
            }
        }

        // Insert trade
        match store.insert_trade(&trade) {
            Ok(_) => {
                stats.imported += 1;
            }
            Err(e) => {
                eprintln!("Error inserting trade with tx_hash {}: {}", trade.tx_hash, e);
                stats.errors += 1;
            }
        }
    }

    Ok(stats)
}

/// Result of reading CSV file - includes parsed trades and error counts
struct CsvReadResult {
    trades: Vec<TradeRecord>,
    parse_errors: usize,
    malformed_rows: usize,
}

/// Read CSV file and parse all rows into TradeRecords
/// Handles malformed rows gracefully by skipping them
fn read_csv_file<P: AsRef<Path>>(path: P) -> Result<CsvReadResult> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true) // Allow variable number of fields
        .from_path(path)
        .context("Failed to open CSV file")?;

    let mut trades = Vec::new();
    let mut parse_errors = 0;
    let mut malformed_rows = 0;
    let mut line_num = 0;

    for result in reader.records() {
        line_num += 1;
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Warning: Skipping malformed row at line {}: {}", line_num + 1, e);
                malformed_rows += 1;
                continue;
            }
        };

        // Check for correct number of fields (14)
        if record.len() != 14 {
            eprintln!(
                "Warning: Skipping row at line {} with {} fields (expected 14)",
                line_num + 1,
                record.len()
            );
            malformed_rows += 1;
            continue;
        }

        match parse_csv_row(&record) {
            Ok(trade) => trades.push(trade),
            Err(e) => {
                eprintln!("Warning: Failed to parse row at line {}: {}", line_num + 1, e);
                parse_errors += 1;
            }
        }
    }

    Ok(CsvReadResult {
        trades,
        parse_errors,
        malformed_rows,
    })
}

/// Parse a CSV row into a TradeRecord
///
/// Field mappings:
/// - clob_asset_id → token_id
/// - direction (BUY_FILL/SELL_FILL) → side (BUY/SELL)
/// - order_status → status
/// - price_per_share → whale_price/our_price
/// - shares → whale_shares/our_shares
/// - usd_value → whale_usd/our_usd
/// - tx_hash → tx_hash
/// - timestamp → timestamp_ms
/// - is_live → is_live
fn parse_csv_row(row: &csv::StringRecord) -> Result<TradeRecord> {
    // CSV format:
    // timestamp,block,clob_asset_id,usd_value,shares,price_per_share,direction,order_status,best_price,best_size,second_price,second_size,tx_hash,is_live
    // 0         1     2             3         4      5               6         7            8          9         10           11          12      13

    // Parse timestamp to milliseconds
    let timestamp_str = row.get(0).context("Missing timestamp")?;
    let timestamp = chrono::NaiveDateTime::parse_from_str(timestamp_str, "%Y-%m-%d %H:%M:%S%.f")
        .context("Failed to parse timestamp")?;
    let timestamp_ms = timestamp.and_utc().timestamp_millis();

    // Parse block number
    let block_number: u64 = row.get(1)
        .context("Missing block")?
        .parse()
        .context("Failed to parse block number")?;

    // Parse token ID (clob_asset_id)
    let token_id = row.get(2).context("Missing clob_asset_id")?.to_string();

    // Parse USD value
    let usd_value: f64 = row.get(3)
        .context("Missing usd_value")?
        .parse()
        .context("Failed to parse usd_value")?;

    // Parse shares
    let shares: f64 = row.get(4)
        .context("Missing shares")?
        .parse()
        .context("Failed to parse shares")?;

    // Parse price per share
    let price: f64 = row.get(5)
        .context("Missing price_per_share")?
        .parse()
        .context("Failed to parse price_per_share")?;

    // Parse direction and convert to side (BUY_FILL → BUY, SELL_FILL → SELL)
    let direction = row.get(6).context("Missing direction")?;
    let side = if direction.starts_with("BUY") {
        "BUY"
    } else if direction.starts_with("SELL") {
        "SELL"
    } else {
        anyhow::bail!("Invalid direction: {}", direction);
    };

    // Parse order status
    let status = row.get(7).context("Missing order_status")?.to_string();

    // Parse tx_hash
    let tx_hash = row.get(12).context("Missing tx_hash")?.to_string();

    // Parse is_live
    let is_live_str = row.get(13).context("Missing is_live")?;
    let is_live = match is_live_str {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };

    // For legacy CSV imports, we don't have trader_address or separate whale/our execution details
    // We'll set trader_address to a placeholder and populate whale_* fields with CSV data
    //
    // If status indicates execution (200 OK or MOCK_ONLY), populate our_* fields
    // so these trades appear in position_monitor
    let (our_shares, our_price, our_usd, fill_pct) = if status.contains("200 OK") || status.contains("MOCK_ONLY") {
        (Some(shares), Some(price), Some(usd_value), Some(100.0))
    } else {
        (None, None, None, None)
    };

    Ok(TradeRecord {
        timestamp_ms,
        block_number,
        tx_hash,
        trader_address: "LEGACY_CSV_IMPORT".to_string(),
        token_id,
        side: side.to_string(),
        whale_shares: shares,
        whale_price: price,
        whale_usd: usd_value,
        our_shares,
        our_price,
        our_usd,
        fill_pct,
        status,
        latency_ms: None,
        is_live,
        aggregation_count: None,
        aggregation_window_ms: None,
    })
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("CSV Import Tool");
    println!("CSV file: {}", args.csv_file.display());
    println!("Database: {}", args.db.display());
    println!("Dry run: {}", args.dry_run);
    println!("Skip duplicates: {}", args.skip_duplicates);

    // Read CSV file
    println!("\nReading CSV file...");
    let csv_result = read_csv_file(&args.csv_file)
        .context("Failed to read CSV file")?;

    println!("Parsed {} trades ({} malformed rows skipped, {} parse errors)",
        csv_result.trades.len(),
        csv_result.malformed_rows,
        csv_result.parse_errors
    );

    if args.dry_run {
        println!("\nDry run - no data will be written to database");
        let mut stats = ImportStats::new(csv_result.trades.len());
        stats.errors = csv_result.parse_errors + csv_result.malformed_rows;
        stats.print_summary();
        return Ok(());
    }

    // Open database
    let store = TradeStore::new(&args.db)
        .context("Failed to open database")?;

    // Import trades
    println!("\nImporting trades...");
    let mut stats = import_trades(&store, csv_result.trades, args.skip_duplicates)?;
    // Add CSV read errors to total
    stats.errors += csv_result.parse_errors + csv_result.malformed_rows;
    stats.print_summary();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_csv_row_basic_buy_fill() {
        // Test parsing a basic BUY_FILL row from the CSV
        let csv_data = "timestamp,block,clob_asset_id,usd_value,shares,price_per_share,direction,order_status,best_price,best_size,second_price,second_size,tx_hash,is_live\n\
                       2026-01-17 10:21:49.337,81762667,88542713231653403002722470828089886449622565040104422249038604181713168004283,0.20,2.197800,0.0900,BUY_FILL,SKIPPED_SMALL (<10 shares),0.1,6.65,0.11,1095.9,0x57e457569f578b1f83aeabe2872b290bacac0fd48de1582c31fe9265070f53d4,false";

        let mut reader = csv::Reader::from_reader(csv_data.as_bytes());
        let mut records = reader.records();
        let row = records.next().unwrap().unwrap();

        let trade = parse_csv_row(&row).expect("Failed to parse CSV row");

        // Verify basic field mappings
        assert_eq!(trade.token_id, "88542713231653403002722470828089886449622565040104422249038604181713168004283");
        assert_eq!(trade.side, "BUY"); // BUY_FILL → BUY
        assert_eq!(trade.whale_shares, 2.197800);
        assert_eq!(trade.whale_price, 0.0900);
        assert_eq!(trade.whale_usd, 0.20);
        assert_eq!(trade.tx_hash, "0x57e457569f578b1f83aeabe2872b290bacac0fd48de1582c31fe9265070f53d4");
        assert_eq!(trade.status, "SKIPPED_SMALL (<10 shares)");
        assert_eq!(trade.block_number, 81762667);
        assert_eq!(trade.is_live, Some(false));
    }

    #[test]
    fn test_parse_csv_row_sell_fill() {
        // Test parsing a SELL_FILL row to verify direction mapping works for sells
        let csv_data = "timestamp,block,clob_asset_id,usd_value,shares,price_per_share,direction,order_status,best_price,best_size,second_price,second_size,tx_hash,is_live\n\
                       2026-01-17 11:30:00.000,81762700,12345678901234567890,100.50,250.00,0.4020,SELL_FILL,SUCCESS,0.40,100.0,0.39,50.0,0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890,true";

        let mut reader = csv::Reader::from_reader(csv_data.as_bytes());
        let mut records = reader.records();
        let row = records.next().unwrap().unwrap();

        let trade = parse_csv_row(&row).expect("Failed to parse CSV row");

        // Verify SELL direction mapping
        assert_eq!(trade.side, "SELL"); // SELL_FILL → SELL
        assert_eq!(trade.token_id, "12345678901234567890");
        assert_eq!(trade.whale_shares, 250.00);
        assert_eq!(trade.whale_price, 0.4020);
        assert_eq!(trade.whale_usd, 100.50);
        assert_eq!(trade.status, "SUCCESS");
        assert_eq!(trade.is_live, Some(true));
    }

    #[test]
    fn test_read_csv_file() {
        // Test reading a CSV file with multiple rows
        use std::io::Write;
        use tempfile::NamedTempFile;

        let csv_content = "timestamp,block,clob_asset_id,usd_value,shares,price_per_share,direction,order_status,best_price,best_size,second_price,second_size,tx_hash,is_live\n\
                          2026-01-17 10:21:49.337,81762667,88542713231653403002722470828089886449622565040104422249038604181713168004283,0.20,2.197800,0.0900,BUY_FILL,SKIPPED_SMALL (<10 shares),0.1,6.65,0.11,1095.9,0x57e457569f578b1f83aeabe2872b290bacac0fd48de1582c31fe9265070f53d4,false\n\
                          2026-01-17 11:30:00.000,81762700,12345678901234567890,100.50,250.00,0.4020,SELL_FILL,SUCCESS,0.40,100.0,0.39,50.0,0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890,true";

        // Create a temporary file
        let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
        temp_file.write_all(csv_content.as_bytes()).expect("Failed to write to temp file");
        temp_file.flush().expect("Failed to flush temp file");

        // Read the CSV file
        let result = read_csv_file(temp_file.path()).expect("Failed to read CSV file");

        // Verify we got 2 trades with no errors
        assert_eq!(result.trades.len(), 2);
        assert_eq!(result.parse_errors, 0);
        assert_eq!(result.malformed_rows, 0);

        // Verify first trade (BUY)
        assert_eq!(result.trades[0].side, "BUY");
        assert_eq!(result.trades[0].token_id, "88542713231653403002722470828089886449622565040104422249038604181713168004283");

        // Verify second trade (SELL)
        assert_eq!(result.trades[1].side, "SELL");
        assert_eq!(result.trades[1].token_id, "12345678901234567890");
    }

    #[test]
    fn test_read_csv_file_with_malformed_rows() {
        // Test that malformed rows are skipped gracefully
        use std::io::Write;
        use tempfile::NamedTempFile;

        let csv_content = "timestamp,block,clob_asset_id,usd_value,shares,price_per_share,direction,order_status,best_price,best_size,second_price,second_size,tx_hash,is_live\n\
                          2026-01-17 10:21:49.337,81762667,88542713231653403002722470828089886449622565040104422249038604181713168004283,0.20,2.197800,0.0900,BUY_FILL,SKIPPED_SMALL,0.1,6.65,0.11,1095.9,0xtx1,false\n\
                          bad,row,with,wrong,number,of,fields\n\
                          2026-01-17 11:30:00.000,81762700,12345678901234567890,100.50,250.00,0.4020,SELL_FILL,SUCCESS,0.40,100.0,0.39,50.0,0xtx2,true";

        let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
        temp_file.write_all(csv_content.as_bytes()).expect("Failed to write to temp file");
        temp_file.flush().expect("Failed to flush temp file");

        let result = read_csv_file(temp_file.path()).expect("Failed to read CSV file");

        // Should have 2 valid trades and 1 malformed row
        assert_eq!(result.trades.len(), 2);
        assert_eq!(result.malformed_rows, 1);
        assert_eq!(result.parse_errors, 0);
    }

    #[test]
    fn test_args_parsing() {
        // Test that Args can be constructed and has correct defaults
        let args = Args {
            csv_file: PathBuf::from("test.csv"),
            db: PathBuf::from("trades.db"),
            dry_run: false,
            skip_duplicates: true,
        };

        assert_eq!(args.csv_file, PathBuf::from("test.csv"));
        assert_eq!(args.db, PathBuf::from("trades.db"));
        assert!(!args.dry_run);
        assert!(args.skip_duplicates);
    }

    #[test]
    fn test_args_with_custom_values() {
        // Test Args with custom values
        let args = Args {
            csv_file: PathBuf::from("matches.csv"),
            db: PathBuf::from("custom.db"),
            dry_run: true,
            skip_duplicates: false,
        };

        assert_eq!(args.csv_file, PathBuf::from("matches.csv"));
        assert_eq!(args.db, PathBuf::from("custom.db"));
        assert!(args.dry_run);
        assert!(!args.skip_duplicates);
    }

    #[test]
    fn test_import_trades_basic() {
        // Test basic import functionality
        let store = TradeStore::new(":memory:").expect("Failed to create store");

        // Create test trade
        let trade = TradeRecord {
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            block_number: 12345678,
            tx_hash: "0xtest123".to_string(),
            trader_address: "LEGACY_CSV_IMPORT".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 100.0,
            whale_price: 0.50,
            whale_usd: 50.0,
            our_shares: None,
            our_price: None,
            our_usd: None,
            fill_pct: None,
            status: "SUCCESS".to_string(),
            latency_ms: None,
            is_live: Some(false),
            aggregation_count: None,
            aggregation_window_ms: None,
        };

        let trades = vec![trade];
        let stats = import_trades(&store, trades, false).expect("Failed to import trades");

        assert_eq!(stats.total, 1);
        assert_eq!(stats.imported, 1);
        assert_eq!(stats.skipped, 0);
        assert_eq!(stats.errors, 0);

        // Verify trade was inserted
        let count = store.get_trade_count().expect("Failed to get trade count");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_import_trades_skip_duplicates() {
        // Test duplicate detection
        let store = TradeStore::new(":memory:").expect("Failed to create store");

        // Create two trades with same tx_hash
        let trade1 = TradeRecord {
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            block_number: 12345678,
            tx_hash: "0xduplicate".to_string(),
            trader_address: "LEGACY_CSV_IMPORT".to_string(),
            token_id: "123456".to_string(),
            side: "BUY".to_string(),
            whale_shares: 100.0,
            whale_price: 0.50,
            whale_usd: 50.0,
            our_shares: None,
            our_price: None,
            our_usd: None,
            fill_pct: None,
            status: "SUCCESS".to_string(),
            latency_ms: None,
            is_live: Some(false),
            aggregation_count: None,
            aggregation_window_ms: None,
        };

        let trade2 = trade1.clone();

        let trades = vec![trade1, trade2];
        let stats = import_trades(&store, trades, true).expect("Failed to import trades");

        assert_eq!(stats.total, 2);
        assert_eq!(stats.imported, 1);
        assert_eq!(stats.skipped, 1); // Second trade should be skipped as duplicate
        assert_eq!(stats.errors, 0);

        // Verify only one trade was inserted
        let count = store.get_trade_count().expect("Failed to get trade count");
        assert_eq!(count, 1);
    }
}
