//! Black-box validation tests (test-plan T15).
//!
//! Tests password and username validation rules as specified in API Spec 03-auth.md.

use nomifun_auth::{validate_password, validate_username};

// --- T15.1: Username legal ---

#[test]
fn t15_1_valid_username_with_underscore_and_hyphen() {
    assert!(validate_username("test_user-1").is_ok());
}

#[test]
fn t15_1_valid_username_alphanumeric() {
    assert!(validate_username("admin123").is_ok());
}

#[test]
fn t15_1_valid_username_mixed_case() {
    assert!(validate_username("TestUser").is_ok());
}

// --- T15.2: Username too short ---

#[test]
fn t15_2_username_two_chars() {
    assert!(validate_username("ab").is_err());
}

#[test]
fn t15_2_username_one_char() {
    assert!(validate_username("a").is_err());
}

#[test]
fn t15_2_username_empty() {
    assert!(validate_username("").is_err());
}

// --- T15.3: Username too long ---

#[test]
fn t15_3_username_33_chars() {
    let name = "a".repeat(33);
    assert!(validate_username(&name).is_err());
}

#[test]
fn t15_3_username_100_chars() {
    let name = "a".repeat(100);
    assert!(validate_username(&name).is_err());
}

// --- T15.4: Username illegal characters ---

#[test]
fn t15_4_username_with_at() {
    assert!(validate_username("test@user").is_err());
}

#[test]
fn t15_4_username_with_space() {
    assert!(validate_username("test user").is_err());
}

#[test]
fn t15_4_username_with_dot() {
    assert!(validate_username("test.user").is_err());
}

#[test]
fn t15_4_username_with_slash() {
    assert!(validate_username("test/user").is_err());
}

// --- T15.5: Username starts/ends with special chars ---

#[test]
fn t15_5_starts_with_underscore() {
    assert!(validate_username("_test").is_err());
}

#[test]
fn t15_5_starts_with_hyphen() {
    assert!(validate_username("-test").is_err());
}

#[test]
fn t15_5_ends_with_underscore() {
    assert!(validate_username("test_").is_err());
}

#[test]
fn t15_5_ends_with_hyphen() {
    assert!(validate_username("test-").is_err());
}

// --- Password validation (supplement) ---

#[test]
fn valid_password_accepted() {
    assert!(validate_password("StrongP@ss123").is_ok());
}

#[test]
fn password_too_short_rejected() {
    assert!(validate_password("short").is_err());
}

#[test]
fn password_too_long_rejected() {
    let long = "a".repeat(129);
    assert!(validate_password(&long).is_err());
}

#[test]
fn weak_password_rejected() {
    assert!(validate_password("password").is_err());
    assert!(validate_password("12345678").is_err());
    assert!(validate_password("qwertyui").is_err());
}
