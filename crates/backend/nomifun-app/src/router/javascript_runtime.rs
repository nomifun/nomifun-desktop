//! Startup composition for the single process-wide JavaScript Runtime.
//!
//! Runtime selection and hot switching are intentionally absent. The verified
//! authority is an internal Host dependency, not a user-selectable setting.

use std::sync::Arc;

use nomifun_js_runtime::RuntimeAuthority;

pub(crate) struct JavaScriptRuntimeFoundation {
    authority: Arc<RuntimeAuthority>,
}

impl JavaScriptRuntimeFoundation {
    pub(crate) fn authority(&self) -> Arc<RuntimeAuthority> {
        Arc::clone(&self.authority)
    }
}

pub(crate) async fn build_javascript_runtime_foundation(
    _pool: nomifun_db::SqlitePool,
    _data_root: std::path::PathBuf,
) -> anyhow::Result<JavaScriptRuntimeFoundation> {
    let authority = RuntimeAuthority::discover().await?;
    Ok(JavaScriptRuntimeFoundation { authority })
}
