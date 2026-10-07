use nomifun_browser_macos::engine::Engine;
use nomifun_browser_platform::{
    run_guard::{BrowserInputState, BrowserRunCoordinator},
    runtime::{
        BrowserAction, BrowserEvaluation, BrowserEvaluationOutcome, BrowserProfile,
        BrowserResourceKey, BrowserRuntime, BrowserRuntimeFactory, BrowserTabCommand,
        BrowserTabLifecycle, BrowserTabTarget, CreateBrowserRuntime, WorkspaceError,
    },
};
use std::sync::Arc;
use tauri::Manager;
use tokio_util::sync::CancellationToken;

async fn create_runtime(
    engine: &Arc<Engine>,
    app: &tauri::AppHandle,
    base_url: &str,
    runtime_generation: u64,
    phase: &str,
) -> Result<(Arc<dyn BrowserRuntime>, BrowserTabTarget), String> {
    let host = super::macos::host::DesktopBrowserHost::new(app.clone(), engine.clone());
    let runtime = host
        .create(CreateBrowserRuntime {
            key: BrowserResourceKey {
                principal_id: "fixture".into(),
                agent_session_id: "native-cef-window-reopen".into(),
                resource_binding_id: "browser-fixture:native-cef-window-reopen".into(),
            },
            runtime_generation,
            profile: BrowserProfile::Ephemeral,
            user_input_enabled: true,
        })
        .await
        .map_err(|error| error.to_string())?;
    runtime
        .surface()
        .ok_or("Native CEF lifecycle surface is missing")?
        .set_surface(
            nomifun_browser_platform::runtime::BrowserSurfaceBounds {
                x: 20.,
                y: 60.,
                width: 1060.,
                height: 620.,
            },
            true,
            Default::default(),
        )
        .await
        .map_err(|error| error.to_string())?;
    let url = format!("{base_url}/browser_workspace.html?window={phase}");
    runtime
        .execute(
            BrowserTabCommand::Create { url: url.clone() },
            CancellationToken::new(),
        )
        .await
        .map_err(|error| error.to_string())?;
    let mut changes = runtime
        .changes()
        .ok_or("Native CEF lifecycle changes are unavailable")?;
    let target = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let snapshot = runtime.snapshot().await.map_err(|error| error.to_string())?;
            if let Some(tab) = snapshot.tabs.iter().find(|tab| {
                tab.url == url && tab.lifecycle == BrowserTabLifecycle::Ready
            }) {
                return Ok::<_, String>(tab.target.clone());
            }
            changes
                .changed()
                .await
                .map_err(|_| "Native CEF lifecycle subscription closed".to_owned())?;
        }
    })
    .await
    .map_err(|_| format!("Native CEF lifecycle {phase} navigation timed out"))??;
    Ok((runtime, target))
}

async fn close_main_window(app: &tauri::AppHandle) -> Result<usize, String> {
    let window = app
        .get_window("main")
        .ok_or("Native CEF lifecycle main window is missing")?;
    let identity = window
        .ns_window()
        .map_err(|_| "Native CEF lifecycle window identity is unavailable")?
        as usize;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(window.close().map_err(|error| error.to_string()));
    })
    .map_err(|_| "Native CEF lifecycle close dispatch failed")?;
    receiver
        .await
        .map_err(|_| "Native CEF lifecycle close receipt was lost")??;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while app.get_window("main").is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| "Native CEF lifecycle old window did not close")?;
    Ok(identity)
}

async fn reopen_main_window(app: &tauri::AppHandle) -> Result<usize, String> {
    let handle = app.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = tauri::window::WindowBuilder::new(&handle, "main")
            .title("NomiFun — macOS CEF reopened surface")
            .inner_size(1100.0, 720.0)
            .min_inner_size(880.0, 600.0)
            .build()
            .map_err(|error| error.to_string());
        let _ = sender.send(result);
    })
    .map_err(|_| "Native CEF lifecycle reopen dispatch failed")?;
    let window = receiver
        .await
        .map_err(|_| "Native CEF lifecycle reopen receipt was lost")??;
    window
        .ns_window()
        .map(|window| window as usize)
        .map_err(|_| "Reopened native CEF window identity is unavailable".into())
}

