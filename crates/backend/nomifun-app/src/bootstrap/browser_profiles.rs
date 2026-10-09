//! Website data is owned by WebKit, outside the files quarantined by reset.
//! The existing immutable reset plan and retired canonical Session identities
//! are sufficient cleanup authority; no second Session/Profile ledger is kept.

use std::path::Path;

use anyhow::{Context, Result, bail};
use nomifun_agent_contracts::PrincipalRef;
#[cfg(feature = "browser-use")]
use nomifun_browser_platform::workspace::BrowserResourceService;
use sqlx::{Row, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};

pub(super) async fn cleanup_retired_browser_profiles(
    config: &crate::AppConfig,
    #[cfg(feature = "browser-use")]
    browser: Option<&BrowserResourceService>,
) -> Result<()> {
    let Some(plan) = nomifun_common::factory_reset::read_pending_v3_reset(
        &config.data_dir, &config.work_dir,
    )? else { return Ok(()); };
    // read_pending_v3_reset validates the plan's canonical root and exact
    // retired path. Still reject links before opening the retired evidence.
    let retired = config.data_dir.join(&plan.retired_dir);
    require_plain_directory(&config.data_dir.join("retired-datasets"))?;
    require_plain_directory(&retired)?;
    let database_path = retired.join("nomifun-backend.db");
    if !regular_file_exists(&database_path)? { return Ok(()); }
    let identities = read_session_identities(&database_path).await?;
    if identities.is_empty() { return Ok(()); }
    let generation_path = retired.join("storage-generation");
    if !regular_file_exists(&generation_path)? {
        // No canonical Session identities means no current managed profile.
        // If identities exist, a missing generation is not proof that their
        // OS-managed website data is absent. Keep the reset pending for repair.
        bail!("retired canonical Sessions have no storage-generation evidence; website-data cleanup cannot be proven and the dataset reset remains pending");
    }
    if std::fs::symlink_metadata(&generation_path)?.len() != 36 {
        bail!("invalid retired Browser storage generation length");
    }
    let generation = std::fs::read_to_string(&generation_path)
        .context("read retired Browser storage generation")?;
    remove_profile_identities(identities, &generation, #[cfg(feature = "browser-use")] browser).await
}

async fn remove_profile_identities(
    identities: Vec<(String, String)>,
    generation: &str,
    #[cfg(feature = "browser-use")]
    browser: Option<&BrowserResourceService>,
) -> Result<()> {
    nomifun_common::validate_uuidv7(generation)
        .context("invalid retired Browser storage generation")?;
    if identities.is_empty() { return Ok(()); }
    #[cfg(not(feature = "browser-use"))]
    bail!("the pending macOS dataset reset requires native website-data cleanup unavailable in this build; start NomiFun Desktop with this data root to finish the reset");
    #[cfg(feature = "browser-use")]
    {
    let browser = browser.context(
        "the pending macOS dataset reset requires native website-data cleanup; start NomiFun Desktop with this data root to finish the reset",
    )?;
    // Startup owns the data-root lock and has not admitted a runtime. Native
    // deletion is idempotent, so a crash resumes from this same reset plan.
    // Only after all callbacks succeed may normal bootstrap finalize it.
    for (owner, session) in identities {
        browser.delete_retired_webkit_profile(&owner, &session, &generation)
            .await.context("remove retired Session's WK website data")?;
    }
    Ok(())
    }
}

fn require_plain_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .context("inspect retired Browser dataset directory")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("retired Browser dataset directory is not a plain directory");
    }
    Ok(())
}

fn regular_file_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => bail!("retired Browser dataset evidence is not a regular file"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("inspect retired Browser dataset evidence"),
    }
}

