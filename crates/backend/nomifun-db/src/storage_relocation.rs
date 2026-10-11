//! Typed mutable domain pointers for relocating one healthy current dataset.
//! This owner never rewrites historical facts or external resource roots.
use std::{path::Path, collections::BTreeMap};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use nomifun_agent_contracts::{AgentBindingValue, ManagedSessionWorkspaceNamespaceV1, PrincipalRef, is_default_managed_workspace,
    managed_workspace_parameters, relocate_storage_path, storage_paths_equal};
use crate::DbError;

struct PathChange { table: &'static str, column: &'static str, rowid: i64, next: String }
fn text(path: &Path) -> String { path.to_string_lossy().into_owned() }

fn managed_session_path(raw: &str, source_work: &Path, target: &Path) -> Option<String> {
    let source = source_work.join("conversations");
    let placeholder = relocate_storage_path(raw, &text(&source), "managed")?;
    let relative = Path::new(&placeholder).strip_prefix("managed").ok()?;
    let session = relative.components().next()?.as_os_str().to_str()?;
    if nomifun_common::validate_uuidv7(session).is_err() { return None; }
    relocate_storage_path(raw, &text(&source), &text(&target.join("conversations")))
}

/// Product ownership comes from current typed Session authority, never from a
/// folder name or a textual prefix. Explicit selected resources stay external
/// unless the canonical writer separately proves their Session-owned namespace.
async fn managed_sessions(tx: &mut Transaction<'_, Sqlite>, source_work: &Path)
    -> Result<(BTreeMap<String, String>, Vec<String>), DbError> {
    let mut managed = BTreeMap::new(); let mut external = Vec::new();
    for row in sqlx::query("SELECT agent_session_id, owner_ref_json, agent_binding_json FROM agent_sessions WHERE state='live' AND remote_binding_id IS NULL")
        .fetch_all(&mut **tx).await? {
        let binding: AgentBindingValue = serde_json::from_str(&row.try_get::<String,_>("agent_binding_json")?)
            .map_err(|e| DbError::Conflict(format!("invalid current Session binding: {e}")))?;
        let owner: PrincipalRef = serde_json::from_str(&row.try_get::<String,_>("owner_ref_json")?)
            .map_err(|e| DbError::Conflict(format!("invalid current Session owner: {e}")))?;
        let resources = binding.typed_resource_bindings.iter().filter(|resource|
            matches!(resource.resource_kind.as_ref(), "workspace" | "process_session")).collect::<Vec<_>>();
        let selected = resources.iter().any(|resource| !is_default_managed_workspace(resource));
        for resource in resources.iter().filter(|resource| !is_default_managed_workspace(resource)) {
            if let Some(root) = resource.typed_parameters.get("workspace_root") { external.push(root.clone()); }
        }
        if !selected && !resources.is_empty() && resources.iter().all(|resource|
            resource.owner_id == owner.principal_id
                && (resource.typed_parameters == managed_workspace_parameters()
                    || (resource.typed_parameters.len() == 1 && resource.typed_parameters.get("workspace_root")
                        .is_some_and(|root| storage_paths_equal(root, &text(source_work)))))) {
            managed.insert(row.try_get("agent_session_id")?, owner.principal_id.to_string());
        }
    }
    Ok((managed, external))
}

async fn changes(tx: &mut Transaction<'_, Sqlite>, source_data: &Path, source_work: &Path, target: &Path,
    managed_namespaces: &[ManagedSessionWorkspaceNamespaceV1])
    -> Result<Vec<PathChange>, DbError> {
    let mut changes = Vec::new();
    let (mut managed, external) = managed_sessions(tx, source_work).await?;
    // These transient receipts come from the canonical staging writer, which
    // validates the opening definition and complete relocation history. The
    // domain registry does not implement a second Session history reader.
    for namespace in managed_namespaces {
        namespace.validate().map_err(DbError::Conflict)?;
        nomifun_common::validate_uuidv7(namespace.agent_session_id.as_ref())
            .map_err(|error| DbError::Conflict(format!("invalid managed Session namespace: {error}")))?;
        let owner_json: Option<String> = sqlx::query_scalar("SELECT owner_ref_json FROM agent_sessions WHERE agent_session_id=? AND state='live' AND remote_binding_id IS NULL")
            .bind(namespace.agent_session_id.as_ref()).fetch_optional(&mut **tx).await?;
        let owner: PrincipalRef = serde_json::from_str(&owner_json.ok_or_else(||
            DbError::Conflict("managed namespace receipt has no local current Session".into()))?)
            .map_err(|error| DbError::Conflict(format!("invalid managed namespace owner: {error}")))?;
        if owner.principal_id != namespace.opening_workspace.owner_id {
            return Err(DbError::Conflict("managed namespace receipt differs from its Session owner".into()));
        }
        managed.insert(namespace.agent_session_id.as_ref().to_owned(), owner.principal_id.to_string());
    }
    for row in sqlx::query("SELECT id, knowledge_base_id, root_path FROM knowledge_bases WHERE managed = 1").fetch_all(&mut **tx).await? {
        let id: String = row.try_get("knowledge_base_id")?; let path: String = row.try_get("root_path")?;
        let old = source_data.join("knowledge").join(&id); let new = target.join("knowledge").join(&id);
        if !storage_paths_equal(&path, &text(&old)) && !storage_paths_equal(&path, &text(&new)) {
            return Err(DbError::Conflict(format!("managed knowledge base {id} has no exact product-owned source path")));
        }
        changes.push(PathChange { table: "knowledge_bases", column: "root_path", rowid: row.try_get("id")?, next: text(&new) });
    }
    // Automation request/template work_dir values are explicit user choices or
    // historical execution metadata. Runtime Session resolution owns defaults.
    for (table, column, proof_columns) in [("terminal_sessions", "cwd", ", user_id"),
        ("channel_sessions", "workspace", ", conversation_id"), ("knowledge_bindings", "target_workpath", "")] {
        let query = format!("SELECT rowid AS relocation_rowid, {column} AS path{proof_columns} FROM {table} WHERE {column} IS NOT NULL");
        for row in sqlx::query(&query).fetch_all(&mut **tx).await? {
            let raw: String = row.try_get("path")?;
            let relative = relocate_storage_path(&raw, &text(&source_work.join("conversations")), "managed");
            let session = relative.as_deref().and_then(|p| Path::new(p).strip_prefix("managed").ok())
                .and_then(|p| p.components().next()).and_then(|p| p.as_os_str().to_str());
            let proven = session.and_then(|id| managed.get(id)).is_some_and(|owner| match table {
                "terminal_sessions" => row.try_get::<String,_>("user_id").is_ok_and(|user| user == *owner),
                "channel_sessions" => row.try_get::<Option<String>,_>("conversation_id").is_ok_and(|id| id.as_deref() == session),
                _ => true,
            });
            let selected_external = external.iter().any(|root| relocate_storage_path(&raw, root, "external").is_some());
            if proven && !selected_external && let Some(next) = managed_session_path(&raw, source_work, target) {
                changes.push(PathChange { table, column, rowid: row.try_get("relocation_rowid")?, next });
            }
        }
    }
    for row in sqlx::query("SELECT rowid AS relocation_rowid, draft_id, owner_user_id, workspace_path FROM plugin_drafts").fetch_all(&mut **tx).await? {
        let id: String = row.try_get("draft_id")?; let owner: String = row.try_get("owner_user_id")?;
        let raw: String = row.try_get("workspace_path")?;
        let old = source_data.join("plugin-drafts").join(&owner).join(&id);
        let new = target.join("plugin-drafts").join(&owner).join(&id);
        if !storage_paths_equal(&raw, &text(&old)) && !storage_paths_equal(&raw, &text(&new)) {
            return Err(DbError::Conflict(format!("Plugin Draft {id} has no exact product-owned workspace")));
        }
        changes.push(PathChange { table: "plugin_drafts", column: "workspace_path", rowid: row.try_get("relocation_rowid")?, next: text(&new) });
    }
    Ok(changes)
}

/// Read-only preflight uses the same ownership checks as the offline writer.
pub async fn validate_storage_relocation_paths(pool: &SqlitePool, source_data: &Path, source_work: &Path) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    changes(&mut tx, source_data, source_work, source_data, &[]).await?;
    tx.rollback().await?;
    Ok(())
}

