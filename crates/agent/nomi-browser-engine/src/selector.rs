//! Stable selector generation: wraps the vendored Playwright `selectorGenerator`
//! (exposed as `InjectedScript.prototype.generateSelectorSimple`) through the
//! existing injected-call seam.
//!
//! The selector returned is a stable, human-readable locator string (CSS / role /
//! text / data-testid based) suitable for recording and replaying browser actions.
//! It does NOT hand-edit the vendored bundle — it calls the already-bundled export
//! via `call_injected`.

use chromiumoxide::cdp::js_protocol::runtime::{CallArgument, RemoteObjectId};

use crate::injected::{InjectError, InjectionManager};

impl InjectionManager {
    /// Generate a stable selector for the element identified by `element_object_id`
    /// (an objectId in the same utility world as the InjectedScript instance).
    ///
    /// Calls the vendored `InjectedScript.prototype.generateSelectorSimple(element)`
    /// which returns a string like `"internal:role=button[name=\"Submit\"]"` or
    /// `"#email"` or `"[data-testid=\"login-btn\"]"` — Playwright's best stable
    /// locator for the element.
    ///
    /// Returns `Ok(selector_string)` on success, or an [`InjectError`] if the call
    /// fails (element detached, world not ready, etc.). **Never panics.**
    pub async fn generate_selector(
        &self,
        frame_id: &str,
        element_object_id: &str,
    ) -> Result<String, InjectError> {
        let element_arg = CallArgument {
            object_id: Some(RemoteObjectId::new(element_object_id.to_string())),
            ..Default::default()
        };

        // generateSelectorSimple is a method on the InjectedScript instance.
        // It takes (targetElement, options?) and returns a selector string.
        // We call it with return_by_value=true to get the string directly.
        let result = self
            .call_injected(
                frame_id,
                "generateSelectorSimple",
                vec![element_arg],
                true, // return_by_value
            )
            .await?;

        // result is the RemoteObject; with return_by_value=true, .value is the string.
        result
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                InjectError::Protocol(format!(
                    "generateSelectorSimple returned no string value: {result}"
                ))
            })
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Real-Chrome test (requires NOMIFUN_CHROME_BINARY)
// ═══════════════════════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {

    /// **Real-Chrome #[ignore] test**: generate a selector for a known element and
    /// verify it's non-empty and re-resolves to the same node.
    ///
    /// Run: `NOMIFUN_CHROME_BINARY="..." cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(generate_selector_returns_stable_selector)'`
    #[tokio::test]
    #[ignore = "需 NOMIFUN_CHROME_BINARY（真 Chrome）：selectorGenerator 冒烟"]
    async fn generate_selector_returns_stable_selector() {
        use crate::injected::InjectionManager;
        use crate::launch::{launch_chrome, LaunchConfig};
        use crate::transport::{Connection, ROOT_SESSION};
        use chromiumoxide::cdp::browser_protocol::page::{
            EnableParams as PageEnable, NavigateParams,
        };
        use chromiumoxide::cdp::browser_protocol::target::{
            CreateTargetParams, EventAttachedToTarget,
        };
        use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};
        use std::time::Duration;

        // 1) Launch headless Chrome.
        let chrome = crate::acquire::resolve_chrome_path(
            &std::env::temp_dir().join("nomifun-selector-test-data"),
            None,
        )
        .await
        .expect("resolve chrome (set NOMIFUN_CHROME_BINARY)");
        let cfg = LaunchConfig {
            chrome_path: chrome,
            user_data_dir: std::env::temp_dir().join("nomifun-selector-test-profile"),
            headful: false,
        };
        let launched = launch_chrome(&cfg, true).await.expect("launch chrome");
        let _child = launched.child;
        let conn = Connection::connect_launched(launched.transport)
            .await
            .expect("connect");
        let _attach_loop = conn.run_attach_loop();
        conn.enable_auto_attach().await.expect("auto attach");

        // 2) Create a page with a fixture form.
        let mut attached = conn.subscribe(EventAttachedToTarget::IDENTIFIER, None);
        let create = CreateTargetParams::new("about:blank");
        let cr = conn
            .send::<CreateTargetParams>(ROOT_SESSION, &create)
            .await
            .expect("createTarget");
        let target_id = cr["targetId"].as_str().expect("targetId").to_string();
        let page_session = loop {
            let ev = tokio::time::timeout(Duration::from_secs(10), attached.recv())
                .await
                .expect("attach timeout")
                .expect("attach recv");
            if let Ok(att) = serde_json::from_value::<EventAttachedToTarget>(ev.params.clone()) {
                let tid: String = att.target_info.target_id.clone().into();
                if tid == target_id && att.target_info.r#type == "page" {
                    break String::from(att.session_id);
                }
            }
        };

        // Navigate to a fixture page (base64-encode to avoid # / quote issues in data: URL).
        let html = r#"<body><button id="submit-btn" data-testid="submit">Submit order</button><input id="email" type="text" name="email"><input id="pw" type="password" name="password"></body>"#;
        let html_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            html.as_bytes(),
        );
        let data_url = format!("data:text/html;base64,{html_b64}");

        conn.send::<PageEnable>(&page_session, &PageEnable::default())
            .await
            .expect("Page.enable");
        let mut load_rx = conn.subscribe("Page.loadEventFired", Some(&page_session));
        conn.send::<NavigateParams>(&page_session, &NavigateParams::new(&data_url))
            .await
            .expect("navigate");
        let _ = tokio::time::timeout(Duration::from_secs(15), load_rx.recv()).await;

        // 3) Arm injection and wait for context.
        let mgr = InjectionManager::new(conn.clone(), page_session.clone());
        let _ctx_loop = mgr.arm().await.expect("arm injection");

        let frame_id = target_id.clone();
        let mut ready = false;
        for _ in 0..50 {
            if mgr.context_id_for(&frame_id).is_ok() {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ready, "utility world context never registered");

        // 4) Get the button element's objectId in the utility world.
        let ctx_id = mgr.context_id_for(&frame_id).expect("ctx id");
        let mut eval = EvaluateParams::new(
            r#"document.querySelector('#submit-btn')"#.to_string(),
        );
        eval.context_id = Some(ExecutionContextId::new(ctx_id));
        eval.return_by_value = Some(false);
        let result = conn
            .send::<EvaluateParams>(&page_session, &eval)
            .await
            .expect("evaluate querySelector");
        let btn_obj_id = result["result"]["objectId"]
            .as_str()
            .expect("button objectId")
            .to_string();

        // 5) Generate a selector for the button.
        let selector = mgr
            .generate_selector(&frame_id, &btn_obj_id)
            .await
            .expect("generate_selector");

        eprintln!("Generated selector: {selector}");
        assert!(
            !selector.is_empty(),
            "selector must be non-empty"
        );

        // 6) Verify the selector re-resolves to the same node by checking the
        //    element has the expected id attribute. Use parseSelector + querySelector
        //    (Playwright's internal selector engine) since the selector may be in
        //    Playwright's internal format (e.g. "internal:role=button[name=...]").
        let result = mgr
            .call_on_injected_handle(
                &frame_id,
                &format!("function() {{ const sel = this.parseSelector({selector_json}); const el = this.querySelector(sel, document, false); return el ? el.id : null; }}",
                    selector_json = serde_json::to_string(&selector).unwrap()),
                vec![],
                None,
                true,
            )
            .await
            .expect("verify selector resolves");

        let resolved_id = result.get("value").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(
            resolved_id, "submit-btn",
            "selector must re-resolve to the same element; selector={selector}, resolved_id={resolved_id}"
        );
    }
}
