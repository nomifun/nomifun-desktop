use super::*;
use nomifun_browser_platform::runtime::{BrowserTabLifecycle, BrowserTabTarget};
use serde_json::json;

fn frame(scope: &mut scopes::ScopedProjection, tab: &mut BrowserTabSnapshot, session: &str, name: &str, loader: &str) {
    let generation=tab.target.document_generation;
    scope.apply(tab, generation, session, "Page.frameNavigated", json!({"frame":{"id":name,"loaderId":loader}}));
    scope.apply(tab, generation, session, "Runtime.executionContextCreated", json!({"context":{"id":1,"uniqueId":format!("{session}-{loader}"),"auxData":{"isDefault":true,"frameId":name}}}));
}
fn attach(scope: &mut scopes::ScopedProjection, tab: &mut BrowserTabSnapshot, parent: &str, session: &str) {
    let generation=tab.target.document_generation;
    scope.apply(tab, generation, parent, "Target.attachedToTarget", json!({"sessionId":session,"targetInfo":{"type":"iframe","targetId":format!("frame-{session}")}}));
}
fn console() -> Value { json!({"executionContextId":1,"type":"error","args":[{"value":"page error"}]}) }

#[test]
fn only_owned_iframe_lineage_and_live_page_contexts_produce_diagnostics() {
    let mut tab=tab(); let mut scope=scopes::ScopedProjection::default();
    scope.apply(&mut tab,1,"unknown","Runtime.consoleAPICalled",console());
    scope.apply(&mut tab,1,"","Target.attachedToTarget",json!({"sessionId":"worker","targetInfo":{"type":"worker","targetId":"worker"}}));
    frame(&mut scope,&mut tab,"worker","worker","loader");
    scope.apply(&mut tab,1,"worker","Runtime.consoleAPICalled",console());
    assert!(tab.diagnostics.entries.is_empty());
    attach(&mut scope,&mut tab,"","one"); attach(&mut scope,&mut tab,"one","two");
    frame(&mut scope,&mut tab,"two","frame-two","loader");
    scope.apply(&mut tab,1,"two","Runtime.consoleAPICalled",console());
    assert_eq!(tab.diagnostics.entries.len(),1);
    scope.apply(&mut tab,1,"","Target.detachedFromTarget",json!({"sessionId":"one"}));
    scope.apply(&mut tab,1,"two","Runtime.consoleAPICalled",console());
    assert_eq!(tab.diagnostics.entries.len(),1);
    attach(&mut scope,&mut tab,"one","late-child");
    frame(&mut scope,&mut tab,"late-child","late","loader");
    scope.apply(&mut tab,1,"late-child","Runtime.consoleAPICalled",console());
    assert_eq!(tab.diagnostics.entries.len(),1);
}

#[test]
fn frame_navigation_revokes_contexts_and_pending_requests_without_touching_siblings() {
    let mut tab=tab(); let mut scope=scopes::ScopedProjection::default();
    for name in ["one","two"] {
        attach(&mut scope,&mut tab,"",name); frame(&mut scope,&mut tab,name,name,"old");
        scope.apply(&mut tab,1,name,"Network.requestWillBeSent",json!({"requestId":"same-id","frameId":name,"loaderId":"old","request":{"url":format!("https://{name}.example/path?secret=hidden")}}));
    }
    scope.apply(&mut tab,1,"one","Page.frameNavigated",json!({"frame":{"id":"one","loaderId":"new"}}));
    scope.apply(&mut tab,1,"one","Runtime.consoleAPICalled",console());
    scope.apply(&mut tab,1,"one","Network.loadingFailed",json!({"requestId":"same-id","errorText":"old failure"}));
    assert!(tab.diagnostics.entries.is_empty());
    scope.apply(&mut tab,1,"two","Network.loadingFailed",json!({"requestId":"same-id","errorText":"current failure"}));
    assert_eq!(tab.diagnostics.entries.len(),1);
    assert_eq!(tab.diagnostics.entries[0].source_url,"https://two.example/path");
    tab.target.document_generation=2; tab.diagnostics=Default::default();
    scope.apply(&mut tab,1,"two","Runtime.consoleAPICalled",console());
    scope.apply(&mut tab,2,"two","Runtime.consoleAPICalled",console());
    assert!(tab.diagnostics.entries.is_empty());
}

