//! A public prefix is immutable content, but cancellation revokes permission
//! to send further frames. The UI cancel endpoint does not take the Session
//! operation lock, so the writer and cancel must share SQLite serialization.
use super::*;

#[tokio::test]
async fn direct_public_prefix_ui_cancel_revokes_original_key_at_the_writer_boundary() {
    use nomifun_robot::services::TurnEvent;
    let server=CompanionFlowModelServer::new().await;
    server.state.hold_research.store(true,std::sync::atomic::Ordering::SeqCst);
    let flow=PhysicalEntryFixture::with_research(&server,"direct-public-ui-cancel",true).await;
    install_search_provider(&flow.fixture._router,&server).await;
    let backend=flow.fixture.services.robot.as_ref().unwrap().conversation_backend_for_test();
    let mut request=flow.request("direct-public-held-request",WEATHER_QUERY);
    request.captured_context=backend.capture_input_context(&flow.companion_id,&flow.device_id,
        flow.robot_id.as_ref(),&flow.connection,None).await.unwrap();
    let original_context=request.captured_context.clone().unwrap();
    let mut events=backend.dispatch(request.clone()).await.unwrap();
    let source=tokio::time::timeout(Duration::from_secs(20),async {loop {
        match events.recv().await.unwrap() {
            TurnEvent::PublicSpeech {source,text}=>{assert_eq!(text,"正在查询原始天气来源。");break source;},
            other=>panic!("the admitted research must publish its real progress before completion: {other:?}"),
        }
    }}).await.unwrap();
    source.validate().unwrap();
    tokio::time::timeout(Duration::from_secs(10),server.state.held_entered.notified()).await.unwrap();
    let key=crate::robot_wiring::robot_turn_idempotency_key(&request);
    let principal=PrincipalRef {principal_kind:"user".into(),principal_id:flow.owner.clone()};
    let input=flow.fixture.owner.canonical().session_input_by_key(&principal,&flow.robot_id,&key).await.unwrap().unwrap();
    let store=flow.fixture.owner.canonical().store();
    let claim=store.session_input_claim(&flow.robot_id,&input.event.event_id).await.unwrap().unwrap();
    assert_eq!(store.read_turn_receipt(&flow.robot_id,&claim.operation_id).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Running);
    assert!(backend.input_context_matches(&request).await.unwrap());
    // This is the exact lease that the writer holds only around permit.send.
    // Do not query the pool while retaining it: this also works with one DB
    // connection and keeps capacity/provider waiting outside its transaction.
    let lease=backend.acquire_output_context(&request,None,None).await.unwrap().unwrap();
    let router=flow.fixture._router.clone();
    let path=format!("/api/agent-sessions/{}/turns/cancel",flow.robot_id.as_ref());
    let root=input.event.event_id.as_ref().to_owned();
    let cancelling=tokio::spawn(async move {api(&router,&path,json!({
        "idempotency_key":"cancel-original-public-prefix","expected_turn_id":root,
    })).await});
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!cancelling.is_finished(),"UI cancel cannot commit through the writer's synchronous source lease");
    drop(lease);
    let receipt=tokio::time::timeout(Duration::from_secs(10),cancelling).await.unwrap().unwrap();
    assert_eq!(receipt["target_operation_id"],claim.operation_id.as_ref());
    assert_eq!(store.read_turn_receipt(&flow.robot_id,&claim.operation_id).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Cancelled);
    assert!(flow.fixture.owner.robot_input_context_matches(&flow.owner,&request).await.unwrap(),
        "cancellation leaves the binding/context scope valid, reproducing the original authorization gap");
    assert!(!backend.input_context_matches(&request).await.unwrap(),"the exact cancelled key releases a held provider promptly");
    assert!(backend.acquire_output_context(&request,None,None).await.unwrap().is_none(),
        "cancel winning before writer acceptance fences every later frame from the original key");
    assert_eq!(request.captured_context.as_ref().unwrap(),&original_context);
    server.state.held_release.notify_one();
    tokio::time::timeout(Duration::from_secs(10),async {while let Some(event)=events.recv().await {
        assert!(!matches!(event,TurnEvent::PublicSpeech {..}),"a cancelled original cannot supply a later spoken result");
        if matches!(event,TurnEvent::Done|TurnEvent::Failed {..}) {break;}
    }}).await.unwrap();
    flow.finish().await;server.close().await;
}
