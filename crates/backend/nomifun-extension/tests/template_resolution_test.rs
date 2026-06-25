//! Integration tests for template resolution (test-plan MV-5 through MV-9).
//!
//! These test the public API surface of env var and @file: template resolution.

use std::collections::HashMap;

use nomifun_extension::{resolve_env_map, resolve_env_templates, resolve_file_reference};

// -- MV-5: env var template resolution --

#[test]
fn mv5_env_var_resolved() {
    unsafe { std::env::set_var("_IT_MY_API_KEY", "test123") };
    let result = resolve_env_templates("${_IT_MY_API_KEY}", false).unwrap();
    assert_eq!(result, "test123");
    unsafe { std::env::remove_var("_IT_MY_API_KEY") };
}

#[test]
fn mv5_env_var_in_map() {
    unsafe { std::env::set_var("_IT_MAP_KEY", "secret_value") };
    let mut env = HashMap::new();
    env.insert("API_KEY".into(), "${_IT_MAP_KEY}".into());

    let resolved = resolve_env_map(&env, false).unwrap();
    assert_eq!(resolved["API_KEY"], "secret_value");

    unsafe { std::env::remove_var("_IT_MAP_KEY") };
}

// -- MV-6: strict mode undefined variable error --

#[test]
fn mv6_strict_mode_undefined_var_error() {
    let err = resolve_env_templates("${_IT_UNDEFINED_VAR}", true).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("_IT_UNDEFINED_VAR"));
}

// -- MV-7: lenient mode undefined variable -> empty string --

#[test]
fn mv7_lenient_mode_undefined_var_empty() {
    let result = resolve_env_templates("prefix_${_IT_UNDEFINED_VAR}_suffix", false).unwrap();
    assert_eq!(result, "prefix__suffix");
}

// -- MV-8: @file: reference resolved --

#[test]
fn mv8_file_reference_resolved() {
    let dir = std::env::temp_dir().join("ext_it_file_ref");
    std::fs::create_dir_all(&dir).unwrap();
    let prompt_dir = dir.join("prompts");
    std::fs::create_dir_all(&prompt_dir).unwrap();
    std::fs::write(prompt_dir.join("system.md"), "You are a helpful assistant.").unwrap();

    let result = resolve_file_reference("@file:prompts/system.md", &dir).unwrap();
    assert_eq!(result, "You are a helpful assistant.");

    std::fs::remove_dir_all(&dir).unwrap();
}

// -- MV-9: @file: reference file not found --

#[test]
fn mv9_file_reference_not_found() {
    let dir = std::env::temp_dir().join("ext_it_file_ref_missing");
    std::fs::create_dir_all(&dir).unwrap();

    let err = resolve_file_reference("@file:nonexistent.md", &dir).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("nonexistent.md"));

    std::fs::remove_dir_all(&dir).unwrap();
}

// -- Edge cases --

#[test]
fn non_file_reference_passes_through() {
    let result = resolve_file_reference("Just a normal system prompt", std::path::Path::new("/tmp")).unwrap();
    assert_eq!(result, "Just a normal system prompt");
}

#[test]
fn multiple_env_vars_in_one_string() {
    unsafe { std::env::set_var("_IT_HOST", "localhost") };
    unsafe { std::env::set_var("_IT_PORT", "8080") };
    let result = resolve_env_templates("http://${_IT_HOST}:${_IT_PORT}/api", false).unwrap();
    assert_eq!(result, "http://localhost:8080/api");
    unsafe { std::env::remove_var("_IT_HOST") };
    unsafe { std::env::remove_var("_IT_PORT") };
}

#[test]
fn env_map_strict_mode_error() {
    let mut env = HashMap::new();
    env.insert("KEY".into(), "${_IT_STRICT_MISSING}".into());
    assert!(resolve_env_map(&env, true).is_err());
}

#[test]
fn dollar_without_brace_is_literal() {
    let result = resolve_env_templates("price is $50 each", false).unwrap();
    assert_eq!(result, "price is $50 each");
}

// -- Path traversal protection --

#[test]
fn path_traversal_with_dotdot_blocked() {
    let dir = std::env::temp_dir().join("ext_it_traversal");
    std::fs::create_dir_all(&dir).unwrap();

    let outside = std::env::temp_dir().join("ext_it_traversal_outside.txt");
    std::fs::write(&outside, "should not be readable").unwrap();

    let err = resolve_file_reference("@file:../ext_it_traversal_outside.txt", &dir).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("traversal") || msg.contains("Path traversal"));

    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_file(&outside).unwrap();
}

#[test]
fn path_traversal_nested_dotdot_blocked() {
    let dir = std::env::temp_dir().join("ext_it_traversal_nested");
    let sub = dir.join("deep");
    std::fs::create_dir_all(&sub).unwrap();

    let outside = std::env::temp_dir().join("ext_it_traversal_nested_secret.txt");
    std::fs::write(&outside, "nested secret").unwrap();

    let err = resolve_file_reference("@file:deep/../../ext_it_traversal_nested_secret.txt", &dir).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("traversal") || msg.contains("Path traversal"));

    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_file(&outside).unwrap();
}

#[test]
fn valid_subdirectory_reference_allowed() {
    let dir = std::env::temp_dir().join("ext_it_valid_subdir");
    let sub = dir.join("data");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("config.json"), r#"{"key": "value"}"#).unwrap();

    let result = resolve_file_reference("@file:data/config.json", &dir).unwrap();
    assert_eq!(result, r#"{"key": "value"}"#);

    std::fs::remove_dir_all(&dir).unwrap();
}
