//! Black-box tests for API response formats (test-plan T3.1, T3.2, T3.3).

use nomifun_api_types::{ApiResponse, ErrorResponse};
use nomifun_common::AppError;

// --- T3.1: Success response format ---

#[test]
fn t3_1_success_response_with_data() {
    let resp = ApiResponse::ok("result");
    let json = serde_json::to_value(&resp).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"], "result");
}

#[test]
fn t3_1_success_response_with_message() {
    let resp = ApiResponse::message("Operation completed");
    let json = serde_json::to_value(&resp).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["message"], "Operation completed");
    // data should be absent (not null)
    assert!(json.get("data").is_none());
}

#[test]
fn t3_1_success_response_with_data_and_message() {
    let resp = ApiResponse::with_message(vec![1, 2, 3], "Found 3 items");
    let json = serde_json::to_value(&resp).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"], serde_json::json!([1, 2, 3]));
    assert_eq!(json["message"], "Found 3 items");
}

#[test]
fn t3_1_success_response_struct_data() {
    #[derive(serde::Serialize)]
    struct UserData {
        id: String,
        name: String,
    }
    let resp = ApiResponse::ok(UserData {
        id: "u1".into(),
        name: "Alice".into(),
    });
    let json = serde_json::to_value(&resp).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["id"], "u1");
    assert_eq!(json["data"]["name"], "Alice");
}

// --- T3.2: Error response format ---

#[test]
fn t3_2_error_response_format() {
    let resp = ErrorResponse::new("Resource not found", "NOT_FOUND");
    let json = serde_json::to_value(&resp).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"], "Resource not found");
    assert_eq!(json["code"], "NOT_FOUND");
}

#[test]
fn t3_2_error_response_has_all_fields() {
    let resp = ErrorResponse::new("err", "CODE");
    let json = serde_json::to_value(&resp).unwrap();

    // Verify all three required fields exist
    assert!(json.get("success").is_some());
    assert!(json.get("error").is_some());
    assert!(json.get("code").is_some());
    assert!(json.get("details").is_none());
}

// --- T3.3: AppError auto-conversion ---

#[test]
fn t3_3_app_error_not_found_to_error_response() {
    let err = AppError::NotFound("user 42".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.error, "Not found: user 42");
    assert_eq!(resp.code, "NOT_FOUND");
}

#[test]
fn t3_3_app_error_bad_request_to_error_response() {
    let err = AppError::BadRequest("missing field: username".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.error, "Bad request: missing field: username");
    assert_eq!(resp.code, "BAD_REQUEST");
}

#[test]
fn t3_3_app_error_unauthorized_to_error_response() {
    let err = AppError::Unauthorized("invalid token".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.code, "UNAUTHORIZED");
}

#[test]
fn t3_3_app_error_forbidden_to_error_response() {
    let err = AppError::Forbidden("access denied".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.code, "FORBIDDEN");
}

#[test]
fn t3_3_app_error_conflict_to_error_response() {
    let err = AppError::Conflict("username taken".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.code, "CONFLICT");
}

#[test]
fn t3_3_app_error_rate_limited_to_error_response() {
    let resp = ErrorResponse::from(AppError::RateLimited);

    assert!(!resp.success);
    assert_eq!(resp.error, "Rate limited");
    assert_eq!(resp.code, "RATE_LIMITED");
}

#[test]
fn t3_3_app_error_internal_to_error_response() {
    let err = AppError::Internal("db connection lost".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.code, "INTERNAL_ERROR");
}

#[test]
fn t3_3_app_error_bad_gateway_to_error_response() {
    let err = AppError::BadGateway("upstream timeout".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.code, "BAD_GATEWAY");
}

#[test]
fn t3_3_app_error_timeout_to_error_response() {
    let err = AppError::Timeout("request timed out".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.code, "TIMEOUT");
}

#[test]
fn t3_3_workspace_error_exposes_structured_details() {
    let err = AppError::WorkspacePathEdgeWhitespace("/tmp/Archive ".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.code, "WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED");
    assert_eq!(
        resp.details.as_ref().and_then(|details| details.get("workspace_path")),
        Some(&serde_json::json!("/tmp/Archive "))
    );
    assert_eq!(
        resp.details
            .as_ref()
            .and_then(|details| details.get("offending_segments")),
        Some(&serde_json::json!(["Archive "]))
    );
    assert_eq!(
        resp.details.as_ref().and_then(|details| details.get("operation")),
        Some(&serde_json::json!("create"))
    );
}

#[test]
fn t3_3_runtime_workspace_error_exposes_structured_details() {
    let err = AppError::WorkspacePathEdgeWhitespaceRuntimeUnsupported("/tmp/Archive ".into());
    let resp = ErrorResponse::from(err);

    assert!(!resp.success);
    assert_eq!(resp.code, "WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED");
    assert_eq!(
        resp.details.as_ref().and_then(|details| details.get("workspace_path")),
        Some(&serde_json::json!("/tmp/Archive "))
    );
    assert_eq!(
        resp.details
            .as_ref()
            .and_then(|details| details.get("offending_segments")),
        Some(&serde_json::json!(["Archive "]))
    );
    assert_eq!(
        resp.details.as_ref().and_then(|details| details.get("operation")),
        Some(&serde_json::json!("runtime"))
    );
}
