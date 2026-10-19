//! One engineering cutover before migration 004. No retired execution reader.
//! Keep ordinary files and consistent SQLite snapshots for manual conversion.
use std::{fs, io::{Read, Write}, path::Path};
use sha2::{Digest, Sha256};
use sqlx::{Column, Connection, Row, SqliteConnection, ValueRef, sqlite::SqliteConnectOptions};
use crate::DbError;

const ROOTS: &[&str] = &["plugin-core", "plugin-data", "plugin-drafts"];
const TABLES: &[&str] = &["plugins", "plugin_artifacts", "plugin_drafts", "plugin_credential_bindings", "plugin_grants", "plugin_library_state", "plugin_mutations"];

fn file_error(error: impl std::fmt::Display) -> DbError {
    DbError::Init(format!("Retired application works could not be protected; schema cutover refused: {error}"))
}

pub(crate) async fn protect_before_retirement(conn: &mut SqliteConnection) -> Result<(), DbError> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='plugins'")
        .fetch_one(&mut *conn).await?;
    if exists == 0 { return Ok(()); }
    // The application composition has already acquired the data-root Host
    // lock. The previous Host and its contained process trees must be closed.
    let files: Vec<(i64,String,String)> = sqlx::query_as("PRAGMA database_list").fetch_all(&mut *conn).await?;
    let Some((_,_,filename)) = files.iter().find(|(_,name,_)| name == "main") else {
        return Err(file_error("main database path is unavailable"));
    };
    if filename.is_empty() { return Ok(()); } // isolated in-memory fixtures
    // Valid receipts alone do not authorize retiring works when canonical
    // generation metadata is missing or differs. Check before destructive DDL.
    let metadata: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_metadata WHERE singleton_key='canonical'")
        .fetch_one(&mut *conn).await?;
    if metadata != 1 { return Err(file_error("canonical generation metadata is incomplete")); }
    crate::agent_store_clean_cut::validate_metadata_before_plugin_retirement(conn).await?;
    let root = Path::new(filename).parent().ok_or_else(|| file_error("database has no parent"))?;
    let protected_root = root.join("preserved-plugin-works");
    fs::create_dir_all(&protected_root).map_err(file_error)?;
    let destination = protected_root.join(uuid::Uuid::now_v7().to_string());
    fs::create_dir(&destination).map_err(file_error)?;
    let mut registration = serde_json::Map::new();
    for table in TABLES {
        let rows = sqlx::query(&format!("SELECT * FROM {table}")).fetch_all(&mut *conn).await?;
        let mut values = Vec::with_capacity(rows.len());
        for row in rows {
            let mut fields = serde_json::Map::new();
            for column in row.columns() {
                let value = row.try_get_raw(column.ordinal())?;
                let value = if value.is_null() { serde_json::Value::Null }
                    else if let Ok(value) = row.try_get::<i64,_>(column.ordinal()) { value.into() }
                    else if let Ok(value) = row.try_get::<String,_>(column.ordinal()) { value.into() }
                    else { return Err(file_error(format!("unexpected value in retired {table}.{}",column.name()))); };
                fields.insert(column.name().to_owned(), value);
            }
            values.push(serde_json::Value::Object(fields));
        }
        registration.insert((*table).to_owned(), serde_json::Value::Array(values));
    }
    let bytes = serde_json::to_vec_pretty(&registration).map_err(file_error)?;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(destination.join("registration.json")).map_err(file_error)?;
    file.write_all(&bytes).map_err(file_error)?;
    file.sync_all().map_err(file_error)?;
    for name in ROOTS {
        let source = root.join(name);
        if source.exists() { protect_tree(&source, &destination.join(name)).await?; }
    }
    fs::write(destination.join("README.txt"), "These are preserved works from the retired NomiFun plugin system. Source files and application data are ordinary files. data.sqlite includes committed WAL records. No application is executed here. Convert projects to ordinary programs and import them in Applications. The original import directories and canonical Agent history were not changed.\n").map_err(file_error)?;
    tracing::info!(path = %destination.display(), "Retired application files protected before schema cutover");
    Ok(())
}

async fn protect_tree(source: &Path, destination: &Path) -> Result<(), DbError> {
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    let mut databases = Vec::new();
    while let Some((source,destination)) = pending.pop() {
        let meta = fs::symlink_metadata(&source).map_err(file_error)?;
        // An external target is not an owned work file and must never be
        // traversed. Unknown special files stop the cutover with originals kept.
        if meta.file_type().is_symlink() { return Err(file_error(format!("link requires manual protection: {}",source.display()))); }
        if meta.is_dir() {
            fs::create_dir(&destination).map_err(file_error)?;
            for item in fs::read_dir(&source).map_err(file_error)? {
                let item = item.map_err(file_error)?;
                let name = item.file_name();
                if name == "data.sqlite-wal" || name == "data.sqlite-shm" { continue; }
                pending.push((item.path(), destination.join(name)));
            }
        } else if meta.is_file() {
            if source.file_name().is_some_and(|name| name == "data.sqlite") {
                databases.push((source,destination));
            } else {
                fs::copy(&source,&destination).map_err(file_error)?;
                if digest_file(&source)? != digest_file(&destination)? { return Err(file_error("work file changed during preservation")); }
                fs::OpenOptions::new().write(true).open(destination).map_err(file_error)?.sync_all().map_err(file_error)?;
            }
        } else { return Err(file_error(format!("unsupported work file: {}",source.display()))); }
    }
    for (source,destination) in databases {
        let opts = SqliteConnectOptions::new().filename(&source).read_only(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let mut database = SqliteConnection::connect_with(&opts).await?;
        // VACUUM reads through SQLite, including committed WAL frames. Never
        // invoke the retired Backup chain or copy only a live main file.
        sqlx::query("VACUUM INTO ?").bind(destination.to_string_lossy().as_ref()).execute(&mut database).await?;
        database.close().await?;
        let mut check = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&destination).read_only(true)).await?;
        let integrity: Vec<String> = sqlx::query_scalar("PRAGMA quick_check").fetch_all(&mut check).await?;
        check.close().await?;
        if integrity != ["ok"] { return Err(file_error("preserved SQLite integrity check failed")); }
        fs::OpenOptions::new().write(true).open(destination).map_err(file_error)?.sync_all().map_err(file_error)?;
    }
    Ok(())
}

fn digest_file(path: &Path) -> Result<Vec<u8>,DbError> {
    let mut file = fs::File::open(path).map_err(file_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(file_error)?;
        if read == 0 { break; }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().to_vec())
}
