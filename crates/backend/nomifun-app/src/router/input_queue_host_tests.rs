//! Product intake, native input boundaries, and queue controls share one Host.
use super::*;

struct InputQueueModel {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    responses: Arc<tokio::sync::Semaphore>,
    task: tokio::task::JoinHandle<()>,
}

impl InputQueueModel {
    async fn new(gated_requests:usize) -> Self {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/v1",listener.local_addr().unwrap());
        let requests=Arc::new(Mutex::new(Vec::<Value>::new()));
        let responses=Arc::new(tokio::sync::Semaphore::new(0));
        let router=axum::Router::new()
            .route("/v1/models",axum::routing::get(||async {
                axum::Json(json!({"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}))
            }))
            .route("/v1/chat/completions",axum::routing::post(|
                axum::extract::State((requests,responses,gated_requests)):axum::extract::State<(Arc<Mutex<Vec<Value>>>,Arc<tokio::sync::Semaphore>,usize)>,
                axum::Json(body):axum::Json<Value>,
            |async move {
                let call={let mut requests=requests.lock().unwrap();requests.push(body.clone());requests.len()};
                if call<=gated_requests {
                    let Ok(permit)=responses.acquire().await else {
                        return ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],String::new());
                    };
                    permit.forget();
                }
                let text=body["messages"].as_array().unwrap().iter().rev().find(|message|message["role"]=="user")
                    .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string())).unwrap();
                let chunk=json!({"id":format!("input-queue-{call}"),"object":"chat.completion.chunk","created":1,
                    "model":"cleanup-fixture","choices":[{"index":0,"delta":{"role":"assistant","content":format!("Acknowledged: {text}")},"finish_reason":null}]});
                let end=json!({"id":format!("input-queue-{call}"),"object":"chat.completion.chunk","created":1,
                    "model":"cleanup-fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
            })).with_state((requests.clone(),responses.clone(),gated_requests));
        let task=tokio::spawn(async move {axum::serve(listener,router).await.unwrap();});
        Self {url,requests,responses,task}
    }

    async fn wait_for_requests(&self,count:usize) {
        tokio::time::timeout(Duration::from_secs(15),async {
            while self.requests.lock().unwrap().len()<count {tokio::time::sleep(Duration::from_millis(2)).await;}
        }).await.expect("accepted idle/FIFO work must reach the actual model without another user action");
    }

    async fn close(self) {self.responses.close();self.task.abort();let _=self.task.await;}
}

async fn idle_fixture(scenario:&str,server:&InputQueueModel)->Fixture {
    let fixture=Fixture::with_input_model(scenario,&server.url).await;
    let session=&fixture.host.options.conversation_id;
    nomifun_agent_execution::AgentExecutionSessionPort::cancel_turn_for_execution(fixture.owner.as_ref(),&fixture.host.options.user_id,
        session,"queue-setup-stop","cleanup-retry-turn").await.unwrap();
    fixture.services.agent_runtime_sessions.retire_runtime_if_matches(session,&fixture.runtime).await.unwrap();
    assert!(server.requests.lock().unwrap().is_empty());
    fixture
}

async fn input_completed(fixture:&Fixture,input:&Value)->nomifun_agent_session::SessionInputClaim {
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let input=nomifun_agent_contracts::EventId::from(input["message_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            if let Some(claim)=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap() {
                let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&claim.operation_id).await.unwrap();
                if receipt.status==nomifun_agent_session::TurnReceiptStatus::Completed {break claim;}
                assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,
                    "input failed instead of progressing: {receipt:?}");
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.expect("accepted input must settle on its exact actual model Turn")
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn input_queue_idle_send_busy_fifo_and_post_completion_send_reach_the_model_without_manual_wake() {
    let server=InputQueueModel::new(1).await;
    let fixture=idle_fixture("composer-idle-fifo-model",&server).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let body=|key:&str,text:&str|json!({"idempotency_key":key,"input":{"content":text}});
    let first_body=body("cold-idle","Acknowledge cold-idle marker.");
    let first=api(&fixture._router,&path,first_body.clone()).await;
    assert_eq!(first["input_delivery"]["placement"],"transcript","idle input belongs in the conversation from acceptance");
    server.wait_for_requests(1).await;
    let second=api(&fixture._router,&path,body("fifo-second","Acknowledge fifo-second marker.")).await;
    let third=api(&fixture._router,&path,body("fifo-third","Acknowledge fifo-third marker.")).await;
    assert_eq!(second["input_delivery"]["placement"],"queued");
    assert_eq!(third["input_delivery"]["placement"],"queued");
    assert_eq!(server.requests.lock().unwrap().len(),1,"busy default input must not enter the active model Turn");
    for input in [&second,&third] {
        assert!(fixture.owner.canonical().store().session_input_claim(&session,&input["message_id"].as_str().unwrap().into()).await.unwrap().is_none());
    }
    server.responses.add_permits(1);
    let one=input_completed(&fixture,&first).await;
    let two=input_completed(&fixture,&second).await;
    let three=input_completed(&fixture,&third).await;
    assert_ne!(one.operation_id,two.operation_id);assert_ne!(two.operation_id,three.operation_id);
    let requests=server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(),3);
    assert!(!requests[0]["messages"].to_string().contains("fifo-second"));
    assert!(requests[1]["messages"].to_string().contains("fifo-second"));
    assert!(!requests[1]["messages"].to_string().contains("fifo-third"));
    assert!(requests[2]["messages"].to_string().contains("fifo-third"));
    // Send at the canonical terminal boundary, while the original relay and
    // automatic carrier retirement may still be converging.
    let after=api(&fixture._router,&path,body("after-terminal","Acknowledge after-terminal marker.")).await;
    assert_eq!(after["input_delivery"]["placement"],"transcript");
    input_completed(&fixture,&after).await;
    let before=fixture.owner.canonical().store().current_cursor(&session).await.unwrap();
    assert_eq!(api(&fixture._router,&path,first_body).await["message_id"],first["message_id"]);
    assert_eq!(fixture.owner.canonical().store().current_cursor(&session).await.unwrap(),before);
    assert_eq!(server.requests.lock().unwrap().len(),4,"replayed consumed input does not execute again");
    fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn input_queue_concurrent_idle_sends_claim_only_the_first_canonical_input_before_its_successor() {
    let server=InputQueueModel::new(1).await;
    let fixture=idle_fixture("composer-concurrent-idle-model",&server).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let (a,b)=tokio::join!(
        api(&fixture._router,&path,json!({"idempotency_key":"idle-race-a","input":{"content":"Acknowledge idle-race-a marker."}})),
        api(&fixture._router,&path,json!({"idempotency_key":"idle-race-b","input":{"content":"Acknowledge idle-race-b marker."}})),
    );
    server.wait_for_requests(1).await;
    let store=fixture.owner.canonical().store();
    let a_source=store.session_input(&session,&a["message_id"].as_str().unwrap().into()).await.unwrap();
    let b_source=store.session_input(&session,&b["message_id"].as_str().unwrap().into()).await.unwrap();
    let (first,later,first_marker,later_marker)=if a_source.event.seq<b_source.event.seq {
        (&a,&b,"idle-race-a","idle-race-b")
    } else {(&b,&a,"idle-race-b","idle-race-a")};
    assert!(store.session_input_claim(&session,&first["message_id"].as_str().unwrap().into()).await.unwrap().is_some());
    assert!(store.session_input_claim(&session,&later["message_id"].as_str().unwrap().into()).await.unwrap().is_none());
    let first_request=server.requests.lock().unwrap()[0]["messages"].to_string();
    assert!(first_request.contains(first_marker));assert!(!first_request.contains(later_marker));
    server.responses.add_permits(1);
    let first_claim=input_completed(&fixture,first).await;
    let later_claim=input_completed(&fixture,later).await;
    assert_ne!(first_claim.operation_id,later_claim.operation_id);
    assert_eq!(server.requests.lock().unwrap().len(),2);
    fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn input_queue_promoted_input_reaches_the_next_request_of_the_exact_running_turn() {
    let server=InputQueueModel::new(2).await;
    let fixture=idle_fixture("composer-promote-model",&server).await;
    let session=AgentSessionId::from(fixture.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let first=api(&fixture._router,&path,json!({"idempotency_key":"promote-root","input":{"content":"Acknowledge promote-root marker."}})).await;
    server.wait_for_requests(1).await;
    let root=fixture.owner.canonical().store().session_input_claim(&session,&first["message_id"].as_str().unwrap().into()).await.unwrap().unwrap();
    let supplement=api(&fixture._router,&path,json!({"idempotency_key":"promote-original","input":{"content":"Also acknowledge promoted-current marker."}})).await;
    let snapshot=queue(&fixture).await;
    let promoted=api(&fixture._router,&format!("/api/agent-sessions/{}/input-queue",session.as_ref()),json!({
        "idempotency_key":"promote-current","expected_version":snapshot["version"],"action":{"kind":"promote",
        "message_id":supplement["message_id"],"turn_id":root.source_message_id}})).await;
    let promoted_item=promoted["items"].as_array().unwrap().iter().find(|item|item["message_id"]==supplement["message_id"]).unwrap();
    assert_eq!(promoted_item["input_delivery"]["placement"],"steering");
    server.responses.add_permits(1);
    server.wait_for_requests(2).await;
    let delivered=fixture.owner.canonical().store().session_input_claim(&session,&supplement["message_id"].as_str().unwrap().into()).await.unwrap().unwrap();
    assert!(delivered.steering);assert_eq!(delivered.operation_id,root.operation_id);
    assert!(matches!(fixture.owner.session_turn_delivery_state(&fixture.host.options.user_id,session.as_ref(),"promote-original").await.unwrap(),
        nomifun_conversation::PublicTurnDeliveryState::Accepted {message_id} if message_id==supplement["message_id"].as_str().unwrap()),
        "an applied input retains its own keyed receipt while sharing the original running Turn");
    let requests=server.requests.lock().unwrap().clone();
    assert!(!requests[0]["messages"].to_string().contains("promoted-current"));
    assert!(requests[1]["messages"].to_string().contains("promoted-current"));
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=? AND source_message_id=?")
        .bind(session.as_ref()).bind(supplement["message_id"].as_str().unwrap()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(turns,0,"promoting the original identity must not mint a second task");
    fixture.finish().await;server.close().await;
}

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
async fn input_queue_preparation_stop_fences_the_source_and_joins_its_original_owner_before_release() {
    let f=Fixture::new("composer-preparation-stop").await;
    let session=AgentSessionId::from(f.host.options.conversation_id.clone());
    let owner=PrincipalRef {principal_kind:"user".into(),principal_id:f.host.options.user_id.clone()};
    f.owner.canonical().cancel_exact_turn(&owner,&session,"close-unexecuted-setup",&"cleanup-retry-turn".into()).await.unwrap();
    let mut inputs=Vec::new();
    for key in ["starting-source","retained-after-starting-stop"] {
        inputs.push(f.owner.canonical().accept_session_input(&owner,&session,key,json!({
            "content":key,"delivery_intent":{"kind":"queue"},"files":[],"inject_skills":[],
        })).await.unwrap().ack.unwrap().event_id);
    }
    let original=inputs.remove(0);let queued=inputs.remove(0);
    let business_gate=f.owner.hold_input_preparation_gate_for_test(&session).await;
    let preparing=f.owner.hold_input_preparation_for_test(&session,&original).unwrap();
    let router=f._router.clone();let path=format!("/api/agent-sessions/{}/turns/cancel",session.as_ref());
    let stop_body=json!({"idempotency_key":"stop-starting-input","expected_turn_id":original});
    let task_body=stop_body.clone();let stop_path=path.clone();
    let stop=tokio::spawn(async move {api(&router,&stop_path,task_body).await});
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let delivery=f.owner.canonical().store().session_input_delivery(&session,&original).await.unwrap();
            if delivery["state"]=="cancelled" && f.owner.input_preparation_observations(&session).unwrap()[0].1 {break;}
            tokio::task::yield_now().await;
        }
    }).await.expect("Stop must commit and signal the exact preparation without taking its business gate");
    assert!(!stop.is_finished(),"a ready canonical head is not proof the original preparation has exited");
    let (_,projection)=response(&f,"GET",&format!("/api/agent-sessions/{}/projection",session.as_ref()),Value::Null).await;
    assert_eq!(projection["data"]["runtime"]["is_processing"],true);
    assert!(projection["data"]["runtime"]["active_turn_id"].is_null(),"preparation is not a fabricated Turn");
    assert!(queue(&f).await["items"].as_array().unwrap().iter().any(|item|item["message_id"]==queued.as_ref()&&item["held"]==true));
    drop(preparing);
    drop(business_gate);
    let receipt=tokio::time::timeout(Duration::from_secs(5),stop).await.unwrap().unwrap();
    assert_eq!(receipt["input_message_id"],original.as_ref());assert!(receipt.get("target_operation_id").is_none());
    let (_,released)=response(&f,"GET",&format!("/api/agent-sessions/{}/projection",session.as_ref()),Value::Null).await;
    assert_eq!(released["data"]["runtime"]["is_processing"],false);
    let replay=api(&f._router,&path,stop_body).await;assert_eq!(replay["duplicate"],true);
    assert!(f.owner.canonical().store().session_input_claim(&session,&queued).await.unwrap().is_none());
    f.finish().await;
}

#[tokio::test]
async fn input_queue_stop_key_cannot_switch_from_a_claimed_turn_to_a_different_preparing_source() {
    let f=Fixture::new("composer-stop-cross-branch-key").await;
    let (_request,runtime)=prepare_live_steering(&f).await;
    let session=AgentSessionId::from(f.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/turns/cancel",session.as_ref());
    let stopped=api(&f._router,&path,json!({"idempotency_key":"one-stop-identity","expected_turn_id":f.message.source_message_id})).await;
    assert!(stopped.get("target_operation_id").is_some());
    let business_gate=f.owner.hold_input_preparation_gate_for_test(&session).await;
    let principal=PrincipalRef {principal_kind:"user".into(),principal_id:f.host.options.user_id.clone()};
    let fresh=f.owner.canonical().accept_session_input(&principal,&session,"later-preparing-source",json!({
        "content":"This is a different explicit input","delivery_intent":{"kind":"queue"},"files":[],"inject_skills":[],
    })).await.unwrap().ack.unwrap().event_id;
    let (status,_)=response(&f,"POST",&path,json!({"idempotency_key":"one-stop-identity","expected_turn_id":fresh})).await;
    assert!(status.is_client_error(),"a Stop key must have one source across both real response variants");
    assert_eq!(f.owner.canonical().store().session_input_delivery(&session,&fresh).await.unwrap()["state"],"pending");
    f.owner.canonical().store().mark_session_input_blocked(&session,&fresh,nomifun_agent_session::SessionInputBlockedReason::CancelledByOwner).await.unwrap();
    drop(business_gate);
    runtime.kill_and_wait(None).await.unwrap();f.finish().await;
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
async fn input_queue_pause_holds_queued_roots_without_stealing_prepared_steering_from_the_running_turn() {
    let f=Fixture::new("composer-pause-preparation-race").await;
    let (request,runtime)=prepare_live_steering(&f).await;
    let session=AgentSessionId::from(f.host.options.conversation_id.clone());
    let path=format!("/api/agent-sessions/{}/messages",session.as_ref());
    let saved=api(&f._router,&path,json!({"idempotency_key":"preparing-immediate",
        "delivery":{"kind":"prefer_current","turn_id":f.message.source_message_id},"input":{"content":"Wait for this original user decision"}})).await;
    let queued=api(&f._router,&path,json!({"idempotency_key":"queued-during-preparation",
        "input":{"content":"Keep this independent task queued"}})).await;
    let entered=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());
    INPUT_PRECLAIM_BARRIERS.get_or_init(Default::default).lock().unwrap().insert(session.as_ref().into(),(entered.clone(),release.clone()));
    let port=request.input_port.unwrap();let cause=request.model_request.causality;
    let waiter=tokio::spawn(async move {port.take(&cause,false).await});
    tokio::time::timeout(Duration::from_secs(5),entered.notified()).await.unwrap();
    let snapshot=queue(&f).await;
    api(&f._router,&format!("/api/agent-sessions/{}/input-queue",session.as_ref()),json!({
        "idempotency_key":"pause-preparing-input","expected_version":snapshot["version"],"action":{"kind":"pause"}})).await;
    release.notify_one();
    let delivered=tokio::time::timeout(Duration::from_secs(5),waiter).await.unwrap().unwrap().unwrap();
    assert_eq!(delivered.len(),1);
    assert_eq!(delivered[0].message_id,saved["message_id"].as_str().unwrap());
    assert_eq!(f.owner.canonical().store().head(&session).await.unwrap().status,"running");
    let claim=f.owner.canonical().store().session_input_claim(&session,&saved["message_id"].as_str().unwrap().into()).await.unwrap().unwrap();
    assert!(claim.steering);assert_eq!(claim.operation_id.as_ref(),"cleanup-retry-turn");
    assert!(f.owner.canonical().store().session_input_claim(&session,&queued["message_id"].as_str().unwrap().into()).await.unwrap().is_none());
    let snapshot=queue(&f).await;
    assert_eq!(snapshot["items"].as_array().unwrap().len(),1);
    assert_eq!(snapshot["items"][0]["message_id"],queued["message_id"]);
    assert_eq!(snapshot["items"][0]["held"],true);
    runtime.kill_and_wait(None).await.unwrap();f.finish().await;
}
