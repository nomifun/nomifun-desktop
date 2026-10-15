//! Real site outcomes are separate from deterministic fixture assertions.
//! Manual mode leaves native input unlocked during idle/interaction phases.
//! No login, challenge solving, automatic retries, or page bodies in reports.
use nomifun_browser_platform::{runtime::*, url_projection::project_metadata_url};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;
use super::outcome::{Evidence, classify};

const SITES: [(&str, &str); 3] = [
    ("baidu_homepage", "https://www.baidu.com/"),
    ("baidu_search", "https://www.baidu.com/s?wd=nomifun"),
    ("bilibili", "https://www.bilibili.com/"),
];

async fn tab(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserTabSnapshot, String> {
    runtime.snapshot().await.map_err(|error| error.to_string())?.tabs.into_iter()
        .find(|tab| tab.target.tab_id == id).ok_or_else(|| "Live target disappeared".into())
}

async fn observe(runtime: &Arc<dyn BrowserRuntime>, id: &str) -> Result<BrowserObservation, String> {
    let cancel = CancellationToken::new();
    let result = tokio::time::timeout(Duration::from_secs(3), runtime.automation()
        .ok_or("Live runtime has no observation port")?.observe(Some(id.into()), cancel.clone())).await;
    match result {
        Ok(result) => result.map_err(|error| error.code().into()),
        Err(_) => { cancel.cancel(); Err("OBSERVATION_TIMEOUT".into()) }
    }
}

pub(crate) async fn verify(runtime: &Arc<dyn BrowserRuntime>, manual: bool, evidence_dir: Option<&Path>) -> Result<Value, String> {
    let result = verify_inner(runtime, manual, evidence_dir).await;
    let cleanup = async {
        runtime.release_pressed_input().await.map_err(|error| error.to_string())?;
        runtime.unlock_user_input().await.map_err(|error| error.to_string())
    }.await;
    match (result, cleanup) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("Live input cleanup failed: {error}")),
        (Err(error), Err(cleanup)) => Err(format!("{error}; input cleanup: {cleanup}")),
    }
}

