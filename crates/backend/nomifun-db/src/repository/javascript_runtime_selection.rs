use std::collections::BTreeSet;

use serde_json::Value;

use crate::{DbError, JavaScriptRuntimeSelectionRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveJavaScriptRuntimeSelectionParams {
    pub expected_revision: i64,
    pub selected_runtime: Option<Value>,
    pub pending_candidate: Option<Value>,
    pub validation_result: Option<Value>,
    pub last_error_code: Option<String>,
    pub non_recommended_warning_acknowledged: BTreeSet<String>,
    pub updated_at: i64,
}

#[async_trait::async_trait]
pub trait IJavaScriptRuntimeSelectionRepository: Send + Sync {
    async fn load(&self) -> Result<JavaScriptRuntimeSelectionRecord, DbError>;

    async fn save_cas(
        &self,
        params: &SaveJavaScriptRuntimeSelectionParams,
    ) -> Result<JavaScriptRuntimeSelectionRecord, DbError>;
}
