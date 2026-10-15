//! Deterministic gates hold real WK popup candidates, never synthetic pages.
//! Pause must close each exact native child while its host worker remains held.
use super::*;

#[cfg(debug_assertions)]
pub(super) async fn verify(runtime: &Arc<dyn BrowserRuntime>, fixture: &Fixture, id: &str, hold_ui: bool) -> Result<Value, String> {
    use nomifun_browser_macos::engine::{hold_popup_admission, PopupAdmissionStage};
    let mut evidence = Vec::new();
    for (name, stage) in [("queued", PopupAdmissionStage::Queued), ("claimed", PopupAdmissionStage::Claimed)] {
        let case = format!("{name}-{}", uuid::Uuid::now_v7());
        runtime.lock_user_input().await.map_err(error)?;
        navigate(runtime, id, &fixture.url(&format!("/popup-pause-parent?case={case}"))).await?;
        let parent = finished(runtime, id).await?;
        let parent_nonce = fixture.document_nonce("/popup-pause-parent", &case)?;
        assert_parent(runtime, fixture, id, &case, &parent_nonce).await?;
        let baseline = runtime.snapshot().await.map_err(error)?;
        let opener_id = uuid::Uuid::parse_str(id.strip_prefix("browser-").ok_or("Popup opener has no native page identity")?)
            .map_err(error)?;
        let hold = hold_popup_admission(opener_id, stage)?;
        trigger(runtime, id, hold_ui, name).await?;
        let candidate = tokio::time::timeout(Duration::from_secs(if hold_ui { 60 } else { 5 }), hold.wait_candidate()).await
            .map_err(|_| format!("No real WK {name} popup candidate arrived; trusted activation requires --hold-ui and the native link"))??;
        if candidate.opener_id != opener_id || candidate.opener_document_generation != parent.target.document_generation
            || candidate.opener_navigation_sequence != parent.load.as_ref().ok_or("Popup parent has no navigation sequence")?.navigation_sequence {
            return Err("Popup gate captured an inaccurate opener document or navigation attempt".into());
        }
        if *candidate.child.closed().borrow() { return Err("Popup child closed before the Pause race was exercised".into()); }
        // Its real main document finishes independently of host admission. This
        // makes the before/after HTTP count deterministic, including a child
        // that WK created while the host is held before tab registration.
        wait_child_finished(&candidate.child).await?;
        let child_nonce = fixture.document_nonce("/popup-pause-child", &case)?;
        let child_count = fixture.count_case("/popup-pause-child", &case);
        if child_count != 1 { return Err(format!("{name} popup issued {child_count} initial main-document requests")); }
        assert_tabs(runtime, &baseline).await?;

        runtime.release_pressed_input().await.map_err(error)?;
        runtime.unlock_user_input().await.map_err(error)?;
        fresh_pause(runtime, id, true).await?;
        // Do not await closure here: a successful Pause ACK already promises it.
        let closed_at_pause_ack = *candidate.child.closed().borrow();
        if !closed_at_pause_ack { return Err(format!("{name} Pause ACK preceded its exact native child close ACK")); }
        assert_tabs(runtime, &baseline).await?;
        runtime.lock_user_input().await.map_err(error)?;
        assert_parent(runtime, fixture, id, &case, &parent_nonce).await?;
        if fixture.count_case("/popup-pause-child", &case) != child_count {
            return Err("Pause admitted another revoked child's main-document request".into());
        }

        runtime.unlock_user_input().await.map_err(error)?;
        fresh_pause(runtime, id, false).await?;
        // Resume happens while the worker is still held. Releasing and waiting
        // for its exact settlement then proves an old candidate cannot revive.
        hold.release();
        let settlement = tokio::time::timeout(Duration::from_secs(10), hold.wait_settled()).await
            .map_err(|_| "Revoked popup host worker did not acknowledge settlement".to_string())??;
        if settlement.candidate_id != candidate.candidate_id || settlement.child_id != candidate.child.id()
            || settlement.admitted || !settlement.native_closed {
            return Err("Revoked popup host settlement was not the exact rejected and natively closed child".into());
        }
        assert_tabs(runtime, &baseline).await?;
        runtime.lock_user_input().await.map_err(error)?;
        assert_parent(runtime, fixture, id, &case, &parent_nonce).await?;
        if fixture.count_case("/popup-pause-child", &case) != child_count || !*candidate.child.closed().borrow() {
            return Err("Resume revived a revoked native child or its main document".into());
        }
        evidence.push(json!({"stage":name,"candidate_id":candidate.candidate_id,
            "opener_id":candidate.opener_id,"opener_document_generation":candidate.opener_document_generation,
            "opener_navigation_sequence":candidate.opener_navigation_sequence,"child_id":candidate.child.id(),
            "server_parent_document_nonce":parent_nonce,"server_child_document_nonce":child_nonce,
            "server_parent_main_document_count":1,"server_child_main_document_count_at_gate":child_count,
            "server_child_main_document_count_after_pause_and_resume":child_count,
            "native_child_closed_at_pause_ack":closed_at_pause_ack,"host_worker_settled":true,
            "active_tabs_unchanged":true,"resume_old_candidate_not_admitted":true,"physical_click":hold_ui}));

        if name == "claimed" {
            // New callback after Resume uses fresh admission rather than the
            // old revoked ticket. Keep the same loaded parent and its nonce.
            trigger(runtime, id, hold_ui, "fresh-after-resume").await?;
            let popup_id = wait_popup(runtime, id, Duration::from_secs(if hold_ui { 60 } else { 5 })).await?
                .ok_or("A fresh popup after Resume was not admitted")?;
            let popup = finished(runtime, &popup_id).await?;
            if fixture.count_case("/popup-pause-child", &case) != child_count + 1 {
                return Err("Fresh popup after Resume did not issue exactly one new main-document request".into());
            }
            runtime.lock_user_input().await.map_err(error)?;
            let observed = observe(runtime, &popup_id).await.map_err(error)?;
            if !observed.content.contains("Compatibility popup Pause child") {
                return Err("Fresh popup after Resume did not expose its actual child document".into());
            }
            command(runtime, BrowserTabCommand::Close { target: popup.target }).await?;
            command(runtime, BrowserTabCommand::Activate { target: tab(runtime, id).await?.target }).await?;
            assert_parent(runtime, fixture, id, &case, &parent_nonce).await?;
        }
    }
    Ok(json!({"checks_passed":true,"deterministic_stage_controls":true,"stages":evidence,
        "new_popup_after_resume_admitted":true,"old_candidates_never_revived":true}))
}

