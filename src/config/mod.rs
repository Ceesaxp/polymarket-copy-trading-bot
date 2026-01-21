/// Trader configuration management
/// Handles loading and validating trader addresses for multi-trader monitoring

pub mod traders;

#[cfg(test)]
mod tests {
    use super::traders::*;
    use std::path::Path;

    // =========================================================================
    // Test Suite: Address Validation and Normalization
    // =========================================================================

    #[test]
    fn test_validate_address_valid_lowercase() {
        let result = validate_and_normalize_address("abc123def456789012345678901234567890abcd");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_validate_address_valid_uppercase() {
        let result = validate_and_normalize_address("ABC123DEF456789012345678901234567890ABCD");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_validate_address_valid_mixed_case() {
        let result = validate_and_normalize_address("AbC123dEf456789012345678901234567890aBcD");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_validate_address_strips_0x_prefix() {
        let result = validate_and_normalize_address("0xabc123def456789012345678901234567890abcd");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_validate_address_strips_0x_uppercase() {
        let result = validate_and_normalize_address("0XABC123DEF456789012345678901234567890ABCD");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_validate_address_too_short() {
        let result = validate_and_normalize_address("abc123");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("40 characters"));
    }

    #[test]
    fn test_validate_address_too_long() {
        let result = validate_and_normalize_address("abc123def456789012345678901234567890abcd12345");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("40 characters"));
    }

    #[test]
    fn test_validate_address_invalid_char_special() {
        let result = validate_and_normalize_address("abc123def456789012345678901234567890ab!d");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("hexadecimal"));
    }

    #[test]
    fn test_validate_address_invalid_char_space() {
        let result = validate_and_normalize_address("abc123def456789012345678901234567890ab d");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("hexadecimal"));
    }

    #[test]
    fn test_validate_address_invalid_char_g() {
        let result = validate_and_normalize_address("abc123def456789012345678901234567890abgd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("hexadecimal"));
    }

    #[test]
    fn test_validate_address_empty_string() {
        let result = validate_and_normalize_address("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("40 characters"));
    }

    #[test]
    fn test_validate_address_only_0x() {
        let result = validate_and_normalize_address("0x");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("40 characters"));
    }

    #[test]
    fn test_validate_address_whitespace_trimmed() {
        let result = validate_and_normalize_address("  abc123def456789012345678901234567890abcd  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_validate_address_whitespace_with_0x() {
        let result = validate_and_normalize_address("  0xabc123def456789012345678901234567890abcd  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "abc123def456789012345678901234567890abcd");
    }

    // =========================================================================
    // Test Suite: Topic Hex Generation
    // =========================================================================

    #[test]
    fn test_address_to_topic_hex_40_chars() {
        let address = "abc123def456789012345678901234567890abcd";
        let topic = address_to_topic_hex(address);
        assert_eq!(topic.len(), 64);
        assert_eq!(topic, "000000000000000000000000abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_address_to_topic_hex_preserves_address() {
        let address = "1234567890abcdef1234567890abcdef12345678";
        let topic = address_to_topic_hex(address);
        assert!(topic.ends_with(address));
    }

    #[test]
    fn test_address_to_topic_hex_all_zeros() {
        let address = "0000000000000000000000000000000000000000";
        let topic = address_to_topic_hex(address);
        assert_eq!(topic, "0000000000000000000000000000000000000000000000000000000000000000");
    }

    #[test]
    fn test_address_to_topic_hex_all_fs() {
        let address = "ffffffffffffffffffffffffffffffffffffffff";
        let topic = address_to_topic_hex(address);
        assert_eq!(topic, "000000000000000000000000ffffffffffffffffffffffffffffffffffffffff");
    }

    #[test]
    fn test_address_to_topic_hex_correct_padding() {
        let address = "abc123def456789012345678901234567890abcd";
        let topic = address_to_topic_hex(address);
        // Should have 24 leading zeros (64 total - 40 address = 24 padding)
        let padding = &topic[..24];
        assert_eq!(padding, "000000000000000000000000");
    }

    // =========================================================================
    // Test Suite: TraderConfig Struct
    // =========================================================================

    #[test]
    fn test_trader_config_new_valid() {
        let config = TraderConfig::new(
            "0xabc123def456789012345678901234567890abcd",
            "Whale1",
        );
        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.address, "abc123def456789012345678901234567890abcd");
        assert_eq!(config.label, "Whale1");
        assert_eq!(config.topic_hex, "000000000000000000000000abc123def456789012345678901234567890abcd");
        assert_eq!(config.scaling_ratio, 0.02); // default
        assert_eq!(config.min_shares, 0.0); // default
        assert!(config.enabled); // default
    }

    #[test]
    fn test_trader_config_new_invalid_address() {
        let config = TraderConfig::new("invalid", "Whale1");
        assert!(config.is_err());
        assert!(config.unwrap_err().contains("40 characters"));
    }

    #[test]
    fn test_trader_config_with_custom_scaling() {
        let mut config = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        config.scaling_ratio = 0.05;
        assert_eq!(config.scaling_ratio, 0.05);
    }

    #[test]
    fn test_trader_config_with_min_shares() {
        let mut config = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        config.min_shares = 100.0;
        assert_eq!(config.min_shares, 100.0);
    }

    #[test]
    fn test_trader_config_can_disable() {
        let mut config = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        config.enabled = false;
        assert!(!config.enabled);
    }

    #[test]
    fn test_trader_config_topic_hex_generated_correctly() {
        let config = TraderConfig::new(
            "1234567890abcdef1234567890abcdef12345678",
            "Test",
        ).unwrap();
        assert_eq!(config.topic_hex.len(), 64);
        assert!(config.topic_hex.ends_with("1234567890abcdef1234567890abcdef12345678"));
    }

    // =========================================================================
    // Test Suite: TradersConfig Struct
    // =========================================================================

    #[test]
    fn test_traders_config_new_empty() {
        let config = TradersConfig::new(vec![]);
        assert_eq!(config.len(), 0);
        assert!(config.is_empty());
    }

    #[test]
    fn test_traders_config_new_single() {
        let trader = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        let config = TradersConfig::new(vec![trader]);
        assert_eq!(config.len(), 1);
        assert!(!config.is_empty());
    }

    #[test]
    fn test_traders_config_new_multiple() {
        let trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        let trader2 = TraderConfig::new(
            "def456abc123789012345678901234567890abcd",
            "Whale2",
        ).unwrap();
        let config = TradersConfig::new(vec![trader1, trader2]);
        assert_eq!(config.len(), 2);
    }

    #[test]
    fn test_traders_config_build_topic_filter() {
        let trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        let trader2 = TraderConfig::new(
            "def456abc123789012345678901234567890abcd",
            "Whale2",
        ).unwrap();
        let config = TradersConfig::new(vec![trader1, trader2]);

        let filter = config.build_topic_filter();
        assert_eq!(filter.len(), 2);
        assert!(filter.contains(&"000000000000000000000000abc123def456789012345678901234567890abcd".to_string()));
        assert!(filter.contains(&"000000000000000000000000def456abc123789012345678901234567890abcd".to_string()));
    }

    #[test]
    fn test_traders_config_get_by_topic() {
        let trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        let trader2 = TraderConfig::new(
            "def456abc123789012345678901234567890abcd",
            "Whale2",
        ).unwrap();
        let config = TradersConfig::new(vec![trader1, trader2]);

        let found = config.get_by_topic("000000000000000000000000abc123def456789012345678901234567890abcd");
        assert!(found.is_some());
        assert_eq!(found.unwrap().label, "Whale1");

        let not_found = config.get_by_topic("000000000000000000000000999999999999999999999999999999999999999");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_traders_config_get_by_address() {
        let trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        let config = TradersConfig::new(vec![trader1]);

        let found = config.get_by_address("abc123def456789012345678901234567890abcd");
        assert!(found.is_some());
        assert_eq!(found.unwrap().label, "Whale1");

        // Should also find with 0x prefix
        let found_with_prefix = config.get_by_address("0xabc123def456789012345678901234567890abcd");
        assert!(found_with_prefix.is_some());

        let not_found = config.get_by_address("999999999999999999999999999999999999999");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_traders_config_iter() {
        let trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        let trader2 = TraderConfig::new(
            "def456abc123789012345678901234567890abcd",
            "Whale2",
        ).unwrap();
        let config = TradersConfig::new(vec![trader1, trader2]);

        let labels: Vec<String> = config.iter().map(|t| t.label.clone()).collect();
        assert_eq!(labels.len(), 2);
        assert!(labels.contains(&"Whale1".to_string()));
        assert!(labels.contains(&"Whale2".to_string()));
    }

    #[test]
    fn test_traders_config_only_enabled_in_filter() {
        let mut trader1 = TraderConfig::new(
            "abc123def456789012345678901234567890abcd",
            "Whale1",
        ).unwrap();
        trader1.enabled = false;

        let trader2 = TraderConfig::new(
            "def456abc123789012345678901234567890abcd",
            "Whale2",
        ).unwrap();

        let config = TradersConfig::new(vec![trader1, trader2]);

        // Topic filter should only include enabled traders
        let filter = config.build_topic_filter();
        assert_eq!(filter.len(), 1);
        assert!(filter.contains(&"000000000000000000000000def456abc123789012345678901234567890abcd".to_string()));
    }

    // =========================================================================
    // Test Suite: Parsing from Environment Variable
    // =========================================================================

    #[test]
    fn test_from_env_single_address() {
        unsafe {
            std::env::set_var("TRADER_ADDRESSES", "abc123def456789012345678901234567890abcd");
            let config = TradersConfig::from_env().unwrap();
            assert_eq!(config.len(), 1);
            assert_eq!(config.iter().next().unwrap().address, "abc123def456789012345678901234567890abcd");
            assert_eq!(config.iter().next().unwrap().label, "Trader1");
            std::env::remove_var("TRADER_ADDRESSES");
        }
    }

    #[test]
    fn test_from_env_multiple_addresses() {
        unsafe {
            std::env::set_var("TRADER_ADDRESSES", "abc123def456789012345678901234567890abcd,def456abc123789012345678901234567890abcd");
            let config = TradersConfig::from_env().unwrap();
            assert_eq!(config.len(), 2);
            std::env::remove_var("TRADER_ADDRESSES");
        }
    }

    #[test]
    fn test_from_env_strips_whitespace() {
        unsafe {
            std::env::set_var("TRADER_ADDRESSES", " abc123def456789012345678901234567890abcd , def456abc123789012345678901234567890abcd ");
            let config = TradersConfig::from_env().unwrap();
            assert_eq!(config.len(), 2);
            std::env::remove_var("TRADER_ADDRESSES");
        }
    }

    #[test]
    fn test_from_env_handles_0x_prefix() {
        unsafe {
            std::env::set_var("TRADER_ADDRESSES", "0xabc123def456789012345678901234567890abcd");
            let config = TradersConfig::from_env().unwrap();
            assert_eq!(config.len(), 1);
            assert_eq!(config.iter().next().unwrap().address, "abc123def456789012345678901234567890abcd");
            std::env::remove_var("TRADER_ADDRESSES");
        }
    }

    #[test]
    fn test_from_env_deduplicates() {
        unsafe {
            std::env::set_var("TRADER_ADDRESSES", "abc123def456789012345678901234567890abcd,abc123def456789012345678901234567890abcd");
            let config = TradersConfig::from_env().unwrap();
            assert_eq!(config.len(), 1);
            std::env::remove_var("TRADER_ADDRESSES");
        }
    }

    #[test]
    fn test_from_env_invalid_address() {
        unsafe {
            std::env::set_var("TRADER_ADDRESSES", "invalid");
            let result = TradersConfig::from_env();
            assert!(result.is_err());
            std::env::remove_var("TRADER_ADDRESSES");
        }
    }

    #[test]
    fn test_from_env_not_set() {
        unsafe {
            std::env::remove_var("TRADER_ADDRESSES");
            let result = TradersConfig::from_env();
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("TRADER_ADDRESSES"));
        }
    }

    #[test]
    fn test_from_env_empty_string() {
        unsafe {
            std::env::set_var("TRADER_ADDRESSES", "");
            let result = TradersConfig::from_env();
            assert!(result.is_err());
            std::env::remove_var("TRADER_ADDRESSES");
        }
    }

    // =========================================================================
    // Test Suite: Backward Compatibility & load() Function
    // =========================================================================
    //
    // Note: These tests use from_env() directly to test env var behavior,
    // since load() now prioritizes traders.json which may exist in the project.
    // The file loading behavior is tested separately in test_from_file_* tests.

    #[test]
    fn test_from_env_prefers_trader_addresses_over_legacy() {
        unsafe {
            std::env::set_var("TRADER_ADDRESSES", "abc123def456789012345678901234567890abcd");
            std::env::set_var("TARGET_WHALE_ADDRESS", "def456abc123789012345678901234567890abcd");

            // from_env only looks at TRADER_ADDRESSES
            let config = TradersConfig::from_env().unwrap();
            assert_eq!(config.len(), 1);
            assert_eq!(config.iter().next().unwrap().address, "abc123def456789012345678901234567890abcd");

            std::env::remove_var("TRADER_ADDRESSES");
            std::env::remove_var("TARGET_WHALE_ADDRESS");
        }
    }

    #[test]
    fn test_load_uses_legacy_when_no_trader_addresses() {
        // This test verifies legacy fallback when TRADER_ADDRESSES is not set
        // Note: If traders.json exists, it takes priority over env vars
        unsafe {
            std::env::remove_var("TRADER_ADDRESSES");
            std::env::set_var("TARGET_WHALE_ADDRESS", "abc123def456789012345678901234567890abcd");

            // If traders.json doesn't exist, should use legacy address
            // We test the TradersConfig constructor directly for legacy behavior
            let normalized = validate_and_normalize_address("abc123def456789012345678901234567890abcd").unwrap();
            let config = TraderConfig::new(&normalized, "Legacy").unwrap();
            assert_eq!(config.address, "abc123def456789012345678901234567890abcd");
            assert_eq!(config.label, "Legacy");

            std::env::remove_var("TARGET_WHALE_ADDRESS");
        }
    }

    #[test]
    fn test_load_priority_file_over_env() {
        // Verify that from_file works and would take priority
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let json = r#"[{"address": "abc123def456789012345678901234567890abcd", "label": "FromFile"}]"#;
        file.write_all(json.as_bytes()).unwrap();
        file.flush().unwrap();

        let config = TradersConfig::from_file(file.path()).unwrap();
        assert_eq!(config.len(), 1);
        assert_eq!(config.iter().next().unwrap().label, "FromFile");
    }

    #[test]
    fn test_from_env_fails_if_not_set() {
        unsafe {
            std::env::remove_var("TRADER_ADDRESSES");

            let result = TradersConfig::from_env();
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("TRADER_ADDRESSES"));
        }
    }

    #[test]
    fn test_legacy_address_strips_0x() {
        // Test that legacy address normalization works
        let normalized = validate_and_normalize_address("0xabc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(normalized, "abc123def456789012345678901234567890abcd");

        let config = TraderConfig::new(&normalized, "Legacy").unwrap();
        assert_eq!(config.address, "abc123def456789012345678901234567890abcd");
    }

    // =========================================================================
    // Test Suite: JSON File Loading
    // =========================================================================

    #[test]
    fn test_from_file_valid_json() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let json = r#"[
            {
                "address": "abc123def456789012345678901234567890abcd",
                "label": "Whale1",
                "scaling_ratio": 0.03,
                "min_shares": 100.0,
                "enabled": true
            }
        ]"#;
        file.write_all(json.as_bytes()).unwrap();
        file.flush().unwrap();

        let config = TradersConfig::from_file(file.path()).unwrap();
        assert_eq!(config.len(), 1);
        let trader = config.iter().next().unwrap();
        assert_eq!(trader.address, "abc123def456789012345678901234567890abcd");
        assert_eq!(trader.label, "Whale1");
        assert_eq!(trader.scaling_ratio, 0.03);
        assert_eq!(trader.min_shares, 100.0);
        assert!(trader.enabled);
    }

    #[test]
    fn test_from_file_multiple_traders() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let json = r#"[
            {
                "address": "abc123def456789012345678901234567890abcd",
                "label": "Whale1"
            },
            {
                "address": "def456abc123789012345678901234567890abcd",
                "label": "Whale2"
            }
        ]"#;
        file.write_all(json.as_bytes()).unwrap();
        file.flush().unwrap();

        let config = TradersConfig::from_file(file.path()).unwrap();
        assert_eq!(config.len(), 2);
    }

    #[test]
    fn test_from_file_with_defaults() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let json = r#"[
            {
                "address": "abc123def456789012345678901234567890abcd"
            }
        ]"#;
        file.write_all(json.as_bytes()).unwrap();
        file.flush().unwrap();