async fn verify_inner(runtime: &Arc<dyn BrowserRuntime>, manual: bool, evidence_dir: Option<&Path>) -> Result<Value, String> {
    // One bounded console reader for all manual sites; no user-entered URL,
    // search text, credentials or stdin contents are copied into evidence.
    let mut console = manual.then(console_reader);
    let mut reports = Vec::new();
    for (name, url) in SITES {
        let existing_tabs: std::collections::BTreeSet<_> = runtime.snapshot().await.map_err(|error| error.to_string())?
            .tabs.into_iter().map(|tab| tab.target.tab_id).collect();
        runtime.release_pressed_input().await.map_err(|error| error.to_string())?;
        runtime.unlock_user_input().await.map_err(|error| error.to_string())?;
        let snapshot = runtime.execute(BrowserTabCommand::Create { url: url.into() }, CancellationToken::new())
            .await.map_err(|error| error.to_string())?;
        let id = snapshot.active_tab_id.ok_or("Live entry created no target")?;
        runtime.surface().ok_or("Live runtime has no surface")?.set_surface(
            BrowserSurfaceBounds { x:20.0,y:40.0,width:840.0,height:540.0 }, true, CancellationToken::new())
            .await.map_err(|error| error.to_string())?;
        eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE {name}_idle_60_seconds input_unlocked");
        let started = Instant::now();
        let deadline = started + Duration::from_secs(60);
        let mut samples = Vec::new();
        let mut grace_counts = None;
        loop {
            let current = tab(runtime, &id).await?;
            let diagnostic = runtime.navigation_diagnostics(current.target.clone()).await.ok();
            if started.elapsed() >= Duration::from_secs(10) && grace_counts.is_none() {
                grace_counts = diagnostic.as_ref().map(|report| report.trace.counters.clone());
            }
            samples.push(json!({"elapsed_ms":started.elapsed().as_millis(),"lifecycle":current.lifecycle,
                "load":current.load.as_ref().map(BrowserLoadSummary::agent_projection),
                "native_counters":diagnostic.as_ref().map(|report| &report.trace.counters),
                "routing_lost":diagnostic.as_ref().map(|report| report.trace.routing_lost)}));
            if Instant::now() >= deadline { break; }
            tokio::time::sleep(Duration::from_secs(5).min(deadline.saturating_duration_since(Instant::now()))).await;
        }
        // Typed Agent observation requires the existing native input lock. Read
        // once after the manual idle phase rather than locking every sample.
        runtime.lock_user_input().await.map_err(|error| error.to_string())?;
        let observed = observe(runtime, &id).await;
        let current = tab(runtime, &id).await?;
        let diagnostic = runtime.navigation_diagnostics(current.target.clone()).await.ok();
        let observation = observed.as_ref().ok().filter(|observation| observation.target == current.target);
        let text = observation.map_or("", |observation| observation.content.as_str());
        let meaningful = observation.is_some_and(|observation| observation.elements.len() > 1
            || observation.content.lines().skip(1).any(|line| !line.trim().is_empty()));
        let counts = diagnostic.as_ref().map(|report| &report.trace.counters);
        let late_page = counts.zip(grace_counts.as_ref()).map_or(0, |(end, start)|
            end.page_navigations.saturating_sub(start.page_navigations));
        let late_unknown = counts.zip(grace_counts.as_ref()).map_or(0, |(end, start)|
            end.unknown_navigations.saturating_sub(start.unknown_navigations));
        let host_commands = counts.map(|counts| counts.user_commands + counts.agent_commands);
        let (outcome, mut entry_passed) = classify(&Evidence {
            text, url: &current.url, site: name,
            native_failed: matches!(current.lifecycle, BrowserTabLifecycle::Failed | BrowserTabLifecycle::Crashed | BrowserTabLifecycle::Stopped),
            current_document_finished: current.load.as_ref().is_some_and(|load|
                load.phase == BrowserNavigationPhase::Finished && load.content_state == BrowserContentState::CurrentDocument),
            meaningful_document: meaningful,
            loop_suspected: current.load.as_ref().and_then(|load| load.navigation_control.as_ref()).is_some_and(|control| control.loop_suspected),
            late_page_navigations: late_page, late_unknown_navigations: late_unknown,
            routing_lost: grace_counts.is_none() || diagnostic.as_ref().is_none_or(|report| report.trace.routing_lost),
            host_commands,
        });
        let screenshot = screenshot(runtime, &id, name, evidence_dir).await;
        let screenshot_same_target = screenshot["target"] == serde_json::to_value(&current.target)
            .map_err(|_| "Target evidence serialization failed")?;
        if screenshot["saved"] == true && !screenshot_same_target { entry_passed = false; }
        let idle_seconds = started.elapsed().as_secs();
        runtime.release_pressed_input().await.map_err(|error| error.to_string())?;
        runtime.unlock_user_input().await.map_err(|error| error.to_string())?;
        let mut interaction = if let Some(console) = &mut console {
            let mut reader = console.lock().await;
            manual_interactions(runtime, &id, name, &mut reader).await?
        } else { json!({"required":false,"completed":false,"scope":"idle_entry_only"}) };
        let active = runtime.snapshot().await.map_err(|error| error.to_string())?.active_tab_id.unwrap_or_else(|| id.clone());
        let final_id = if manual { active } else { id.clone() };
        if manual {
            let final_evidence = manual_final_evidence(runtime, &final_id, name, evidence_dir).await?;
            let passed = interaction["completed"] == true && final_evidence["normal_current_document"] == true;
            interaction["completed"] = json!(passed);
            interaction["final_document"] = final_evidence;
            runtime.release_pressed_input().await.map_err(|error| error.to_string())?;
            runtime.unlock_user_input().await.map_err(|error| error.to_string())?;
        }
        let final_tab = tab(runtime, &final_id).await?;
        let final_report = runtime.navigation_diagnostics(final_tab.target.clone()).await.ok();
        reports.push(json!({"site":name,"outcome":outcome,"entry_check_passed":entry_passed,
            "idle_observed_seconds":idle_seconds,"idle_final_url":project_metadata_url(&current.url),
            "idle_final_target":current.target,"idle_final_lifecycle":current.lifecycle,
            "last_read_error":observed.err(),"host_command_count":host_commands,
            "page_navigation_count":counts.map(|counts| counts.page_navigations),
            "redirect_count":counts.map(|counts| counts.redirects),
            "late_page_navigations_after_10_seconds":late_page,"late_unknown_navigations":late_unknown,
            "idle_navigation_report":diagnostic,"samples":samples,"screenshot":screenshot,
            "screenshot_matches_idle_target":screenshot_same_target,
            "manual_interactions":interaction,"final_navigation_report":final_report,
            "final_url":project_metadata_url(&final_tab.url)}));
        // A physical link can create a popup. Close every tab created by this
        // site phase, preserving all pre-existing conformance fixture pages.
        for owned in runtime.snapshot().await.map_err(|error| error.to_string())?.tabs {
            if !existing_tabs.contains(&owned.target.tab_id) {
                runtime.execute(BrowserTabCommand::Close { target: owned.target }, CancellationToken::new())
                    .await.map_err(|error| error.to_string())?;
            }
        }
    }
    let passed = reports.len() == SITES.len() && reports.iter().all(|report|
        report["entry_check_passed"] == true && (!manual || report["manual_interactions"]["completed"] == true));
    let native_acceptance = passed && manual && reports.iter().all(|report| report["screenshot"]["saved"] == true
        && report["screenshot_matches_idle_target"] == true && report["manual_interactions"]["final_document"]["screenshot"]["saved"] == true);
    Ok(json!({"entry_checks_passed":passed,"sites":reports,"manual_mode":manual,
        "native_site_acceptance_completed":native_acceptance,
        "observation_scope":"typed_observe_after_each_idle_phase_no_raw_dom_retained",
        "login":"not_covered","actual_video_playback":"not_covered","captcha":"observed_only_never_passed_or_solved",
        "real_production_address_bar":"requires_separate_desktop_UI_acceptance"}))
}