async fn read_session_identities(database_path: &Path) -> Result<Vec<(String, String)>> {
    let pool = SqlitePoolOptions::new().max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(database_path).read_only(true))
        .await.context("open retired dataset for Browser cleanup without mutation")?;
    let result = async {
        let object_type: Option<String> = sqlx::query_scalar(
            "SELECT type FROM sqlite_schema WHERE name = 'agent_sessions' COLLATE NOCASE AND type IN ('table', 'view')",
        ).fetch_optional(&pool).await?;
        let Some(object_type) = object_type else { return Ok(vec![]); };
        if object_type != "table" {
            bail!("retired canonical Session identity object is not a table");
        }
        let columns: Vec<String> = sqlx::query("PRAGMA table_info(agent_sessions)")
            .fetch_all(&pool).await?.into_iter().map(|row| row.get("name")).collect();
        if !columns.iter().any(|name| name == "agent_session_id")
            || !columns.iter().any(|name| name == "owner_ref_json") {
            bail!("retired canonical Session table is missing required identity columns");
        }
        let rows = sqlx::query("SELECT agent_session_id, owner_ref_json FROM agent_sessions")
            .fetch_all(&pool).await?;
        rows.into_iter().map(|row| {
            let owner: PrincipalRef = serde_json::from_str(row.try_get("owner_ref_json")?)
                .context("invalid retired canonical Session owner during Browser cleanup")?;
            if owner.principal_kind != "user" {
                bail!("retired Browser Session owner is not a user");
            }
            Ok((owner.principal_id, row.try_get("agent_session_id")?))
        }).collect::<Result<Vec<_>>>()
    }.await;
    pool.close().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cleanup_has_no_canonical_identities_when_session_table_is_absent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("retired.db");
        let pool = SqlitePoolOptions::new().max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(true))
            .await.unwrap();
        sqlx::query("CREATE TABLE unrelated(value TEXT)").execute(&pool).await.unwrap();
        pool.close().await;
        assert!(read_session_identities(&path).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cleanup_rejects_malformed_identity_columns_even_when_session_table_is_empty() {
        for table in ["agent_sessions", "AGENT_SESSIONS"] {
            for columns in ["agent_session_id TEXT", "owner_ref_json TEXT", "unrelated TEXT"] {
                for with_row in [false, true] {
                    let directory = tempfile::tempdir().unwrap();
                    let path = directory.path().join("retired.db");
                    let pool = SqlitePoolOptions::new().max_connections(1)
                        .connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(true))
                        .await.unwrap();
                    sqlx::query(&format!("CREATE TABLE {table}({columns})"))
                        .execute(&pool).await.unwrap();
                    if with_row {
                        sqlx::query("INSERT INTO agent_sessions DEFAULT VALUES")
                            .execute(&pool).await.unwrap();
                    }
                    pool.close().await;
                    let error = read_session_identities(&path).await.unwrap_err();
                    assert!(error.to_string().contains("missing required identity columns"),
                        "table {table}, schema {columns}, with_row={with_row}: {error}");
                }
            }
        }
    }

    #[tokio::test]
    async fn cleanup_rejects_a_view_in_place_of_the_session_table() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("retired.db");
        let pool = SqlitePoolOptions::new().max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(true))
            .await.unwrap();
        sqlx::query("CREATE VIEW agent_sessions AS SELECT 1 AS unrelated WHERE 0")
            .execute(&pool).await.unwrap();
        pool.close().await;
        let error = read_session_identities(&path).await.unwrap_err();
        assert!(error.to_string().contains("identity object is not a table"));
    }

    #[tokio::test]
    async fn cleanup_reads_exact_canonical_owners_including_deleted_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("retired.db");
        let pool = SqlitePoolOptions::new().max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&path).create_if_missing(true))
            .await.unwrap();
        sqlx::query("CREATE TABLE agent_sessions(agent_session_id TEXT, owner_ref_json TEXT, state TEXT)")
            .execute(&pool).await.unwrap();
        for (session, state) in [("one", "live"), ("two", "deleted")] {
            sqlx::query("INSERT INTO agent_sessions VALUES (?, ?, ?)")
                .bind(session).bind(r#"{"principal_kind":"user","principal_id":"owner"}"#)
                .bind(state).execute(&pool).await.unwrap();
        }
        pool.close().await;
        assert_eq!(read_session_identities(&path).await.unwrap(),
            vec![("owner".into(), "one".into()), ("owner".into(), "two".into())]);
    }

    #[test]
    fn cleanup_rejects_linked_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        std::fs::write(&target, "private").unwrap();
        let link = directory.path().join("linked");
        std::os::unix::fs::symlink(target, &link).unwrap();
        assert!(regular_file_exists(&link).is_err());
    }

    #[tokio::test]
    async fn native_cleanup_is_required_even_when_build_has_no_browser_feature() {
        let result = remove_profile_identities(
            vec![("owner".into(), "session".into())],
            "0190f5fe-7c00-7a00-8000-000000000002",
            #[cfg(feature = "browser-use")] None,
        ).await;
        assert!(result.unwrap_err().to_string().contains("start NomiFun Desktop"));
        remove_profile_identities(
            vec![], "0190f5fe-7c00-7a00-8000-000000000002",
            #[cfg(feature = "browser-use")] None,
        ).await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_rejects_invalid_generation_before_attempting_native_removal() {
        let error = remove_profile_identities(
            vec![("owner".into(), "session".into())], "not-a-dataset-generation",
            #[cfg(feature = "browser-use")] None,
        ).await.unwrap_err();
        assert!(error.to_string().contains("invalid retired Browser storage generation"));
    }
}
