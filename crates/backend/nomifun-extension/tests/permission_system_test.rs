//! Integration tests for permission system (test-plan PS-1 through PS-7).
//!
//! These test the public API surface of risk level calculation and permission summary.

use nomifun_extension::{
    ExtPermissions, FilesystemScope, NetworkPermission, PermissionLevel, RiskLevel, build_permission_summary,
    calculate_risk_level,
};

// -- PS-1: storage + events only → safe --

#[test]
fn ps1_storage_and_events_is_safe() {
    let perms = ExtPermissions {
        storage: Some(true),
        events: Some(true),
        ..Default::default()
    };
    assert_eq!(calculate_risk_level(&perms), RiskLevel::Safe);
}

// -- PS-2: scoped network → moderate --

#[test]
fn ps2_scoped_network_is_moderate() {
    let perms = ExtPermissions {
        network: Some(NetworkPermission::Scoped {
            allowed_domains: vec!["api.example.com".into()],
            reasoning: "API calls".into(),
        }),
        ..Default::default()
    };
    assert_eq!(calculate_risk_level(&perms), RiskLevel::Moderate);
}

// -- PS-3: shell → dangerous --

#[test]
fn ps3_shell_is_dangerous() {
    let perms = ExtPermissions {
        shell: Some(true),
        ..Default::default()
    };
    assert_eq!(calculate_risk_level(&perms), RiskLevel::Dangerous);
}

// -- PS-4: full filesystem → dangerous --

#[test]
fn ps4_full_filesystem_is_dangerous() {
    let perms = ExtPermissions {
        filesystem: Some(FilesystemScope::Full),
        ..Default::default()
    };
    assert_eq!(calculate_risk_level(&perms), RiskLevel::Dangerous);
}

// -- PS-5: workspace filesystem → moderate --

#[test]
fn ps5_workspace_filesystem_is_moderate() {
    let perms = ExtPermissions {
        filesystem: Some(FilesystemScope::Workspace),
        ..Default::default()
    };
    assert_eq!(calculate_risk_level(&perms), RiskLevel::Moderate);
}

// -- PS-6: unrestricted network → dangerous --

#[test]
fn ps6_unrestricted_network_is_dangerous() {
    let perms = ExtPermissions {
        network: Some(NetworkPermission::Unrestricted(true)),
        ..Default::default()
    };
    assert_eq!(calculate_risk_level(&perms), RiskLevel::Dangerous);
}

// -- PS-7: no permissions → safe --

#[test]
fn ps7_no_permissions_is_safe() {
    let perms = ExtPermissions::default();
    assert_eq!(calculate_risk_level(&perms), RiskLevel::Safe);
}

// -- Summary integration --

#[test]
fn summary_contains_correct_risk_and_details() {
    let perms = ExtPermissions {
        storage: Some(true),
        network: Some(NetworkPermission::Scoped {
            allowed_domains: vec!["api.example.com".into()],
            reasoning: "needed".into(),
        }),
        ..Default::default()
    };
    let summary = build_permission_summary(&perms);
    assert_eq!(summary.risk_level, RiskLevel::Moderate);
    assert_eq!(summary.permissions, perms);

    // Should have 7 detail entries (one per permission type)
    assert_eq!(summary.details.len(), 7);

    // Storage should be Full
    let storage = summary.details.iter().find(|d| d.permission == "storage").unwrap();
    assert_eq!(storage.level, PermissionLevel::Full);

    // Network should be Limited
    let network = summary.details.iter().find(|d| d.permission == "network").unwrap();
    assert_eq!(network.level, PermissionLevel::Limited);

    // Shell should be None
    let shell = summary.details.iter().find(|d| d.permission == "shell").unwrap();
    assert_eq!(shell.level, PermissionLevel::None);
}

#[test]
fn summary_dangerous_permissions_detail() {
    let perms = ExtPermissions {
        shell: Some(true),
        filesystem: Some(FilesystemScope::Full),
        network: Some(NetworkPermission::Unrestricted(true)),
        ..Default::default()
    };
    let summary = build_permission_summary(&perms);
    assert_eq!(summary.risk_level, RiskLevel::Dangerous);

    let shell = summary.details.iter().find(|d| d.permission == "shell").unwrap();
    assert_eq!(shell.level, PermissionLevel::Full);

    let fs = summary.details.iter().find(|d| d.permission == "filesystem").unwrap();
    assert_eq!(fs.level, PermissionLevel::Full);

    let net = summary.details.iter().find(|d| d.permission == "network").unwrap();
    assert_eq!(net.level, PermissionLevel::Full);
}
