//! Persisted installation defaults; the Kernel, not this repository, validates
//! Provider compatibility. CAS prevents a stale settings page overwriting a choice.
use std::collections::BTreeMap;

use nomifun_agent_contracts::{ExecutionRoleId, InstallationRoleBinding, RoleProviderSelection};
use sqlx::SqlitePool;

use crate::DbError;

pub async fn load_installation_role_bindings(
    pool: &SqlitePool,
) -> Result<BTreeMap<ExecutionRoleId, InstallationRoleBinding>, DbError> {
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT role_id, role_contract_ref_json, provider_mount_id, binding_version, updated_at
         FROM installation_role_bindings ORDER BY role_id",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, json, mount, version, updated_at_ms)| {
            let role: nomifun_agent_contracts::ExactRoleContractRef =
                serde_json::from_str(&json)
                    .map_err(|error| DbError::Init(format!("invalid Role default: {error}")))?;
            let binding_version = u64::try_from(version)
                .ok()
                .filter(|v| *v > 0)
                .ok_or_else(|| DbError::Init("invalid Role binding version".into()))?;
            if role.key.role_id.as_ref() != id {
                return Err(DbError::Init("Role default identity mismatch".into()));
            }
            Ok((
                id.into(),
                InstallationRoleBinding {
                    selection: RoleProviderSelection {
                        role,
                        provider_mount_id: mount.into(),
                    },
                    binding_version,
                    updated_at_ms,
                },
            ))
        })
        .collect()
}

pub async fn put_installation_role_binding(
    pool: &SqlitePool,
    selection: RoleProviderSelection,
    expected_version: u64,
    updated_at_ms: i64,
) -> Result<InstallationRoleBinding, DbError> {
    let version = i64::try_from(expected_version)
        .ok()
        .filter(|v| *v < i64::MAX)
        .ok_or_else(|| DbError::Conflict("Role binding version exhausted or invalid".into()))?;
    let json =
        serde_json::to_string(&selection.role).map_err(|error| DbError::Init(error.to_string()))?;
    let result = if version == 0 {
        sqlx::query(
            "INSERT INTO installation_role_bindings
            (role_id, role_contract_ref_json, provider_mount_id, binding_version, updated_at)
            VALUES (?, ?, ?, 1, ?) ON CONFLICT(role_id) DO NOTHING",
        )
        .bind(selection.role.key.role_id.as_ref())
        .bind(&json)
        .bind(selection.provider_mount_id.as_ref())
        .bind(updated_at_ms)
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            "UPDATE installation_role_bindings SET role_contract_ref_json = ?,
            provider_mount_id = ?, binding_version = binding_version + 1, updated_at = ?
            WHERE role_id = ? AND binding_version = ?",
        )
        .bind(&json)
        .bind(selection.provider_mount_id.as_ref())
        .bind(updated_at_ms)
        .bind(selection.role.key.role_id.as_ref())
        .bind(version)
        .execute(pool)
        .await?
    };
    if result.rows_affected() != 1 {
        return Err(DbError::Conflict(
            "Role default changed; reload before saving".into(),
        ));
    }
    Ok(InstallationRoleBinding {
        selection,
        binding_version: expected_version + 1,
        updated_at_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(mount: &str) -> RoleProviderSelection {
        RoleProviderSelection {
            role: nomifun_agent_contracts::ExactRoleContractRef {
                key: nomifun_agent_contracts::RoleContractKey {
                    role_id: "test.context".into(),
                    contract_version: "1.0.0".into(),
                },
                contract_digest: "a".repeat(64).into(),
            },
            provider_mount_id: mount.into(),
        }
    }

    #[tokio::test]
    async fn defaults_are_durable_exact_and_compare_and_swap() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("defaults.db");
        let db = crate::init_database(&path).await.unwrap();
        let a = put_installation_role_binding(db.pool(), selection("builtin-context"), 0, 10)
            .await
            .unwrap();
        assert_eq!(a.binding_version, 1);
        assert!(
            put_installation_role_binding(db.pool(), selection("other"), 0, 20)
                .await
                .is_err()
        );
        let b = put_installation_role_binding(db.pool(), selection("user-context"), 1, 30)
            .await
            .unwrap();
        assert_eq!(b.binding_version, 2);
        assert!(
            put_installation_role_binding(db.pool(), selection("stale"), 1, 40)
                .await
                .is_err()
        );
        crate::validate_id_schema_contract(db.pool()).await.unwrap();
        crate::validate_id_data_contract(db.pool()).await.unwrap();
        db.pool().close().await;
        let reopened = crate::init_database(&path).await.unwrap();
        assert_eq!(
            load_installation_role_bindings(reopened.pool())
                .await
                .unwrap()[&"test.context".into()],
            b
        );
    }
}
