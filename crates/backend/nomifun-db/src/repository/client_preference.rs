use crate::error::DbError;
use crate::models::ClientPreference;

/// Client preference data access abstraction.
///
/// Provides CRUD operations on the generic key-value `client_preferences` table.
#[async_trait::async_trait]
pub trait IClientPreferenceRepository: Send + Sync {
    /// Returns all client preferences.
    async fn get_all(&self) -> Result<Vec<ClientPreference>, DbError>;

    /// Returns preferences for the given keys only.
    /// Keys that don't exist are simply omitted from the result.
    async fn get_by_keys(&self, keys: &[&str]) -> Result<Vec<ClientPreference>, DbError>;

    /// Inserts or updates a batch of key-value pairs.
    async fn upsert_batch(&self, entries: &[(&str, &str)]) -> Result<(), DbError>;

    /// Deletes the given keys.
    async fn delete_keys(&self, keys: &[&str]) -> Result<(), DbError>;
}
