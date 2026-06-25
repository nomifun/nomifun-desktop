//! Black-box integration tests for PlanConfig (TC-3.2-01 through TC-3.2-03).
//!
//! These test the public API of PlanConfig from a config-file consumer's
//! perspective: default values, full TOML override, and partial override.

use nomi_config::config::ConfigFile;
use nomi_config::plan::PlanConfig;

/// TC-3.2-01: PlanConfig default values.
/// Input: no `[plan]` section in config.
/// Expected: enabled = true, plan_directory = ".nomi/plans".
#[test]
fn tc_3_2_01_plan_config_defaults() {
    let cfg = PlanConfig::default();
    assert!(cfg.enabled);
    assert_eq!(cfg.plan_directory, ".nomi/plans");
}

/// TC-3.2-01 (variant): absent [plan] section in ConfigFile yields defaults.
#[test]
fn tc_3_2_01_absent_plan_section_uses_defaults() {
    let config: ConfigFile = toml::from_str("").unwrap();
    assert!(config.plan.enabled);
    assert_eq!(config.plan.plan_directory, ".nomi/plans");
}

/// TC-3.2-02: PlanConfig TOML deserialization with all fields.
/// Input: [plan] section with enabled = false and custom plan_directory.
/// Expected: correct parsing.
#[test]
fn tc_3_2_02_plan_config_toml_full() {
    let toml_str = r#"
[plan]
enabled = false
plan_directory = "/custom/plans"
"#;
    let config: ConfigFile = toml::from_str(toml_str).unwrap();
    assert!(!config.plan.enabled);
    assert_eq!(config.plan.plan_directory, "/custom/plans");
}

/// TC-3.2-03: PlanConfig partial field override.
/// Input: [plan] section with only enabled = false (no plan_directory).
/// Expected: enabled = false, plan_directory uses default.
#[test]
fn tc_3_2_03_plan_config_partial_override() {
    let toml_str = r#"
[plan]
enabled = false
"#;
    let config: ConfigFile = toml::from_str(toml_str).unwrap();
    assert!(!config.plan.enabled);
    assert_eq!(config.plan.plan_directory, ".nomi/plans");
}

/// ConfigFile with [plan] section alongside other sections parses completely.
#[test]
fn plan_config_coexists_with_other_sections() {
    let toml_str = r#"
[default]
provider = "anthropic"

[compact]
context_window = 100000

[plan]
enabled = true
plan_directory = ".nomi/custom-plans"
"#;
    let config: ConfigFile = toml::from_str(toml_str).unwrap();
    assert!(config.plan.enabled);
    assert_eq!(config.plan.plan_directory, ".nomi/custom-plans");
    assert_eq!(config.default.provider, "anthropic");
    assert_eq!(config.compact.context_window, 100_000);
}
