//! Native history controls over an actual same-document SPA and full documents.
//! HTTP receipts and server-issued nonces establish which document is retained.
use super::*;

pub(super) async fn verify(runtime: &Arc<dyn BrowserRuntime>, fixture: &Fixture, id: &str) -> Result<Value, String> {
    runtime.lock_user_input().await.map_err(error)?;
    let case = uuid::Uuid::now_v7().to_string();
    let url = fixture.url(&format!("/spa-history?case={case}"));
    navigate(runtime, id, &url).await?;
    let initial = settled_url(runtime, id, &url).await?;
    let nonce = fixture.document_nonce("/spa-history", &case)?;
    let initial_count = fixture.count_case("/spa-history", &case);
    assert_document(runtime, id, &nonce, "root").await?;
    click_named(runtime, id, "Push SPA first").await?;
    settled_url(runtime, id, &format!("{url}&step=first")).await?;
    click_named(runtime, id, "Push SPA second").await?;
    let before_back = settled_url(runtime, id, &format!("{url}&step=second")).await?;
    let captured_sequence = sequence(&before_back)?;
    let before_trace = trace_sequence(runtime, id).await?;

    command(runtime, BrowserTabCommand::Back { target: before_back.target }).await?;
    let back = settled_url(runtime, id, &format!("{url}&step=first")).await?;
    assert_document(runtime, id, &nonce, "first").await?;
    if sequence(&back)? <= captured_sequence {
        return Err("SPA Back did not settle a fresh native history control sequence".into());
    }
    let after_back = report(runtime, &back.target).await?;
    command(runtime, BrowserTabCommand::Forward { target: back.target.clone() }).await?;
    let forward = settled_url(runtime, id, &format!("{url}&step=second")).await?;
    assert_document(runtime, id, &nonce, "second").await?;
    if sequence(&forward)? <= sequence(&back)? {
        return Err("SPA Forward did not settle a fresh native history control sequence".into());
    }
    if fixture.count_case("/spa-history", &case) != initial_count {
        return Err("SPA Back/Forward re-requested its main document".into());
    }
    let same_document = report(runtime, &forward.target).await?;
    if same_document.trace.events.iter().any(|event| event.sequence > before_trace
        && matches!(event.kind, BrowserNavigationEventKind::Started | BrowserNavigationEventKind::Committed)) {
        return Err("SPA history fabricated full-document start or commit callbacks".into());
    }

    // Keep the exact current target, but use the sequence captured before Back.
    // This isolates the sequence fence from document-generation rejection.
    let stable = forward.clone();
    for stale in [
        BrowserTabCommand::StopLoading { target: stable.target.clone(), expected_navigation_sequence: captured_sequence },
        BrowserTabCommand::Reload { target: stable.target.clone(), expected_navigation_sequence: captured_sequence },
    ] {
        if !matches!(runtime.execute(stale, CancellationToken::new()).await, Err(WorkspaceError::StaleTarget | WorkspaceError::StaleObservation)) {
            return Err("A stale navigation control mutated the current SPA document".into());
        }
        let current = tab(runtime, id).await?;
        if current.target != stable.target || current.load != stable.load || current.url != stable.url
            || fixture.count_case("/spa-history", &case) != initial_count {
            return Err("A rejected old navigation sequence changed the live document".into());
        }
        assert_document(runtime, id, &nonce, "second").await?;
    }

    // The same controls must still run normal lifecycle for a real document.
    let full_url = fixture.url("/redirect-final");
    let cross_start = trace_sequence(runtime, id).await?;
    navigate(runtime, id, &full_url).await?;
    let full = settled_url(runtime, id, &full_url).await?;
    let cross_report = report(runtime, &full.target).await?;
    if !cross_report.trace.events.iter().any(|event| event.sequence > cross_start && event.kind == BrowserNavigationEventKind::Started)
        || !cross_report.trace.events.iter().any(|event| event.sequence > cross_start && event.kind == BrowserNavigationEventKind::Committed) {
        return Err("Full-document navigation lost its native lifecycle after SPA controls".into());
    }
    command(runtime, BrowserTabCommand::Back { target: full.target }).await?;
    settled_url(runtime, id, &format!("{url}&step=second")).await?;
    let back_from_full = tab(runtime, id).await?;
    command(runtime, BrowserTabCommand::Forward { target: back_from_full.target }).await?;
    let final_tab = settled_url(runtime, id, &full_url).await?;
    let final_observation = observe(runtime, id).await.map_err(error)?;
    if !final_observation.content.contains("Compatibility redirects settled") {
        return Err("Full-document Forward did not restore its actual document".into());
    }
    Ok(json!({
        "same_document_back_forward_finished_ready_100":true,"same_document_url_change":"query_step_preserves_case",
        "server_document_nonce":nonce,"server_main_document_count_before":initial_count,
        "server_main_document_count_after_same_document_controls":initial_count,
        "same_document_no_started_or_committed_callbacks":true,
        "stale_stop_and_reload_rejected_with_current_exact_target":true,
        "initial_document_generation":initial.target.document_generation,
        "before_back_sequence":captured_sequence,"back_sequence":sequence(&back)?,"forward_sequence":sequence(&forward)?,
        "back_report":after_back,"forward_report":same_document,
        "full_document_navigation_and_history_still_finish":true,"final_report":report(runtime, &final_tab.target).await?,
    }))
}

fn sequence(tab: &BrowserTabSnapshot) -> Result<u64, String> {
    tab.load.as_ref().map(|load| load.navigation_sequence).ok_or_else(|| "History target has no captured navigation sequence".into())
}

async fn settled_url(runtime: &Arc<dyn BrowserRuntime>, id: &str, url: &str) -> Result<BrowserTabSnapshot, String> {
    let current = wait_tab(runtime, id, |tab| tab.url == url && tab.lifecycle == BrowserTabLifecycle::Ready
        && tab.load.as_ref().is_some_and(|load| load.phase == BrowserNavigationPhase::Finished
            && load.estimated_progress == Some(100))).await?;
    Ok(current)
}

async fn click_named(runtime: &Arc<dyn BrowserRuntime>, id: &str, name: &str) -> Result<(), String> {
    let observed = observe(runtime, id).await.map_err(error)?;
    let reference = observed.elements.iter().find(|element| element.name == name)
        .ok_or_else(|| format!("Missing SPA fixture action {name}"))?.reference.clone();
    runtime.automation().ok_or("Missing history automation port")?
        .act(BrowserAction::click(reference), CancellationToken::new()).await.map_err(error)?;
    Ok(())
}

async fn assert_document(runtime: &Arc<dyn BrowserRuntime>, id: &str, nonce: &str, step: &str) -> Result<(), String> {
    let observed = observe(runtime, id).await.map_err(error)?;
    if !observed.content.contains(&format!("Document nonce: {nonce}")) || !observed.content.contains(&format!("SPA step: {step}")) {
        return Err("SPA history changed the real document nonce or lost its history state".into());
    }
    Ok(())
}