#[cfg(not(debug_assertions))]
pub(super) async fn verify(_runtime: &Arc<dyn BrowserRuntime>, _fixture: &Fixture, _id: &str, _hold_ui: bool) -> Result<Value, String> {
    Err("Deterministic popup acceptance requires the debug fixture build".into())
}

#[cfg(debug_assertions)]
async fn trigger(runtime: &Arc<dyn BrowserRuntime>, id: &str, hold_ui: bool, stage: &str) -> Result<(), String> {
    if hold_ui {
        runtime.release_pressed_input().await.map_err(error)?;
        runtime.unlock_user_input().await.map_err(error)?;
        eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_HOLD popup_pause={stage}: Click Open Pause race popup in the disposable native window");
    } else {
        runtime.lock_user_input().await.map_err(error)?;
        let observed = observe(runtime, id).await.map_err(error)?;
        let reference = observed.elements.iter().find(|element| element.name == "Open Pause race popup")
            .ok_or("Popup Pause fixture anchor is missing")?.reference.clone();
        runtime.automation().ok_or("Missing popup Pause automation port")?
            .act(BrowserAction::click(reference), CancellationToken::new()).await.map_err(error)?;
    }
    Ok(())
}

#[cfg(debug_assertions)]
async fn wait_child_finished(child: &Arc<nomifun_browser_macos::engine::Page>) -> Result<(), String> {
    let mut changed = child.subscribe();
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let current = changed.borrow_and_update().clone();
            if current.load.phase == BrowserNavigationPhase::Finished { return Ok::<_, String>(()); }
            if matches!(current.lifecycle, BrowserTabLifecycle::Failed | BrowserTabLifecycle::Crashed | BrowserTabLifecycle::Stopped) {
                return Err("Held popup's real native document failed before Pause".into());
            }
            changed.changed().await.map_err(error)?;
        }
    }).await.map_err(|_| "Held popup's real native document did not finish".to_string())?
}

#[cfg(debug_assertions)]
async fn assert_parent(runtime: &Arc<dyn BrowserRuntime>, fixture: &Fixture, id: &str, case: &str, nonce: &str) -> Result<(), String> {
    let observed = observe(runtime, id).await.map_err(error)?;
    if fixture.count_case("/popup-pause-parent", case) != 1 || !observed.content.contains(&format!("Document nonce: {nonce}")) {
        return Err("Popup Pause/Resume reloaded the parent document or changed its nonce".into());
    }
    Ok(())
}

#[cfg(debug_assertions)]
async fn assert_tabs(runtime: &Arc<dyn BrowserRuntime>, baseline: &BrowserRuntimeSnapshot) -> Result<(), String> {
    let current = runtime.snapshot().await.map_err(error)?;
    let ids = |snapshot: &BrowserRuntimeSnapshot| snapshot.tabs.iter().map(|tab| tab.target.tab_id.clone()).collect::<std::collections::BTreeSet<_>>();
    if current.active_tab_id != baseline.active_tab_id || ids(&current) != ids(baseline) {
        return Err("A queued or claimed popup changed the active workspace tabs".into());
    }
    Ok(())
}
