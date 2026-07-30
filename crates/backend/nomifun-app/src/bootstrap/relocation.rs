//! Post-relocation absolute-path rewrite for the main database.
//!
//! `bootstrap::data_root` moves a legacy default data root
//! (`NomiFun/Nomi<suffix>`) into the current channel root and leaves a
//! [`RELOCATED_FROM_MARKER`] file (JSON, see
//! [`super::data_root::RelocationMarker`]) in the new data dir. The files
//! move, but absolute paths stored *inside* the database still point at the
//! old root:
//!
//! * `knowledge_bases.root_path` — the knowledge purge guard checks
//!   `starts_with({data_dir}/knowledge)`, so a stale prefix breaks both
//!   mounting and managed-purge.
//! * `conversations.extra` `$.workspace` — custom-workspace association used
//!   for session grouping and workspace reuse (managed workspaces are
//!   recomputed from their durable token and need no rewrite).
//! * `terminal_sessions.cwd` — the directory a terminal session relaunches in.
//! * `agent_executions.work_dir` / `agent_execution_templates.work_dir` —
//!   execution working roots.
//! * `channel_sessions.workspace` — channel-bot conversation workspaces.
//! * `knowledge_bindings.target_workpath` — workpath-addressed knowledge
//!   bindings.
//!
//! This step runs once after the database is open and migrated (end of
//! [`super::init_data_layer`]): it rewrites the old-root prefix to the new
//! data root and then renames the marker to [`RELOCATED_DONE_MARKER`].
//! Matching details:
//!
//! * **All separator spellings** of the root are tried (backslash, forward
//!   slash, and — on rows written by releases that persisted
//!   `fs::canonicalize` output verbatim — the Windows `\\?\` prefixed form),
//!   and for each spelling the boundary character right after the prefix may
//!   be either `\` or `/` (`Path::join` on Windows appends `\` even to a
//!   `/`-spelled prefix).
//! * **Matching is case-insensitive** (`lower(...)` on the comparison side
//!   only): Windows paths are case-insensitive, so `c:\users\...` rows must
//!   match a `C:\Users\...` root. The replacement side keeps the original
//!   suffix bytes untouched (`?new || substr(col, length(?old) + 1)` — ASCII
//!   case-folding preserves length, so the cut point is exact).
//! * `conversations.extra` rows are guarded by `json_valid(...)` so a single
//!   corrupt JSON blob cannot fail the whole rewrite, and all statements run
//!   inside **one transaction** — the rewrite is all-or-nothing.
//! * A marker whose `old_root` is suspiciously shallow (a drive root or a
//!   single top-level dir) is refused: such a prefix would rewrite half the
//!   database. See [`old_root_is_specific`].
//!
//! Any failure keeps the marker so the next boot retries (the rewrite is
//! idempotent: a rewritten row no longer matches the old prefix). It never
//! fails the boot.

use std::path::Path;

use nomifun_db::Database;
use tracing::{info, warn};

use super::data_root::{
    RELOCATED_DONE_MARKER, RELOCATED_FROM_MARKER, RelocationMarker,
};

/// Text columns rewritten with plain prefix replacement.
const TEXT_COLUMN_REWRITES: [(&str, &str); 6] = [
    ("knowledge_bases", "root_path"),
    ("terminal_sessions", "cwd"),
    ("agent_executions", "work_dir"),
    ("agent_execution_templates", "work_dir"),
    ("channel_sessions", "workspace"),
    ("knowledge_bindings", "target_workpath"),
];

