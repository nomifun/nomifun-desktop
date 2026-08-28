/// The singleton installation-scoped Remote front-door token row.
///
/// Only the hash is persisted; plaintext is returned exactly once when minted.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InstanceApiTokenRow {
    pub id: i64,
    pub singleton_key: String,
    pub token_hash: String,
    pub created_at: i64,
}