#[test]
fn unique_context_destroy_does_not_revoke_a_reused_numeric_context_id() {
    let mut tab=tab(); let mut scope=scopes::ScopedProjection::default();
    frame(&mut scope,&mut tab,"","root","new");
    scope.apply(&mut tab,1,"","Runtime.executionContextDestroyed",json!({"executionContextId":1,"executionContextUniqueId":"old-identity"}));
    scope.apply(&mut tab,1,"","Runtime.consoleAPICalled",console());
    assert_eq!(tab.diagnostics.entries.len(),1);
    scope.apply(&mut tab,1,"","Runtime.executionContextDestroyed",json!({"executionContextId":1,"executionContextUniqueId":"-new"}));
    scope.apply(&mut tab,1,"","Runtime.consoleAPICalled",console());
    assert_eq!(tab.diagnostics.entries.len(),1);
}

#[test]
fn late_tree_seed_cannot_roll_back_navigation_or_resurrect_detached_frames() {
    let mut tab=tab(); let mut scope=scopes::ScopedProjection::default();
    frame(&mut scope,&mut tab,"","root","new");
    scope.apply(&mut tab,1,"","Nomi.frameTree",json!({"frameTree":{"frame":{"id":"root","loaderId":"old"}}}));
    scope.apply(&mut tab,1,"","Runtime.consoleAPICalled",console());
    scope.apply(&mut tab,1,"","Network.requestWillBeSent",json!({"requestId":"current","frameId":"root","loaderId":"new","request":{"url":"https://example.com/current"}}));
    scope.apply(&mut tab,1,"","Network.loadingFailed",json!({"requestId":"current","errorText":"current failure"}));
    assert_eq!(tab.diagnostics.entries.len(),2);
    scope.apply(&mut tab,1,"","Page.frameDetached",json!({"frameId":"root"}));
    scope.apply(&mut tab,1,"","Nomi.frameTree",json!({"frameTree":{"frame":{"id":"root","loaderId":"old"}}}));
    scope.apply(&mut tab,1,"","Network.requestWillBeSent",json!({"requestId":"late","frameId":"root","loaderId":"old","request":{"url":"https://example.com/old"}}));
    scope.apply(&mut tab,1,"","Network.loadingFailed",json!({"requestId":"late","errorText":"late failure"}));
    assert_eq!(tab.diagnostics.entries.len(),2);
}

#[test]
fn detached_seeded_subtree_cannot_be_resurrected_by_a_late_snapshot() {
    let mut tab=tab();let mut scope=scopes::ScopedProjection::default();
    let tree=json!({"frameTree":{"frame":{"id":"parent","loaderId":"parent-old"},"childFrames":[{"frame":{"id":"child","parentId":"parent","loaderId":"child-old"}}]}});
    scope.apply(&mut tab,1,"","Nomi.frameTree",tree.clone());
    scope.apply(&mut tab,1,"","Page.frameDetached",json!({"frameId":"parent"}));
    scope.apply(&mut tab,1,"","Nomi.frameTree",tree);
    scope.apply(&mut tab,1,"","Network.requestWillBeSent",json!({"requestId":"late","frameId":"child","loaderId":"child-old","request":{"url":"https://example.com/old"}}));
    scope.apply(&mut tab,1,"","Network.loadingFailed",json!({"requestId":"late","errorText":"old child failure"}));
    assert!(tab.diagnostics.entries.is_empty());
}

#[test]
fn lost_lifecycle_reads_and_full_queues_fence_diagnostics_but_plain_log_loss_does_not() {
    let metadata=Mutex::new(tab()); let lost=AtomicBool::new(false);
    let (sender,_receiver)=mpsc::channel(1);
    enqueue(&sender,&metadata,&lost,"Runtime.consoleAPICalled",None);
    assert!(!lost.load(Ordering::Acquire));
    enqueue(&sender,&metadata,&lost,"Runtime.executionContextDestroyed",None);
    assert!(lost.load(Ordering::Acquire));
    lost.store(false,Ordering::Release);
    enqueue(&sender,&metadata,&lost,"Runtime.consoleAPICalled",Some((String::new(),"{}".into())));
    enqueue(&sender,&metadata,&lost,"Target.detachedFromTarget",Some((String::new(),"{}".into())));
    assert!(lost.load(Ordering::Acquire));
    assert_eq!(metadata.lock().unwrap().diagnostics.dropped,3);
    let mut tab=metadata.lock().unwrap();
    assert!(tab.diagnostics.unavailable);
    tab.diagnostics.clear_page();
    assert!(tab.diagnostics.unavailable);
}

