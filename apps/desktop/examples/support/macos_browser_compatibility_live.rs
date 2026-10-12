//! Optional external entry checks, separate from deterministic fixture assertions.
//! No login, media playback claim, captcha handling, or navigation retries.
use nomifun_browser_platform::{runtime::*, url_projection::project_metadata_url};
use serde_json::{Value, json};
use std::{sync::Arc, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;

struct Site {
    name: &'static str,
    tab_id: String,
    samples: Vec<Value>,
    last_observation: Option<BrowserObservation>,
    last_read_error: Option<String>,
}

pub(crate) async fn verify(runtime: &Arc<dyn BrowserRuntime>) -> Result<Value, String> {
    runtime.lock_user_input().await.map_err(|error| error.to_string())?;
    let mut sites = Vec::new();
    for (name, url) in [("bilibili", "https://www.bilibili.com/"), ("baidu", "https://www.baidu.com/s?wd=nomifun")] {
        let snapshot = runtime.execute(BrowserTabCommand::Create { url: url.into() }, CancellationToken::new())
            .await.map_err(|error| error.to_string())?;
        let tab_id = snapshot.active_tab_id.ok_or("External fixture created no target")?;
        sites.push(Site { name, tab_id, samples: Vec::new(), last_observation: None, last_read_error: None });
    }
    eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_STAGE live_sites_observe_60_seconds");
    let started = Instant::now();
    let deadline = started + Duration::from_secs(60);
    loop {
        for site in &mut sites {
            let snapshot = runtime.snapshot().await.map_err(|error| error.to_string())?;
            let Some(tab) = snapshot.tabs.iter().find(|tab| tab.target.tab_id == site.tab_id) else { continue; };
            let report = runtime.navigation_diagnostics(tab.target.clone()).await;
            let host_requested = report.as_ref().ok().map(|report| report.trace.events.iter().filter(|event|
                event.kind == BrowserNavigationEventKind::Requested
                    && matches!(event.source, BrowserNavigationSource::UserCommand | BrowserNavigationSource::AgentCommand)).count());
            site.samples.push(json!({"elapsed_ms":started.elapsed().as_millis(),"lifecycle":tab.lifecycle,
                "load":tab.load.as_ref().map(BrowserLoadSummary::agent_projection),"host_requested_count":host_requested}));
            if tab.load.as_ref().is_some_and(|load| load.content_state != BrowserContentState::None) {
                let cancel = CancellationToken::new();
                let observed = tokio::time::timeout(Duration::from_secs(3), runtime.automation()
                    .ok_or("External fixture has no observation port")?.observe(Some(site.tab_id.clone()), cancel.clone())).await;
                match observed {
                    Ok(Ok(observation)) => { site.last_observation = Some(observation); site.last_read_error = None; }
                    Ok(Err(error)) => { site.last_read_error = Some(error.code().into()); }
                    Err(_) => { cancel.cancel(); site.last_read_error = Some("OBSERVATION_TIMEOUT".into()); }
                }
            }
        }
        if Instant::now() >= deadline { break; }
        tokio::time::sleep(Duration::from_secs(5).min(deadline.saturating_duration_since(Instant::now()))).await;
    }
    let mut reports = Vec::new();
    for site in sites {
        let snapshot = runtime.snapshot().await.map_err(|error| error.to_string())?;
        let Some(tab) = snapshot.tabs.into_iter().find(|tab| tab.target.tab_id == site.tab_id) else {
            reports.push(json!({"site":site.name,"outcome":"missing_tab","entry_check_passed":false,"samples":site.samples}));
            continue;
        };
        let diagnostic = runtime.navigation_diagnostics(tab.target.clone()).await.ok();
        let observation = site.last_observation.as_ref().filter(|observation| observation.target == tab.target);
        let text = observation.map_or("", |observation| observation.content.as_str());
        let meaningful = observation.is_some_and(|observation| observation.elements.len() > 1
            || observation.content.lines().skip(1).any(|line| !line.trim().is_empty()));
        let blocked_version = text.contains("浏览器版本过低") || text.contains("浏览器下载建议")
            || tab.url.contains("activity-CjJbuaD7Xw");
        let (outcome, passed) = if blocked_version { ("browser_version_rejected", false) }
            else if matches!(tab.lifecycle, BrowserTabLifecycle::Failed | BrowserTabLifecycle::Crashed | BrowserTabLifecycle::Stopped) {
                ("native_load_failure", false)
            } else if !meaningful || tab.load.as_ref().is_none_or(|load| load.phase != BrowserNavigationPhase::Finished) {
                ("insufficient_current_document_evidence", false)
            } else if site.name == "bilibili" && (text.contains("哔哩哔哩") || text.to_ascii_lowercase().contains("bilibili")) {
                ("homepage_document_observed", true)
            } else if site.name == "baidu" && (text.contains("安全验证") || text.contains("验证码")) {
                ("verification_document_observed", true)
            } else if site.name == "baidu" && text.contains("百度") && (text.contains("nomifun") || text.contains("搜索工具") || text.contains("下一页")) {
                ("search_document_observed", true)
            } else { ("unclassified_site_document", false) };
        let host_requests = diagnostic.as_ref().map(|report| report.trace.events.iter().filter(|event|
            event.kind == BrowserNavigationEventKind::Requested
                && matches!(event.source, BrowserNavigationSource::UserCommand | BrowserNavigationSource::AgentCommand)).count());
        reports.push(json!({"site":site.name,"outcome":outcome,"entry_check_passed":passed && host_requests == Some(1),
            "final_url":project_metadata_url(&tab.url),"final_lifecycle":tab.lifecycle,
            "last_read_error":site.last_read_error,"host_requested_count":host_requests,
            "dom_excerpt":text.chars().take(1500).collect::<String>(),"samples":site.samples,
            "navigation_report":diagnostic}));
    }
    runtime.release_pressed_input().await.map_err(|error| error.to_string())?;
    runtime.unlock_user_input().await.map_err(|error| error.to_string())?;
    let passed = reports.len() == 2 && reports.iter().all(|report| report["entry_check_passed"] == true);
    Ok(json!({"observed_seconds":started.elapsed().as_secs(),"entry_checks_passed":passed,"sites":reports,
        "login":"not_covered","actual_video_playback":"not_covered","captcha":"observed_only_not_solved"}))
}