/// Call only against the offline copied database, after canonical Session
/// resources have proved their old knowledge-root references. Namespace receipts
/// must be the successful canonical writer's output for this staging operation.
pub async fn relocate_storage_domain_paths(pool: &SqlitePool, source_data: &Path, source_work: &Path, target: &Path,
    managed_namespaces: &[ManagedSessionWorkspaceNamespaceV1]) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    for change in changes(&mut tx, source_data, source_work, target, managed_namespaces).await? {
        // Identifiers come exclusively from the static domain registry above.
        sqlx::query(&format!("UPDATE {} SET {} = ? WHERE rowid = ?", change.table, change.column))
            .bind(change.next).bind(change.rowid).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_session_paths_require_a_component_boundary_and_session_identity() {
        let source = std::env::temp_dir().join("relocation-source"); let target = std::env::temp_dir().join("relocation-target");
        let session = "0190f5fe-7c00-7a00-8000-0000000000f1";
        assert_eq!(managed_session_path(&text(&source.join("conversations").join(session).join("file")), &source, &target), Some(text(&target.join("conversations").join(session).join("file"))));
        assert!(managed_session_path(&text(&source.join("conversations-other").join(session)), &source, &target).is_none());
        assert!(managed_session_path(&text(&source.join("conversations").join("external-project")), &source, &target).is_none());
    }

    #[tokio::test]
    async fn a_uuid_named_directory_does_not_authorize_rewriting_a_selected_terminal_path() {
        let db = crate::init_database_memory().await.unwrap();
        let owner: String = sqlx::query_scalar("SELECT owner_user_id FROM installation_identity").fetch_one(db.pool()).await.unwrap();
        let source = std::env::temp_dir().join("storage-proof-source"); let target = std::env::temp_dir().join("storage-proof-target");
        let selected = text(&source.join("conversations").join("0190f5fe-7c00-7a00-8000-0000000000f1"));
        sqlx::query("INSERT INTO terminal_sessions(terminal_id,name,cwd,command,created_at,updated_at,user_id) VALUES(?,'selected',?,'shell',1,1,?)")
            .bind("0190f5fe-7c00-7a00-8000-0000000000f2").bind(&selected).bind(owner).execute(db.pool()).await.unwrap();
        relocate_storage_domain_paths(db.pool(), &source, &source, &target, &[]).await.unwrap();
        let actual: String = sqlx::query_scalar("SELECT cwd FROM terminal_sessions").fetch_one(db.pool()).await.unwrap();
        assert_eq!(actual, selected);
        db.close().await;
    }
}
