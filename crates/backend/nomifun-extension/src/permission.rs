use crate::types::{
    ExtPermissions, FilesystemScope, NetworkPermission, PermissionDetail, PermissionLevel, PermissionSummary, RiskLevel,
};

/// Calculate the overall risk level from permission declarations.
///
/// Rules (from API Spec):
/// - **dangerous**: `shell=true`, `filesystem=full`, or `network=true` (unrestricted)
/// - **moderate**: scoped `network` (domain-restricted) or `filesystem=extension-only|workspace`
/// - **safe**: everything else (only storage, events, clipboard, activeUser, or nothing)
pub fn calculate_risk_level(permissions: &ExtPermissions) -> RiskLevel {
    // Dangerous: shell access
    if permissions.shell == Some(true) {
        return RiskLevel::Dangerous;
    }

    // Dangerous: full filesystem access
    if permissions.filesystem == Some(FilesystemScope::Full) {
        return RiskLevel::Dangerous;
    }

    // Dangerous: unrestricted network
    if let Some(NetworkPermission::Unrestricted(true)) = &permissions.network {
        return RiskLevel::Dangerous;
    }

    // Moderate: scoped network (with allowed domains)
    if matches!(&permissions.network, Some(NetworkPermission::Scoped { .. })) {
        return RiskLevel::Moderate;
    }

    // Moderate: workspace or extension-only filesystem
    if matches!(
        permissions.filesystem,
        Some(FilesystemScope::Workspace) | Some(FilesystemScope::ExtensionOnly)
    ) {
        return RiskLevel::Moderate;
    }

    RiskLevel::Safe
}

/// Build a complete permission summary with risk analysis details.
pub fn build_permission_summary(permissions: &ExtPermissions) -> PermissionSummary {
    let risk_level = calculate_risk_level(permissions);
    let details = build_details(permissions);
    PermissionSummary {
        permissions: permissions.clone(),
        risk_level,
        details,
    }
}

fn build_details(permissions: &ExtPermissions) -> Vec<PermissionDetail> {
    vec![
        build_storage_detail(permissions.storage),
        build_network_detail(&permissions.network),
        build_shell_detail(permissions.shell),
        build_filesystem_detail(permissions.filesystem),
        build_bool_detail("clipboard", permissions.clipboard, "Clipboard read/write access"),
        build_bool_detail(
            "activeUser",
            permissions.active_user,
            "Access to current user information",
        ),
        build_bool_detail("events", permissions.events, "Extension event bus communication"),
    ]
}

fn build_storage_detail(storage: Option<bool>) -> PermissionDetail {
    match storage {
        Some(true) => PermissionDetail {
            permission: "storage".into(),
            level: PermissionLevel::Full,
            description: "Persistent key-value storage access".into(),
        },
        _ => PermissionDetail {
            permission: "storage".into(),
            level: PermissionLevel::None,
            description: "No storage access".into(),
        },
    }
}

fn build_network_detail(network: &Option<NetworkPermission>) -> PermissionDetail {
    match network {
        Some(NetworkPermission::Unrestricted(true)) => PermissionDetail {
            permission: "network".into(),
            level: PermissionLevel::Full,
            description: "Unrestricted network access".into(),
        },
        Some(NetworkPermission::Scoped { allowed_domains, .. }) => PermissionDetail {
            permission: "network".into(),
            level: PermissionLevel::Limited,
            description: format!("Network access limited to: {}", allowed_domains.join(", ")),
        },
        _ => PermissionDetail {
            permission: "network".into(),
            level: PermissionLevel::None,
            description: "No network access".into(),
        },
    }
}

fn build_shell_detail(shell: Option<bool>) -> PermissionDetail {
    match shell {
        Some(true) => PermissionDetail {
            permission: "shell".into(),
            level: PermissionLevel::Full,
            description: "System command execution".into(),
        },
        _ => PermissionDetail {
            permission: "shell".into(),
            level: PermissionLevel::None,
            description: "No shell access".into(),
        },
    }
}

fn build_filesystem_detail(filesystem: Option<FilesystemScope>) -> PermissionDetail {
    match filesystem {
        Some(FilesystemScope::Full) => PermissionDetail {
            permission: "filesystem".into(),
            level: PermissionLevel::Full,
            description: "Full filesystem access".into(),
        },
        Some(FilesystemScope::Workspace) => PermissionDetail {
            permission: "filesystem".into(),
            level: PermissionLevel::Limited,
            description: "Workspace directory access".into(),
        },
        Some(FilesystemScope::ExtensionOnly) => PermissionDetail {
            permission: "filesystem".into(),
            level: PermissionLevel::Limited,
            description: "Extension directory access only".into(),
        },
        None => PermissionDetail {
            permission: "filesystem".into(),
            level: PermissionLevel::None,
            description: "No filesystem access".into(),
        },
    }
}