/// One-shot, idempotent, never-fails-the-boot. See module docs.
pub async fn rewrite_relocated_paths(database: &Database, data_dir: &Path) {
    let marker_path = data_dir.join(RELOCATED_FROM_MARKER);
    if !marker_path.exists() {
        return;
    }

    let marker: RelocationMarker = match std::fs::read_to_string(&marker_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(marker) => marker,
        Err(e) => {
            // Unreadable marker: keep it (a deliberate bug signal — this
            // warns on every boot until someone looks at it) and do nothing.
            warn!(marker = %marker_path.display(), error = %e, "relocation: marker unreadable; skipping path rewrite");
            return;
        }
    };

    let old_root_owned = nomifun_common::paths::simplified(Path::new(
        marker.old_root.trim_end_matches(['/', '\\']),
    ))
    .display()
    .to_string();
    let old_root = old_root_owned.as_str();
    let new_root_owned = nomifun_common::paths::simplified(data_dir)
        .display()
        .to_string();
    let new_root = new_root_owned.trim_end_matches(['/', '\\']);
    if old_root.is_empty() {
        warn!(marker = %marker_path.display(), "relocation: marker has empty old_root; skipping path rewrite");
        return;
    }
    if !old_root_is_specific(old_root) {
        // Keep the marker as a bug signal (warns every boot), same as the
        // unreadable-marker case: rewriting with a drive-root prefix would
        // touch every absolute path in the database.
        warn!(
            marker = %marker_path.display(),
            old_root,
            "relocation: marker old_root is suspiciously shallow; refusing path rewrite"
        );
        return;
    }

    match rewrite_path_prefixes(database.pool(), old_root, new_root).await {
        Ok(rows) => {
            info!(rows, old_root, new_root, "relocation: rewrote absolute paths in database");
            // Close the gate. If this rename fails the next boot re-runs the
            // rewrite, which is a no-op (nothing matches the old prefix any
            // more), and retries the rename.
            let done = data_dir.join(RELOCATED_DONE_MARKER);
            let _ = std::fs::remove_file(&done);
            if let Err(e) = std::fs::rename(&marker_path, &done) {
                warn!(error = %e, "relocation: failed to finalize marker; rewrite will re-run (no-op) next boot");
            }
        }
        Err(e) => {
            // Keep the marker: retried on the next boot. Never block startup.
            warn!(error = %e, old_root, "relocation: database path rewrite failed; will retry next boot");
        }
    }
}

/// Guard against a corrupt or hand-crafted marker whose `old_root` is a
/// drive root or a single top-level directory (`C:\`, `C:\Temp`, `/tmp`):
/// used as a rewrite prefix it would match — and mangle — most absolute
/// paths in the database. The legitimate legacy roots
/// (`<app-data>/NomiFun/Nomi<suffix>`, `<temp>/nomifun-data/Nomi<suffix>`)
/// always have at least two non-drive components, on every platform.
fn old_root_is_specific(old_root: &str) -> bool {
    old_root
        .split(['/', '\\'])
        .filter(|seg| !seg.is_empty() && !seg.ends_with(':'))
        .count()
        >= 2
}

/// Rewrite the `old_root` prefix to `new_root` in every known absolute-path
/// location. Matching is case-insensitive and accepts either separator as
/// the boundary character after the prefix; the stored suffix bytes are
/// preserved verbatim (see module docs). All statements run in a single
/// transaction. Returns total rows touched.
pub async fn rewrite_path_prefixes(
    pool: &nomifun_db::SqlitePool,
    old_root: &str,
    new_root: &str,
) -> Result<u64, nomifun_db::sqlx::Error> {
    let mut affected = 0u64;
    // One transaction for everything: a mid-flight failure must not leave
    // some tables rewritten and others not — the marker stays and the whole
    // rewrite re-runs on the next boot from a consistent state.
    let mut tx = pool.begin().await?;
    for (old, new) in prefix_variants(old_root, new_root) {
        // Plain text columns. Prefix matching uses exact `substr` comparison
        // (no LIKE — backslashes and underscores in Windows paths would need
        // escaping), wrapped in `lower()` because Windows paths are
        // case-insensitive. It requires either full equality or a path
        // separator (`\` or `/`, mixed spellings happen via Path::join) right
        // after the prefix, so `...\NomiOther` is never mistaken for `...\Nomi`.
        for (table, column) in TEXT_COLUMN_REWRITES {
            let sql = format!(
                "UPDATE {table} SET {column} = ?2 || substr({column}, length(?1) + 1) \
                 WHERE {column} IS NOT NULL AND (lower({column}) = lower(?1) \
                    OR lower(substr({column}, 1, length(?1) + 1)) IN (lower(?1) || '\\', lower(?1) || '/'))"
            );
            affected += nomifun_db::sqlx::query(&sql)
                .bind(&old)
                .bind(&new)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        }

        // `conversations.extra` is a JSON object; only its `workspace` key
        // holds an absolute path. `json_set` keeps every other key intact.
        // The `CASE WHEN json_valid(...)` wrapper (NOT a plain `json_valid()
        // AND ...` — SQLite may reorder AND operands, CASE branches are
        // guaranteed lazy) turns corrupt blobs into NULL so a single bad row
        // neither errors the UPDATE nor blocks the rewrite of valid rows.
        affected += nomifun_db::sqlx::query(
            "UPDATE conversations SET extra = json_set(extra, '$.workspace', \
                 ?2 || substr(json_extract(extra, '$.workspace'), length(?1) + 1)) \
             WHERE lower(json_extract(CASE WHEN json_valid(extra) THEN extra END, '$.workspace')) = lower(?1) \
                OR lower(substr(json_extract(CASE WHEN json_valid(extra) THEN extra END, '$.workspace'), \
                                1, length(?1) + 1)) \
                       IN (lower(?1) || '\\', lower(?1) || '/')",
        )
        .bind(&old)
        .bind(&new)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }
    tx.commit().await?;
    Ok(affected)
}

