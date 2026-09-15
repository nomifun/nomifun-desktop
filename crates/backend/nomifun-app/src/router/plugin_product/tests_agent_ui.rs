use super::*;

#[test]
fn authoring_rejects_session_view_but_keeps_ordinary_page_and_continuous_service() {
    let declaration = serde_json::json!({"agent_view":{"name":"Retired","description":"No Session access"}});
    assert!(super::super::authoring::validate_plugin_source(HTML,None,Some(&declaration)).is_err());
    assert!(super::super::authoring::validate_plugin_source(HTML,None,None).is_ok());
    let service = "export async function start() { return {async invoke() { return {}; }, async dispose() {}}; }";
    let ordinary = serde_json::json!({"lifecycle":"continuous","actions":[{"id":"echo","name":"Echo","description":"Normal app Service","input_schema":{"type":"object"},"output_schema":{"type":"object"},"effect":"pure"}]});
    super::super::authoring::validate_plugin_source(HTML,Some(service),Some(&ordinary)).unwrap();
}

#[tokio::test]
async fn session_grants_are_rejected_while_ordinary_surface_opens() {
    let (_db,_root,service,owner) = fixture().await;
    let mut value = draft();
    service.put_draft(&owner,&mut value).await.unwrap();
    let SaveOutcome::Saved(saved) = service.save_draft(&owner,&mut value,None).await.unwrap() else {panic!("ordinary UI-only save")};
    let id = &saved.plugin.plugin_id;
    let release = saved.plugin.releases.active.as_ref().unwrap();
    let error = service.application.open_surface_with_agent_session(&owner,id,Some(("old-session",&release.release_digest))).await.unwrap_err();
    assert!(error.to_string().contains("unsupported"));
    let surface = service.application.open_surface(&owner,id).await.unwrap();
    let error = service.application.surface_bridge_request(&owner,id,&surface.surface_capability,surface.active_release_epoch,&surface.expected_release_digest,
        serde_json::from_value(serde_json::json!({"call_id":"observe","target":{"target":"agent_session","request":{"operation":"observe","after_seq":0,"limit":10}}})).unwrap()
    ).await.unwrap_err();
    assert!(error.to_string().contains("unsupported"));
    let storage_request = || serde_json::from_value(serde_json::json!({"call_id":"storage-probe","target":{"target":"host_kv","request":{"operation":"get","key":"ordinary-app"}}})).unwrap();
    assert!(service.application.surface_bridge_request(&owner,id,&surface.surface_capability,surface.active_release_epoch,&surface.expected_release_digest,storage_request()).await.is_ok());
    service.application.close_surface(&owner,id,&surface.surface_session_id,&surface.surface_capability).await.unwrap();
    assert!(service.application.surface_bridge_request(&owner,id,&surface.surface_capability,surface.active_release_epoch,&surface.expected_release_digest,storage_request()).await.is_err());
}
