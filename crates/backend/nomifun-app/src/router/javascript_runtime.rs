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

pub(crate) async fn build_javascript_runtime_foundation()
-> anyhow::Result<JavaScriptRuntimeFoundation> {
    let mut progress = nomifun_runtime::subscribe_tool_preparation(nomifun_runtime::ToolId::Node);
    let preparation_needed = progress.borrow().phase != nomifun_runtime::ToolPreparationPhase::Ready;
    if preparation_needed {
        eprintln!(
            "Preparing the approved Node runtime for application startup. First preparation may require a network connection; completed preparation is reused offline."
        );
        report_node_preparation(&progress.borrow());
    }
    let discovery = RuntimeAuthority::discover();
    tokio::pin!(discovery);
    let mut observing = preparation_needed;
    loop {
        tokio::select! {
            biased;
            result = &mut discovery => {
                if preparation_needed || result.is_err() {
                    report_node_preparation(&nomifun_runtime::tool_preparation_snapshot(nomifun_runtime::ToolId::Node));
                }
                let authority = result.map_err(|error| anyhow::anyhow!("Required Node startup preparation failed: {error}. Check network/component preparation and retry startup."))?;
                return Ok(JavaScriptRuntimeFoundation { authority });
            },
            changed = progress.changed(), if observing => {
                if changed.is_ok() { report_node_preparation(&progress.borrow()); } else { observing = false; }
            },
        }
    }
}

fn report_node_preparation(snapshot: &nomifun_runtime::ToolPreparationSnapshot) {
    // Stderr keeps CLI startup progress visible without contaminating stdout
    // protocol helpers. The supplier owns fixed, non-secret failure details.
    if let Ok(value) = serde_json::to_string(snapshot) {
        eprintln!("[runtime-preparation] {value}");
    }
}
