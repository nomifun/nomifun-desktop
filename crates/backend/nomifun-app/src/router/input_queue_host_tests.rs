//! Product intake, native input boundaries, and queue controls share one Host.
use super::*;

async fn response(f:&Fixture,method:&str,path:&str,body:Value)->(axum::http::StatusCode,Value) {
    let result=f._router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust","cleanup-fixture-local-trust").header("content-type","application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status=result.status();
    let value=serde_json::from_slice(&axum::body::to_bytes(result.into_body(),4*1024*1024).await.unwrap()).unwrap();
    (status,value)
}

async fn queue(f:&Fixture)->Value {
    let (status,value)=response(f,"GET",&format!("/api/agent-sessions/{}/input-queue",f.host.options.conversation_id),Value::Null).await;
    assert!(status.is_success(),"queue GET: {status} {value}");value["data"].clone()
}

#[tokio::test]
async fn input_queue_default_preserves_each_root_and_immediate_only_reaches_observed_turn() {
    let f=Fixture::new("composer-queue-default").await;
    let (request,runtime)=prepare_live_steering(&f).await;
    let session=AgentSessionId::from(f.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let a=api(&f._router,&path,json!({"idempotency_key":"queue-a","input":{"content":"Run A after the current task"}})).await;
    let b=api(&f._router,&path,json!({"idempotency_key":"queue-b","delivery":{"kind":"queue"},"input":{"content":"Run B after A"}})).await;
    let immediate=json!({"idempotency_key":"immediate-c","delivery":{"kind":"prefer_current","turn_id":f.message.source_message_id},
        "input":{"content":"C corrects this exact current task"}});
    let c=api(&f._router,&path,immediate.clone()).await;
    let port=request.input_port.unwrap();let cause=request.model_request.causality;
    assert!(port.has_pending(&cause).await.unwrap());
    let delivered=port.take(&cause,false).await.unwrap();
    assert_eq!(delivered.len(),1);assert_eq!(delivered[0].message_id,c["message_id"].as_str().unwrap());
    assert!(!port.has_pending(&cause).await.unwrap(),"queued roots must not cause model sampling or tool cancellation");
    assert!(port.take(&cause,false).await.unwrap().is_empty());
    for input in [&a,&b] {
        assert!(f.owner.canonical().store().session_input_claim(&session,&input["message_id"].as_str().unwrap().into()).await.unwrap().is_none());
    }
    let pending=queue(&f).await;
    assert_eq!(pending["items"].as_array().unwrap().iter().map(|item|item["message_id"].clone()).collect::<Vec<_>>(),vec![a["message_id"].clone(),b["message_id"].clone()]);
    let before=f.owner.canonical().store().current_cursor(&session).await.unwrap();
    let replay=api(&f._router,&path,immediate).await;
    assert_eq!(replay["message_id"],c["message_id"]);assert_eq!(replay["replayed"],true);
    assert_eq!(before,f.owner.canonical().store().current_cursor(&session).await.unwrap());
    runtime.kill_and_wait(None).await.unwrap();f.finish().await;
}

#[tokio::test]
async fn input_queue_closed_window_retains_immediate_without_claim_or_retarget() {
    let f=Fixture::new("composer-closed-window").await;
    let (request,runtime)=prepare_live_steering(&f).await;
    let session=AgentSessionId::from(f.host.options.conversation_id.clone());
    let port=request.input_port.unwrap();let cause=request.model_request.causality;
    assert!(port.take(&cause,true).await.unwrap().is_empty());
    let body=json!({"idempotency_key":"closed-window","delivery":{"kind":"prefer_current","turn_id":f.message.source_message_id},"input":{"content":"Keep this original decision for a later independent task"}});
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let first=api(&f._router,&path,body.clone()).await;
    assert_eq!(api(&f._router,&path,body).await["message_id"],first["message_id"]);
    assert!(!port.has_pending(&cause).await.unwrap());
    assert!(f.owner.canonical().store().session_input_claim(&session,&first["message_id"].as_str().unwrap().into()).await.unwrap().is_none());
    assert_eq!(queue(&f).await["items"][0]["message_id"],first["message_id"]);
    runtime.kill_and_wait(None).await.unwrap();f.finish().await;
}

#[tokio::test]
async fn input_queue_remove_is_terminal_and_replay_survives_a_later_domain_owner() {
    let f=Fixture::new("composer-input-receipt").await;
    let (_request,runtime)=prepare_live_steering(&f).await;
    let session=AgentSessionId::from(f.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let input=json!({"idempotency_key":"original-queue-intent","delivery":{"kind":"queue"},"input":{"content":"One original queued task"}});
    let first=api(&f._router,&path,input.clone()).await;let snapshot=queue(&f).await;
    let original_input=nomifun_agent_contracts::EventId::from(first["message_id"].as_str().unwrap());
    let original_preparation=f.owner.hold_input_preparation_for_test(&session,&original_input).unwrap();
    let command=json!({"idempotency_key":"remove-original","expected_version":snapshot["version"],"action":{"kind":"remove","message_id":first["message_id"]}});
    let queue_path=format!("/api/agent-sessions/{}/input-queue",session.as_ref());
    let removed=api(&f._router,&queue_path,command.clone()).await;
    assert!(removed["items"].as_array().unwrap().is_empty());
    assert_eq!(f.owner.input_preparation_observations(&session).unwrap()[0].1,true,"terminal removal signals the original preparation owner");
    drop(original_preparation);
    assert_eq!(api(&f._router,&queue_path,command.clone()).await["version"],removed["version"]);
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,created_at,updated_at) VALUES(?,?,'attempt',?,?,1,1)")
        .bind(session.as_ref()).bind(uuid::Uuid::now_v7().to_string()).bind(uuid::Uuid::now_v7().to_string()).bind(uuid::Uuid::now_v7().to_string()).execute(f.pool()).await.unwrap();
    assert!(queue(&f).await["items"].as_array().unwrap().is_empty(),"read-only queue projection does not acquire ordinary input authority");
    assert_eq!(api(&f._router,&queue_path,command).await["version"],removed["version"],"a later domain owner cannot revoke the original control acknowledgement");
    let before=f.owner.canonical().store().current_cursor(&session).await.unwrap();
    let replay=api(&f._router,&path,input.clone()).await;
    assert_eq!(replay["message_id"],first["message_id"]);assert_eq!(replay["replayed"],true);
    let mut fresh=input.clone();fresh["idempotency_key"]=json!("new-domain-rejected");
    let (status,error)=response(&f,"POST",&path,fresh).await;
    assert!(status.is_client_error());assert_eq!(error["code"],"INPUT_NOT_ACCEPTED");
    let mut changed=input;changed["delivery"]=json!({"kind":"prefer_current","turn_id":f.message.source_message_id});
    let (status,error)=response(&f,"POST",&path,changed).await;
    assert!(status.is_client_error());assert_ne!(error["code"],"INPUT_NOT_ACCEPTED");
    assert_eq!(before,f.owner.canonical().store().current_cursor(&session).await.unwrap());
    assert_eq!(f.owner.canonical().store().session_input_delivery(&session,&first["message_id"].as_str().unwrap().into()).await.unwrap()["state"],"cancelled");
    runtime.kill_and_wait(None).await.unwrap();f.finish().await;
}

#[tokio::test]
async fn input_queue_permanent_validation_only_releases_an_authoritatively_absent_identity() {
    let f=Fixture::new("composer-permanent-validation").await;
    let (_request,runtime)=prepare_live_steering(&f).await;
    let path=format!("/api/agent-sessions/{}/messages",f.host.options.conversation_id);
    let input=json!({"idempotency_key":"original-valid","input":{"content":"One stable intent"}});
    let saved=api(&f._router,&path,input.clone()).await;
    for key in ["new-invalid","original-valid"] {
        let (status,error)=response(&f,"POST",&path,json!({"idempotency_key":key,"input":{"content":"too large".repeat(10000)}})).await;
        assert!(status.is_client_error());
        assert_eq!(error["code"]=="INPUT_NOT_ACCEPTED",key=="new-invalid");
    }
    assert_eq!(api(&f._router,&path,input).await["message_id"],saved["message_id"]);
    runtime.kill_and_wait(None).await.unwrap();f.finish().await;
}

#[tokio::test]
async fn input_queue_stop_commits_hold_before_signalling_and_get_never_dispatches() {
    let f=Fixture::new("composer-stop-input-hold").await;
    let (_request,runtime)=prepare_live_steering(&f).await;
    let session=AgentSessionId::from(f.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let saved=api(&f._router,&path,json!({"idempotency_key":"held-after-stop","input":{"content":"A retained queued task"}})).await;
    let barrier=f.owner.pause_next_cancellation_signal(&session);
    let router=f._router.clone();let stop_path=format!("/api/agent-sessions/{}/turns/cancel",session.as_ref());
    let source=f.message.source_message_id.clone();
    let stop=tokio::spawn(async move {api(&router,&stop_path,json!({"idempotency_key":"stop-and-hold","expected_turn_id":source})).await});
    tokio::time::timeout(Duration::from_secs(5),barrier.entered.notified()).await.unwrap();
    let held=queue(&f).await;assert_eq!(held["items"][0]["held"],true);
    let cursor=f.owner.canonical().store().current_cursor(&session).await.unwrap();
    assert_eq!(queue(&f).await,held);
    assert_eq!(f.owner.canonical().store().current_cursor(&session).await.unwrap(),cursor);
    assert!(f.owner.canonical().store().session_input_claim(&session,&saved["message_id"].as_str().unwrap().into()).await.unwrap().is_none());
    barrier.release.notify_one();stop.await.unwrap();
    assert!(f.owner.canonical().store().pending_session_inputs(&session,None).await.unwrap().is_empty());
    runtime.kill_and_wait(None).await.unwrap();f.finish().await;
}

#[tokio::test]
async fn input_queue_pause_winning_native_preparation_does_not_fail_the_running_turn() {
    let f=Fixture::new("composer-pause-preparation-race").await;
    let (request,runtime)=prepare_live_steering(&f).await;
    let session=AgentSessionId::from(f.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let saved=api(&f._router,&path,json!({"idempotency_key":"preparing-immediate",
        "delivery":{"kind":"prefer_current","turn_id":f.message.source_message_id},"input":{"content":"Wait for this original user decision"}})).await;
    let entered=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());
    INPUT_PRECLAIM_BARRIERS.get_or_init(Default::default).lock().unwrap().insert(session.as_ref().into(),(entered.clone(),release.clone()));
    let port=request.input_port.unwrap();let cause=request.model_request.causality;
    let waiter=tokio::spawn(async move {port.take(&cause,false).await});
    tokio::time::timeout(Duration::from_secs(5),entered.notified()).await.unwrap();
    let snapshot=queue(&f).await;
    api(&f._router,&format!("/api/agent-sessions/{}/input-queue",session.as_ref()),json!({
        "idempotency_key":"pause-preparing-input","expected_version":snapshot["version"],"action":{"kind":"pause"}})).await;
    release.notify_one();
    assert!(tokio::time::timeout(Duration::from_secs(5),waiter).await.unwrap().unwrap().unwrap().is_empty());
    assert_eq!(f.owner.canonical().store().head(&session).await.unwrap().status,"running");
    assert!(f.owner.canonical().store().session_input_claim(&session,&saved["message_id"].as_str().unwrap().into()).await.unwrap().is_none());
    assert_eq!(queue(&f).await["items"][0]["held"],true);
    runtime.kill_and_wait(None).await.unwrap();f.finish().await;
}