async fn manual_final_evidence(runtime: &Arc<dyn BrowserRuntime>, id: &str, name: &str,
    evidence_dir: Option<&Path>) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let current = tab(runtime, id).await?;
        if current.load.as_ref().is_none_or(|load| !matches!(load.phase,
            BrowserNavigationPhase::Requested | BrowserNavigationPhase::Provisional | BrowserNavigationPhase::Committed))
            || Instant::now() >= deadline { break; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let before = runtime.navigation_diagnostics(tab(runtime, id).await?.target).await.ok();
    // Observe final manual stability, rather than reusing the pre-click page.
    tokio::time::sleep(Duration::from_secs(3)).await;
    runtime.lock_user_input().await.map_err(|error| error.to_string())?;
    let observation = observe(runtime, id).await;
    let current = tab(runtime, id).await?;
    let report = runtime.navigation_diagnostics(current.target.clone()).await.ok();
    let current_observation = observation.as_ref().ok().filter(|observation| observation.target == current.target);
    let text = current_observation.map_or("", |observation| observation.content.as_str());
    let late_page = report.as_ref().zip(before.as_ref()).map_or(0, |(end, start)|
        end.trace.counters.page_navigations.saturating_sub(start.trace.counters.page_navigations));
    let late_unknown = report.as_ref().zip(before.as_ref()).map_or(0, |(end, start)|
        end.trace.counters.unknown_navigations.saturating_sub(start.trace.counters.unknown_navigations));
    let (outcome, normal) = classify(&Evidence { text, url:&current.url,site:"manual_result",
        native_failed:matches!(current.lifecycle,BrowserTabLifecycle::Failed | BrowserTabLifecycle::Crashed | BrowserTabLifecycle::Stopped),
        current_document_finished:current.load.as_ref().is_some_and(|load| load.phase == BrowserNavigationPhase::Finished
            && load.content_state == BrowserContentState::CurrentDocument),
        meaningful_document:current_observation.is_some_and(|observation| observation.elements.len() > 1
            || observation.content.lines().skip(1).any(|line| !line.trim().is_empty())),
        loop_suspected:current.load.as_ref().and_then(|load| load.navigation_control.as_ref()).is_some_and(|control| control.loop_suspected),
        late_page_navigations:late_page,late_unknown_navigations:late_unknown,
        routing_lost:before.is_none() || report.as_ref().is_none_or(|report| report.trace.routing_lost),
        host_commands:report.as_ref().map(|report| report.trace.counters.user_commands + report.trace.counters.agent_commands),
    });
    let capture = screenshot(runtime,id,&format!("{name}-manual"),evidence_dir).await;
    let same_target = capture["target"] == serde_json::to_value(&current.target).map_err(|_| "Target evidence serialization failed")?;
    Ok(json!({"outcome":outcome,"normal_current_document":normal && (!capture["saved"].as_bool().unwrap_or(false) || same_target),
        "url":project_metadata_url(&current.url),"target":current.target,"last_read_error":observation.err(),
        "stability_observed_seconds":3,"page_navigation_delta":late_page,"unknown_navigation_delta":late_unknown,
        "screenshot":capture,"navigation_report":report}))
}

