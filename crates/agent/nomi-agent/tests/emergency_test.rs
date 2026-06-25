//! Black-box integration tests for emergency truncation (TC-2.5-01 .. TC-2.5-04).
//!
//! These tests treat `is_at_emergency_limit` as a public API and verify
//! functional requirements from test-plan.md without relying on internal details.

use nomi_agent::compact::emergency::{EMERGENCY_USER_MESSAGE, is_at_emergency_limit};
use nomi_config::compact::CompactConfig;

// ── TC-2.5-01: Below emergency threshold ───────────────────────────────────

#[test]
fn tc_2_5_01_below_emergency_threshold() {
    // context_window=200_000, emergency_buffer=3_000
    // emergency_limit = 200k - 3k = 197k
    // 190k < 197k → false
    let config = CompactConfig::default();
    assert!(
        !is_at_emergency_limit(190_000, &config),
        "190k tokens should be below the 197k emergency limit"
    );
}

// ── TC-2.5-02: Above emergency threshold ───────────────────────────────────

#[test]
fn tc_2_5_02_above_emergency_threshold() {
    // 198k >= 197k → true
    let config = CompactConfig::default();
    assert!(
        is_at_emergency_limit(198_000, &config),
        "198k tokens should exceed the 197k emergency limit"
    );
}

// ── TC-2.5-03: Exactly at emergency threshold ──────────────────────────────

#[test]
fn tc_2_5_03_at_exact_emergency_threshold() {
    // 197k >= 197k → true
    let config = CompactConfig::default();
    assert!(
        is_at_emergency_limit(197_000, &config),
        "197k tokens should trigger at exactly the emergency limit"
    );
}

// ── TC-2.5-04: Small context window ────────────────────────────────────────

#[test]
fn tc_2_5_04_small_context_window() {
    // context_window=8_000, emergency_buffer=3_000
    // emergency_limit = 8k - 3k = 5k
    // 6k >= 5k → true
    let config = CompactConfig {
        context_window: 8_000,
        emergency_buffer: 3_000,
        ..CompactConfig::default()
    };
    assert!(
        is_at_emergency_limit(6_000, &config),
        "6k tokens should exceed 5k emergency limit on an 8k context window"
    );
}

// ── Additional integration-level checks ────────────────────────────────────

#[test]
fn emergency_check_ignores_enabled_flag() {
    // Emergency is the safety net — it fires even when compact is disabled
    let config = CompactConfig {
        enabled: false,
        ..CompactConfig::default()
    };
    assert!(
        is_at_emergency_limit(198_000, &config),
        "emergency check must fire regardless of the enabled flag"
    );
}

#[test]
fn user_message_is_actionable() {
    // The message should tell the user what to do
    assert!(
        EMERGENCY_USER_MESSAGE.contains("/compact"),
        "emergency message should mention /compact"
    );
    assert!(
        EMERGENCY_USER_MESSAGE.contains("new conversation"),
        "emergency message should mention starting a new conversation"
    );
}

#[test]
fn autocompact_fires_before_emergency() {
    // Verify that the autocompact threshold is lower than the emergency limit
    // so autocompact gets a chance to run before the safety net kicks in.
    use nomi_agent::compact::auto::should_autocompact;

    let config = CompactConfig::default();

    // Pick a token count that triggers autocompact but not emergency
    let token_count: u64 = 170_000;
    let autocompact_triggers = should_autocompact(token_count, &config);
    let emergency_triggers = is_at_emergency_limit(token_count, &config);

    assert!(
        autocompact_triggers && !emergency_triggers,
        "at 170k tokens, autocompact should trigger (threshold 167k) \
         but emergency should not (limit 197k)"
    );
}

#[test]
fn both_trigger_near_limit() {
    // When very close to the limit, both autocompact and emergency should fire
    use nomi_agent::compact::auto::should_autocompact;

    let config = CompactConfig::default();
    let token_count: u64 = 198_000;

    assert!(should_autocompact(token_count, &config));
    assert!(is_at_emergency_limit(token_count, &config));
}
