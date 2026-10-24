use nomifun_common::DatabaseUnavailableReason;

/// Classify only SQLite contention and pool admission timeouts. Syntax errors,
/// missing/corrupt data, I/O failures and a closed pool remain failures. This
/// does not establish whether a write executed and must never trigger replay.
pub fn database_unavailable_reason(error: &sqlx::Error) -> Option<DatabaseUnavailableReason> {
    match error {
        sqlx::Error::PoolTimedOut => Some(DatabaseUnavailableReason::PoolTimedOut),
        sqlx::Error::Database(error) => {
            error.try_downcast_ref::<sqlx::sqlite::SqliteError>()?;
            // Extended SQLite result codes retain the primary code in the low
            // byte (for example SQLITE_BUSY_SNAPSHOT). Do not parse prose.
            let code = error.code()?.parse::<u32>().ok()? & 0xff;
            match code {
                5 => Some(DatabaseUnavailableReason::Busy),
                6 => Some(DatabaseUnavailableReason::Locked),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "availability_tests.rs"]
mod tests;