fn console_reader() -> Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>> {
    use std::io::BufRead;
    static CONSOLE: std::sync::OnceLock<Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>>> = std::sync::OnceLock::new();
    CONSOLE.get_or_init(|| {
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break; };
                // Only recognized control words are retained. Drop arbitrary input.
                let word = line.trim().to_ascii_lowercase();
                if ["typed", "clicked", "entry", "back", "reload", "hide", "show", "done"].contains(&word.as_str())
                    && sender.blocking_send(word).is_err() { break; }
            }
        });
        Arc::new(tokio::sync::Mutex::new(receiver))
    }).clone()
}

async fn manual_interactions(runtime: &Arc<dyn BrowserRuntime>, id: &str, name: &str,
    console: &mut tokio::sync::mpsc::Receiver<String>) -> Result<Value, String> {
    // Drain commands typed during the preceding idle phase; they cannot prove
    // interactions with this site's newly opened document.
    while console.try_recv().is_ok() {}
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_HOLD {name}: type a search in the native page then enter 'typed'; physically click a result/link then enter 'clicked'. Console commands target the active native tab: entry (this site's entry URL), back, reload, hide, show, done. If a new popup has no Back history, use entry then back. Do not log in or solve challenges. Timeout: 180 seconds.");
    let initial = tab(runtime, id).await?;
    let before = runtime.navigation_diagnostics(initial.target.clone()).await.ok();
    let baseline = before.as_ref().and_then(|report| report.trace.events.last()).map_or(0, |event| event.sequence);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(180);
    let mut actions = Vec::new();
    let mut marked = std::collections::BTreeSet::new();
    let mut completed = false;
    while Instant::now() < deadline {
        let word = match tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), console.recv()).await {
            Ok(Some(word)) => word,
            _ => break,
        };
        let active = runtime.snapshot().await.map_err(|error| error.to_string())?.active_tab_id.unwrap_or_else(|| id.into());
        let current = tab(runtime, &active).await?;
        if word == "back" && !current.can_go_back {
            actions.push(json!({"action":"back","performed":false,"reason":"no_history","target":current.target}));
            eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_HOLD Active tab has no Back history; entry then back creates a testable history step");
            continue;
        }
        let before_command = runtime.navigation_diagnostics(current.target.clone()).await.ok();
        let command = match word.as_str() {
            "entry" => Some(BrowserTabCommand::Navigate { target:current.target.clone(),
                url:SITES.iter().find(|(site,_)| *site == name).ok_or("Unknown manual site")?.1.into() }),
            "back" => Some(BrowserTabCommand::Back { target: current.target.clone() }),
            "reload" => Some(BrowserTabCommand::Reload { target: current.target.clone(),
                expected_navigation_sequence:current.load.as_ref().ok_or("Manual reload target has no native navigation sequence")?.navigation_sequence }),
            _ => None,
        };
        if let Some(command) = command {
            runtime.execute(command, CancellationToken::new()).await.map_err(|error| error.to_string())?;
        } else if matches!(word.as_str(), "hide" | "show") {
            runtime.surface().ok_or("Live runtime has no surface")?.set_surface(
                BrowserSurfaceBounds { x:20.0,y:40.0,width:840.0,height:540.0 }, word == "show", CancellationToken::new())
                .await.map_err(|error| error.to_string())?;
        }
        let current = tab(runtime, &active).await?;
        let report = runtime.navigation_diagnostics(current.target.clone()).await.ok();
        let effective = !matches!(word.as_str(),"entry" | "back" | "reload") || report.as_ref().zip(before_command.as_ref())
            .is_some_and(|(after,before)| after.trace.counters.user_commands > before.trace.counters.user_commands);
        if effective { marked.insert(word.clone()); }
        actions.push(json!({"action":word,"elapsed_ms":started.elapsed().as_millis(),"target":current.target,
            "performed":effective,
            "native_counters":report.as_ref().map(|report| &report.trace.counters),
            "physical_page_action":"typed/clicked are operator attestation; native action facts are recorded separately"}));
        if word == "done" { completed = true; break; }
    }
    let current = tab(runtime, id).await?;
    let report = runtime.navigation_diagnostics(current.target.clone()).await.ok();
    let native_page_action = report.as_ref().is_some_and(|report| report.trace.events.iter().any(|event|
        event.sequence > baseline && event.action.as_ref().is_some_and(|action|
            matches!(action.navigation_type, BrowserNavigationActionType::LinkActivated | BrowserNavigationActionType::FormSubmitted)
                && action.source_main_frame == Some(true))));
    let all_actions = ["typed", "clicked", "back", "reload", "hide", "show"].iter().all(|word| marked.contains(*word));
    Ok(json!({"required":true,"completed":completed && all_actions && native_page_action,
        "operator_finished":completed,"required_control_actions_recorded":all_actions,
        "native_link_or_form_navigation_observed":native_page_action,
        "driver":"physical_page_input_and_typed_host_command_console","elapsed_seconds":started.elapsed().as_secs(),"actions":actions}))
}

async fn screenshot(runtime: &Arc<dyn BrowserRuntime>, id: &str, name: &str, directory: Option<&Path>) -> Value {
    use base64::Engine;
    let Some(directory) = directory else { return json!({"saved":false,"reason":"no_evidence_directory"}); };
    let Some(automation) = runtime.automation() else { return json!({"saved":false,"reason":"no_screenshot_port"}); };
    let cancel = CancellationToken::new();
    let capture = tokio::time::timeout(Duration::from_secs(5), automation.screenshot(Some(id.into()), cancel.clone())).await;
    let capture = match capture {
        Ok(Ok(capture)) => capture,
        Ok(Err(error)) => return json!({"saved":false,"reason":error.code()}),
        Err(_) => { cancel.cancel(); return json!({"saved":false,"reason":"screenshot_timeout"}); }
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&capture.png_base64) else {
        return json!({"saved":false,"reason":"invalid_png_encoding"});
    };
    let filename = format!("{name}-idle.png");
    if std::fs::write(directory.join(&filename), bytes).is_err() { return json!({"saved":false,"reason":"write_failed"}); }
    json!({"saved":true,"filename":filename,"target":capture.target,"width":capture.width,"height":capture.height})
}
