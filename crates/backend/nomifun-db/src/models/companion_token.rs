use nomifun_common::TimestampMs;

/// One row of `companion_access_token`: a per-companion Remote front-door token,
/// stored only as its SHA-256 hash. `companion_id` is the primary key, so each
/// companion holds at most one live token (minting again rotates it).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CompanionApiTokenRow {
    pub companion_id: String,
    pub token_hash: String,
    pub created_at: TimestampMs,
}