fn build_bool_detail(name: &str, value: Option<bool>, granted_desc: &str) -> PermissionDetail {
    match value {
        Some(true) => PermissionDetail {
            permission: name.into(),
            level: PermissionLevel::Full,
            description: granted_desc.into(),
        },
        _ => PermissionDetail {
            permission: name.into(),
            level: PermissionLevel::None,
            description: format!("No {name} access"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- calculate_risk_level --

    #[test]
    fn test_no_permissions_is_safe() {
        let perms = ExtPermissions::default();
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Safe);
    }

    #[test]
    fn test_storage_and_events_only_is_safe() {
        let perms = ExtPermissions {
            storage: Some(true),
            events: Some(true),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Safe);
    }

    #[test]
    fn test_clipboard_is_safe() {
        let perms = ExtPermissions {
            clipboard: Some(true),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Safe);
    }

    #[test]
    fn test_active_user_is_safe() {
        let perms = ExtPermissions {
            active_user: Some(true),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Safe);
    }

    #[test]
    fn test_scoped_network_is_moderate() {
        let perms = ExtPermissions {
            network: Some(NetworkPermission::Scoped {
                allowed_domains: vec!["api.example.com".into()],
                reasoning: "API calls".into(),
            }),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Moderate);
    }

    #[test]
    fn test_workspace_filesystem_is_moderate() {
        let perms = ExtPermissions {
            filesystem: Some(FilesystemScope::Workspace),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Moderate);
    }

    #[test]
    fn test_extension_only_filesystem_is_moderate() {
        let perms = ExtPermissions {
            filesystem: Some(FilesystemScope::ExtensionOnly),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Moderate);
    }

    #[test]
    fn test_shell_is_dangerous() {
        let perms = ExtPermissions {
            shell: Some(true),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Dangerous);
    }

    #[test]
    fn test_full_filesystem_is_dangerous() {
        let perms = ExtPermissions {
            filesystem: Some(FilesystemScope::Full),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Dangerous);
    }

    #[test]
    fn test_unrestricted_network_is_dangerous() {
        let perms = ExtPermissions {
            network: Some(NetworkPermission::Unrestricted(true)),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Dangerous);
    }

    #[test]
    fn test_dangerous_overrides_moderate() {
        let perms = ExtPermissions {
            shell: Some(true),
            network: Some(NetworkPermission::Scoped {
                allowed_domains: vec!["example.com".into()],
                reasoning: "test".into(),
            }),
            ..Default::default()
        };
        assert_eq!(calculate_risk_level(&perms), RiskLevel::Dangerous);
    }

    // -- build_permission_summary --

    #[test]
    fn test_summary_includes_all_permissions() {
        let perms = ExtPermissions {
            storage: Some(true),
            events: Some(true),
            ..Default::default()
        };
        let summary = build_permission_summary(&perms);
        assert_eq!(summary.risk_level, RiskLevel::Safe);
        assert_eq!(summary.permissions, perms);
        assert_eq!(summary.details.len(), 7);
    }

    #[test]
    fn test_summary_storage_detail() {
        let perms = ExtPermissions {
            storage: Some(true),
            ..Default::default()
        };
        let summary = build_permission_summary(&perms);
        let storage = summary.details.iter().find(|d| d.permission == "storage").unwrap();
        assert_eq!(storage.level, PermissionLevel::Full);
    }

    #[test]
    fn test_summary_network_scoped_detail() {
        let perms = ExtPermissions {
            network: Some(NetworkPermission::Scoped {
                allowed_domains: vec!["a.com".into(), "b.com".into()],
                reasoning: "test".into(),
            }),
            ..Default::default()
        };
        let summary = build_permission_summary(&perms);
        let network = summary.details.iter().find(|d| d.permission == "network").unwrap();
        assert_eq!(network.level, PermissionLevel::Limited);
        assert!(network.description.contains("a.com"));
        assert!(network.description.contains("b.com"));
    }

    #[test]
    fn test_summary_filesystem_full_detail() {
        let perms = ExtPermissions {
            filesystem: Some(FilesystemScope::Full),
            ..Default::default()
        };
        let summary = build_permission_summary(&perms);
        let fs = summary.details.iter().find(|d| d.permission == "filesystem").unwrap();
        assert_eq!(fs.level, PermissionLevel::Full);
    }

    #[test]
    fn test_summary_no_permissions_all_none() {
        let perms = ExtPermissions::default();
        let summary = build_permission_summary(&perms);
        for detail in &summary.details {
            assert_eq!(
                detail.level,
                PermissionLevel::None,
                "{} should be None",
                detail.permission
            );
        }
    }
}
