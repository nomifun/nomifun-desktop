use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct JavaScriptRuntimeSelectionRow {
    pub id: i64,
    pub singleton_key: String,
    pub selected_runtime_json: Option<String>,
    pub pending_candidate_json: Option<String>,
    pub validation_result_json: Option<String>,
    pub last_error_code: Option<String>,
    pub non_recommended_warning_acknowledged_json: String,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaScriptRuntimeSelectionRecord {
    pub selected_runtime: Option<Value>,
    pub pending_candidate: Option<Value>,
    pub validation_result: Option<Value>,
    pub last_error_code: Option<String>,
    pub non_recommended_warning_acknowledged: BTreeSet<String>,
    pub revision: i64,
    pub updated_at: i64,
}

impl Default for JavaScriptRuntimeSelectionRecord {
    fn default() -> Self {
        Self {
            selected_runtime: None,
            pending_candidate: None,
            validation_result: None,
            last_error_code: None,
            non_recommended_warning_acknowledged: BTreeSet::new(),
            revision: 0,
            updated_at: 0,
        }
    }
}