/// The (old, new) replacement pairs to run: the backslash spelling, the
/// forward-slash spelling, and — on Windows shapes — the `\\?\`-prefixed
/// verbatim spelling of the same root. Rows were written by different code
/// paths over time (`Path::display` on Windows yields `\`,
/// frontend-supplied or normalized paths may use `/`, and releases before
/// the path-simplification fix persisted verbatim `fs::canonicalize` output),
/// so all forms can exist in one database. Each stored path keeps its own
/// suffix verbatim; every variant maps to the plain (non-verbatim) new root.
fn prefix_variants(old_root: &str, new_root: &str) -> Vec<(String, String)> {
    let old_bs = old_root.replace('/', "\\");
    let new_bs = new_root.replace('/', "\\");
    let mut candidates = vec![
        (old_bs.clone(), new_bs.clone()),
        (old_root.replace('\\', "/"), new_root.replace('\\', "/")),
    ];
    // Verbatim spelling only makes sense for plain drive-letter paths.
    let is_drive_path = old_bs.len() >= 3
        && old_bs.as_bytes()[0].is_ascii_alphabetic()
        && old_bs.as_bytes()[1] == b':'
        && old_bs.as_bytes()[2] == b'\\';
    if is_drive_path {
        candidates.push((format!(r"\\?\{old_bs}"), new_bs));
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for cand in candidates {
        // Skip degenerate (old == new) and duplicate variants (on Unix both
        // separator spellings of a `/`-only path collapse into one).
        if cand.0 != cand.1 && !out.iter().any(|v| v.0 == cand.0) {
            out.push(cand);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::{ConversationId, TerminalId};
    use nomifun_db::sqlx;

    const OLD: &str = r"C:\Users\u\AppData\Local\NomiFun\Nomi";
    const NEW: &str = r"C:\Users\u\AppData\Local\NomiFun";
    const OLD_FS: &str = "C:/Users/u/AppData/Local/NomiFun/Nomi";
    const OLD_VERBATIM: &str = r"\\?\C:\Users\u\AppData\Local\NomiFun\Nomi";

    async fn test_database() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let database =
            nomifun_db::init_database(&dir.path().join("nomifun-backend.db"))
                .await
                .unwrap();
        (dir, database)
    }

    async fn insert_kb(pool: &nomifun_db::SqlitePool, id: &str, root_path: &str) {
        sqlx::query(
            "INSERT INTO knowledge_bases \
                 (knowledge_base_id, name, description, root_path, managed, extra, created_at, updated_at) \
             VALUES (?, ?, '', ?, 1, '{}', 1, 1)",
        )
        .bind(id)
        .bind(id)
        .bind(root_path)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn kb_root(pool: &nomifun_db::SqlitePool, id: &str) -> String {
        sqlx::query_scalar(
            "SELECT root_path FROM knowledge_bases WHERE knowledge_base_id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn insert_terminal(
        pool: &nomifun_db::SqlitePool,
        id: &TerminalId,
        cwd: &str,
    ) {
        let owner = nomifun_db::installation_owner_id(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO terminal_sessions \
                 (terminal_id, user_id, name, cwd, command, args, created_at, updated_at) \
             VALUES (?, ?, 't', ?, 'bash', '[]', 1, 1)",
        )
        .bind(id.as_str())
        .bind(owner)
        .bind(cwd)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_conversation(
        pool: &nomifun_db::SqlitePool,
        id: &ConversationId,
        extra: &str,
    ) {
        let owner = nomifun_db::installation_owner_id(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO conversations \
                 (conversation_id, user_id, name, type, extra, created_at, updated_at) \
             VALUES (?, ?, 'c', 'nomi', ?, 1, 1)",
        )
        .bind(id.as_str())
        .bind(owner)
        .bind(extra)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn rewrites_all_stored_spellings_and_preserves_suffixes() {
        let (_dir, database) = test_database().await;
        let pool = database.pool();
        insert_kb(pool, &nomifun_common::KnowledgeBaseId::new().into_string(), &format!(r"{OLD}\knowledge\kb-a")).await;
        let kb_fs = nomifun_common::KnowledgeBaseId::new().into_string();
        insert_kb(pool, &kb_fs, &format!("{OLD_FS}/knowledge/kb-b")).await;
        let kb_verbatim = nomifun_common::KnowledgeBaseId::new().into_string();
        insert_kb(pool, &kb_verbatim, &format!(r"{OLD_VERBATIM}\knowledge\kb-c")).await;
        let kb_external = nomifun_common::KnowledgeBaseId::new().into_string();
        insert_kb(pool, &kb_external, r"D:\external\kb").await;
        let kb_lookalike = nomifun_common::KnowledgeBaseId::new().into_string();
        insert_kb(
            pool,
            &kb_lookalike,
            r"C:\Users\u\AppData\Local\NomiFun\NomiOther\kb",
        )
        .await;
        let terminal = TerminalId::new();
        insert_terminal(pool, &terminal, &format!(r"{OLD}\conversations\ws-1")).await;
        let conversation = ConversationId::new();
        insert_conversation(
            pool,
            &conversation,
            &format!(
                r#"{{"workspace":"{}"}}"#,
                format!(r"{OLD}\conversations\ws-2").replace('\\', "\\\\")
            ),
        )
        .await;
        // The v3 schema CHECK-constrains `extra` to valid JSON objects, so a
        // corrupt blob cannot exist in a v3 database; the CASE WHEN
        // json_valid(...) guard in the UPDATE stays purely defensive.
        let untouched = ConversationId::new();
        insert_conversation(pool, &untouched, r#"{"note":"no workspace"}"#).await;

        let affected = rewrite_path_prefixes(pool, OLD, NEW).await.unwrap();

        assert!(affected >= 5, "expected >=5 rewritten rows, got {affected}");
        let roots: Vec<String> = sqlx::query_scalar(
            "SELECT root_path FROM knowledge_bases ORDER BY root_path",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert!(
            roots.contains(&format!(r"{NEW}\knowledge\kb-a"))
                && roots.contains(&format!("C:/Users/u/AppData/Local/NomiFun/knowledge/kb-b"))
                && roots.contains(&format!(r"{NEW}\knowledge\kb-c")),
            "all spellings must be rewritten onto the new root, got {roots:?}"
        );
        assert_eq!(kb_root(pool, &kb_external).await, r"D:\external\kb");
        assert_eq!(
            kb_root(pool, &kb_lookalike).await,
            r"C:\Users\u\AppData\Local\NomiFun\NomiOther\kb",
            "prefix boundary must not match sibling directories"
        );
        let cwd: String = sqlx::query_scalar(
            "SELECT cwd FROM terminal_sessions WHERE terminal_id = ?",
        )
        .bind(terminal.as_str())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(cwd, format!(r"{NEW}\conversations\ws-1"));
        let workspace: String = sqlx::query_scalar(
            "SELECT json_extract(extra, '$.workspace') FROM conversations WHERE conversation_id = ?",
        )
        .bind(conversation.as_str())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(workspace, format!(r"{NEW}\conversations\ws-2"));
        let untouched_extra: String = sqlx::query_scalar(
            "SELECT extra FROM conversations WHERE conversation_id = ?",
        )
        .bind(untouched.as_str())
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(
            untouched_extra, r#"{"note":"no workspace"}"#,
            "rows without a workspace stay untouched"
        );
        database.close().await;
    }

    #[tokio::test]
    async fn rewrite_is_idempotent() {
        let (_dir, database) = test_database().await;
        let pool = database.pool();
        let kb = nomifun_common::KnowledgeBaseId::new().into_string();
        insert_kb(pool, &kb, &format!(r"{OLD}\knowledge\kb")).await;

        rewrite_path_prefixes(pool, OLD, NEW).await.unwrap();
        let affected = rewrite_path_prefixes(pool, OLD, NEW).await.unwrap();

        assert_eq!(affected, 0, "second run must be a no-op");
        assert_eq!(kb_root(pool, &kb).await, format!(r"{NEW}\knowledge\kb"));
        database.close().await;
    }

    #[test]
    fn shallow_old_roots_are_refused() {
        assert!(!old_root_is_specific(r"C:\"));
        assert!(!old_root_is_specific(r"C:\Temp"));
        assert!(!old_root_is_specific("/tmp"));
        assert!(old_root_is_specific(r"C:\Users\u\AppData\Local\NomiFun\Nomi"));
        assert!(old_root_is_specific("/home/u/.local/share/NomiFun/Nomi"));
    }

    #[test]
    fn verbatim_variant_only_added_for_drive_paths() {
        let variants = prefix_variants(OLD, NEW);
        assert!(
            variants
                .iter()
                .any(|(old, _)| old == OLD_VERBATIM),
            "drive-letter roots gain a verbatim variant: {variants:?}"
        );
        let unix = prefix_variants("/data/old", "/data/new");
        assert!(
            unix.iter().all(|(old, _)| !old.starts_with(r"\\")),
            "unix roots gain no verbatim variant: {unix:?}"
        );
    }
}