fn tab() -> BrowserTabSnapshot {
    BrowserTabSnapshot {
        target: BrowserTabTarget {
            tab_id: "fixture".into(),
            runtime_generation: 1,
            document_generation: 1,
        },
        title: String::new(),
        url: "https://example.com".into(),
        lifecycle: BrowserTabLifecycle::Ready,
        can_go_back: false,
        can_go_forward: false,
        blocked_permissions: vec![],
        permission_requests: vec![],
        script_dialog: None,
        diagnostics: Default::default(),
    }
}

#[test]
fn console_projection_is_bounded_and_does_not_export_remote_handles() {
    let mut tab = tab();
    let mut projection = Projection::default();
    for _ in 0..40 {
        assert!(projection.apply(&mut tab, 1, "Runtime.consoleAPICalled", json!({"type":"error","args":[{"type":"object","objectId":"private-handle","description":"x".repeat(2000)}]})));
    }
    assert_eq!(tab.diagnostics.entries.len(), MAX_ENTRIES);
    assert_eq!(tab.diagnostics.dropped, 8);
    assert!(
        serde_json::to_value(&tab)
            .unwrap()
            .get("diagnostics")
            .is_none()
    );
    assert_eq!(tab.diagnostics.entries[0].message.len(), MAX_TEXT);
    assert!(
        !serde_json::to_string(&tab.diagnostics)
            .unwrap()
            .contains("private-handle")
    );
}

#[test]
fn queued_old_document_events_do_not_repopulate_cleared_diagnostics() {
    let mut tab = tab();
    let mut projection = Projection::default();
    tab.target.document_generation = 2;
    assert!(!projection.apply(
        &mut tab,
        1,
        "Runtime.exceptionThrown",
        json!({"exceptionDetails":{"text":"old error"}})
    ));
    assert!(tab.diagnostics.entries.is_empty());
}

#[test]
fn network_metadata_discards_credentials_queries_fragments_and_request_bodies() {
    let mut tab = tab();
    let mut projection = Projection::default();
    projection.apply(&mut tab, 1, "Network.requestWillBeSent", json!({"requestId":"one","request":{"url":"https://user:pass@example.com/path?token=secret#value","postData":"private-body","headers":{"Authorization":"private-header"}}}));
    projection.apply(
        &mut tab,
        1,
        "Network.loadingFailed",
        json!({"requestId":"one","errorText":"net::ERR_FAILED"}),
    );
    let encoded = serde_json::to_string(&tab.diagnostics).unwrap();
    assert_eq!(
        tab.diagnostics.entries[0].source_url,
        "https://example.com/path"
    );
    for secret in ["pass", "token", "secret", "private-body", "private-header"] {
        assert!(!encoded.contains(secret));
    }
    assert!(projection.requests.is_empty());
}

#[test]
fn in_flight_request_metadata_has_a_hard_limit_and_clears_on_navigation() {
    let mut tab = tab();
    let mut projection = Projection::default();
    for id in 0..200 {
        projection.apply(
            &mut tab,
            1,
            "Network.requestWillBeSent",
            json!({"requestId":id.to_string(),"request":{"url":"https://example.com/"}}),
        );
    }
    assert_eq!(projection.requests.len(), MAX_REQUESTS);
    assert_eq!(tab.diagnostics.dropped, 72);
    tab.target.document_generation = 2;
    projection.apply(
        &mut tab,
        2,
        "Network.loadingFailed",
        json!({"requestId":"0","errorText":"cancelled"}),
    );
    assert!(tab.diagnostics.entries.is_empty());
    assert!(projection.requests.is_empty());
}