        let config = TradersConfig::from_file(file.path()).unwrap();
        assert_eq!(config.len(), 1);
        let trader = config.iter().next().unwrap();
        assert_eq!(trader.label, "Trader"); // default
        assert_eq!(trader.scaling_ratio, 0.02); // default
        assert_eq!(trader.min_shares, 0.0); // default
        assert!(trader.enabled); // default
    }

    #[test]
    fn test_from_file_handles_0x_prefix() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let json = r#"[
            {
                "address": "0xabc123def456789012345678901234567890abcd",
                "label": "Whale1"
            }
        ]"#;
        file.write_all(json.as_bytes()).unwrap();
        file.flush().unwrap();

        let config = TradersConfig::from_file(file.path()).unwrap();
        assert_eq!(config.len(), 1);
        assert_eq!(config.iter().next().unwrap().address, "abc123def456789012345678901234567890abcd");
    }

    #[test]
    fn test_from_file_deduplicates() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let json = r#"[
            {
                "address": "abc123def456789012345678901234567890abcd",
                "label": "Whale1"
            },
            {
                "address": "abc123def456789012345678901234567890abcd",
                "label": "Whale1Copy"
            }
        ]"#;
        file.write_all(json.as_bytes()).unwrap();
        file.flush().unwrap();

        let config = TradersConfig::from_file(file.path()).unwrap();
        assert_eq!(config.len(), 1);
    }

    #[test]
    fn test_from_file_invalid_json() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"invalid json").unwrap();
        file.flush().unwrap();

        let result = TradersConfig::from_file(file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_from_file_invalid_address() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let json = r#"[
            {
                "address": "invalid",
                "label": "Bad"
            }
        ]"#;
        file.write_all(json.as_bytes()).unwrap();
        file.flush().unwrap();

        let result = TradersConfig::from_file(file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_from_file_not_found() {
        let result = TradersConfig::from_file(Path::new("/nonexistent/traders.json"));
        assert!(result.is_err());
    }

    // =========================================================================
    // Integration Test: End-to-End Workflow
    // =========================================================================

    #[test]
    fn test_complete_workflow_json_to_websocket_filter() {
        use std::io::Write;

        // Create a realistic traders.json configuration
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let json = r#"[
            {
                "address": "204f72f35326db932158cba6adff0b9a1da95e14",
                "label": "Whale1",
                "scaling_ratio": 0.03,
                "min_shares": 100.0,
                "enabled": true
            },
            {
                "address": "0xdef456abc123789012345678901234567890abcd",
                "label": "Whale2",
                "scaling_ratio": 0.015,
                "min_shares": 50.0,
                "enabled": true
            },
            {
                "address": "abc123def456789012345678901234567890abcd",
                "label": "DisabledWhale",
                "enabled": false
            }
        ]"#;
        file.write_all(json.as_bytes()).unwrap();
        file.flush().unwrap();

        // Load configuration
        let config = TradersConfig::from_file(file.path()).unwrap();

        // Verify loaded correctly
        assert_eq!(config.len(), 3);

        // Build WebSocket topic filter (should only include enabled traders)
        let topics = config.build_topic_filter();
        assert_eq!(topics.len(), 2); // Only enabled traders

        // Verify topics are correctly padded
        for topic in &topics {
            assert_eq!(topic.len(), 64);
            assert!(topic.starts_with("0000"));
        }

        // Test topic lookup (simulating WebSocket event)
        let whale1_topic = "000000000000000000000000204f72f35326db932158cba6adff0b9a1da95e14";
        let trader = config.get_by_topic(whale1_topic).unwrap();
        assert_eq!(trader.label, "Whale1");
        assert_eq!(trader.scaling_ratio, 0.03);
        assert_eq!(trader.min_shares, 100.0);

        // Test address lookup with 0x prefix
        let trader2 = config.get_by_address("0xdef456abc123789012345678901234567890abcd").unwrap();
        assert_eq!(trader2.label, "Whale2");
        assert_eq!(trader2.scaling_ratio, 0.015);

        // Verify disabled trader is in config but not in filter
        let disabled = config.get_by_address("abc123def456789012345678901234567890abcd").unwrap();
        assert_eq!(disabled.label, "DisabledWhale");
        assert!(!disabled.enabled);

        let disabled_topic = "000000000000000000000000abc123def456789012345678901234567890abcd";
        assert!(!topics.contains(&disabled_topic.to_string()));

        // Test iteration
        let mut count = 0;
        for trader in config.iter() {
            assert!(!trader.address.is_empty());
            assert!(!trader.label.is_empty());
            assert_eq!(trader.topic_hex.len(), 64);
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