async fn wait_for_fixture(
    runtime: &Arc<dyn BrowserRuntime>,
    coordinator: &Arc<BrowserRunCoordinator>,
    run: &nomifun_browser_platform::run_guard::BrowserRunGuard,
    target: &BrowserTabTarget,
) -> Result<(), String> {
    for _ in 0..100 {
        let result = {
            let runtime = runtime.clone();
            let target = target.clone();
            coordinator
                .agent_operation(run, move |cancel| async move {
                    Ok(runtime
                        .automation()
                        .expect("CEF automation")
                        .evaluate(
                            BrowserEvaluation {
                                target,
                                expression: "document.readyState==='complete'&&document.querySelector('#field')!==null&&document.querySelector('#result')?.textContent==='等待 Agent'".into(),
                            },
                            cancel,
                        )
                        .await)
                })
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?
        };
        if matches!(
            result.outcome,
            BrowserEvaluationOutcome::Completed { value } if value == true
        ) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Err("Native CEF lifecycle fixture did not settle".into())
}

pub(super) async fn verify(
    engine: &Arc<Engine>,
    app: &tauri::AppHandle,
    base_url: &str,
) -> Result<serde_json::Value, String> {
    let (old_runtime, old_target) = create_runtime(engine, app, base_url, 51, "old").await?;
    let old_coordinator = BrowserRunCoordinator::new(old_runtime.clone());
    let old_run = old_coordinator
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    old_run.require_explicit_finish();
    wait_for_fixture(&old_runtime, &old_coordinator, &old_run, &old_target).await?;
    let old_observation = {
        let runtime = old_runtime.clone();
        let tab_id = old_target.tab_id.clone();
        old_coordinator
            .agent_operation(&old_run, move |cancel| async move {
                Ok(runtime
                    .automation()
                    .expect("CEF automation")
                    .observe(Some(tab_id), cancel)
                    .await)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
    };
    let old_reference = old_observation
        .elements
        .iter()
        .find(|element| element.name == "验证点击" && element.role == "button")
        .ok_or("Native CEF lifecycle old reference is missing")?
        .reference
        .clone();
    let dialog = {
        let runtime = old_runtime.clone();
        let target = old_target.clone();
        old_coordinator
            .agent_operation(&old_run, move |cancel| async move {
                Ok(runtime
                    .automation()
                    .expect("CEF automation")
                    .evaluate(
                        BrowserEvaluation {
                            target,
                            expression: "confirm('window close pending work')".into(),
                        },
                        cancel,
                    )
                    .await)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
    };
    if !matches!(dialog.outcome, BrowserEvaluationOutcome::AwaitingDialog { .. }) {
        return Err(format!("Native CEF lifecycle did not retain pending dialog work: {dialog:?}"));
    }

    let old_window_identity = close_main_window(app).await?;
    old_run.cancel();
    old_coordinator
        .finish(&old_run)
        .await
        .map_err(|error| error.to_string())?;
    let old_gate = old_coordinator.snapshot().await;
    old_runtime.close().await.map_err(|error| error.to_string())?;
    if old_gate.input_state != BrowserInputState::UserReady || old_gate.input_gate_failed {
        return Err("Native CEF lifecycle pending work did not settle before old runtime close".into());
    }
    if !matches!(old_runtime.snapshot().await, Err(WorkspaceError::WorkspaceClosed)) {
        return Err("Native CEF lifecycle old runtime remained readable after close".into());
    }
    let old_action = old_runtime
        .automation()
        .expect("CEF automation")
        .act(BrowserAction::click(old_reference), CancellationToken::new())
        .await;
    if !matches!(old_action, Err(WorkspaceError::WorkspaceClosed)) {
        return Err(format!("Native CEF lifecycle old reference was not fenced: {old_action:?}"));
    }

    let new_window_identity = reopen_main_window(app).await?;
    if old_window_identity == new_window_identity {
        return Err("Native CEF lifecycle reused the closed NSWindow identity".into());
    }
    let (new_runtime, new_target) = create_runtime(engine, app, base_url, 52, "new").await?;
    if new_target.tab_id == old_target.tab_id
        || new_target.runtime_generation == old_target.runtime_generation
    {
        return Err("Native CEF lifecycle reused an old surface identity".into());
    }
    let old_target_result = new_runtime
        .execute(
            BrowserTabCommand::Reload {
                target: old_target.clone(),
            },
            CancellationToken::new(),
        )
        .await;
    if !matches!(old_target_result, Err(WorkspaceError::TabNotFound)) {
        return Err(format!("Native CEF lifecycle admitted the old target in the new runtime: {old_target_result:?}"));
    }

    let new_coordinator = BrowserRunCoordinator::new(new_runtime.clone());
    let new_run = new_coordinator
        .begin()
        .await
        .map_err(|error| error.to_string())?;
    new_run.require_explicit_finish();
    wait_for_fixture(&new_runtime, &new_coordinator, &new_run, &new_target).await?;
    let observed = {
        let runtime = new_runtime.clone();
        let tab_id = new_target.tab_id.clone();
        new_coordinator
            .agent_operation(&new_run, move |cancel| async move {
                Ok(runtime
                    .automation()
                    .expect("CEF automation")
                    .observe(Some(tab_id), cancel)
                    .await)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
    };
    let field = observed
        .elements
        .iter()
        .find(|element| element.name == "输入内容" && element.role == "textbox")
        .ok_or("Reopened native CEF textbox is missing")?
        .reference
        .clone();
    {
        let runtime = new_runtime.clone();
        new_coordinator
            .agent_operation(&new_run, move |cancel| async move {
                Ok(runtime
                    .automation()
                    .expect("CEF automation")
                    .act(
                        BrowserAction::Type {
                            element: field,
                            text: "reopened-中文".into(),
                        },
                        cancel,
                    )
                    .await)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
    }
    let observed = {
        let runtime = new_runtime.clone();
        let tab_id = new_target.tab_id.clone();
        new_coordinator
            .agent_operation(&new_run, move |cancel| async move {
                Ok(runtime
                    .automation()
                    .expect("CEF automation")
                    .observe(Some(tab_id), cancel)
                    .await)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
    };
    let button = observed
        .elements
        .iter()
        .find(|element| element.name == "验证点击" && element.role == "button")
        .ok_or("Reopened native CEF click target is missing")?
        .reference
        .clone();
    {
        let runtime = new_runtime.clone();
        new_coordinator
            .agent_operation(&new_run, move |cancel| async move {
                Ok(runtime
                    .automation()
                    .expect("CEF automation")
                    .act(BrowserAction::click(button), cancel)
                    .await)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
    }
    let proof = {
        let runtime = new_runtime.clone();
        let target = new_target.clone();
        new_coordinator
            .agent_operation(&new_run, move |cancel| async move {
                Ok(runtime
                    .automation()
                    .expect("CEF automation")
                    .evaluate(
                        BrowserEvaluation {
                            target,
                            expression: "(()=>{const field=document.querySelector('#field');const result=document.querySelector('#result');return {value:field?.value,clicks:Number(result?.dataset.clicks||0),trusted:result?.dataset.lastClickTrusted==='true'}})()".into(),
                        },
                        cancel,
                    )
                    .await)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?
    };
    let proof = match proof.outcome {
        BrowserEvaluationOutcome::Completed { value } => value,
        outcome => return Err(format!("Reopened native CEF proof did not complete: {outcome:?}")),
    };
    if proof["value"] != "reopened-中文" || proof["clicks"] != 1 || proof["trusted"] != true {
        return Err(format!("Reopened native CEF input proof failed: {proof}"));
    }
    new_coordinator
        .finish(&new_run)
        .await
        .map_err(|error| error.to_string())?;
    new_runtime.close().await.map_err(|error| error.to_string())?;
    eprintln!("CEF_REOPEN_PHASE old_window_closed");
    eprintln!("CEF_REOPEN_PHASE pending_work_drained");
    eprintln!("CEF_REOPEN_PHASE new_surface_verified");
    let checks = serde_json::json!({
        "old_tauri_window_closed": true,
        "pending_dialog_work_drained": true,
        "old_runtime_and_reference_fenced": true,
        "new_nswindow_identity_unique": true,
        "new_runtime_and_tab_identity_unique": true,
        "old_target_rejected_by_new_runtime": true,
        "reopened_surface_unicode_and_trusted_click": true,
    });
    Ok(serde_json::json!({
        "scope": "native-cef-window-reopen",
        "checks": checks,
        "metrics": {
            "old_runtime_generation": old_target.runtime_generation,
            "new_runtime_generation": new_target.runtime_generation,
            "old_document_generation": old_target.document_generation,
            "new_document_generation": new_target.document_generation,
        },
        "passed": checks.as_object().is_some_and(|values| values.values().all(|value| value == true)),
    }))
}
