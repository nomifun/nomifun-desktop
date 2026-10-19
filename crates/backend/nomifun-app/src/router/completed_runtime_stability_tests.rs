//! Real App/SDK load: history stays open while completed transports retire.
use super::*;
use std::sync::atomic::{AtomicUsize,Ordering};
use tokio::sync::Semaphore;

static SQL_QUERY_PROBE:std::sync::OnceLock<Arc<AtomicUsize>>=std::sync::OnceLock::new();

// Opt-in measurement in a dedicated, serial test process. Count SQLx's
// completed-query events across its native worker threads without retaining
// SQL text, adding a product counter, or changing a connection/owner policy.
fn enable_sql_query_probe() {
    if std::env::var("NOMIFUN_LIFECYCLE_SQL_TRACE").as_deref()!=Ok("1") {return;}
    SQL_QUERY_PROBE.get_or_init(|| {
        use tracing_subscriber::{Layer as _,layer::SubscriberExt as _};
        struct QueryCount(Arc<AtomicUsize>);
        impl<S:tracing::Subscriber> tracing_subscriber::Layer<S> for QueryCount {
            fn on_event(&self,event:&tracing::Event<'_>,_:tracing_subscriber::layer::Context<'_,S>) {
                if event.metadata().target()=="sqlx::query" {self.0.fetch_add(1,Ordering::Relaxed);}
            }
        }
        let count=Arc::new(AtomicUsize::new(0));
        let subscriber=tracing_subscriber::registry().with(QueryCount(count.clone()).with_filter(
            tracing_subscriber::filter::Targets::new().with_target("sqlx::query",tracing::Level::DEBUG)));
        tracing::subscriber::set_global_default(subscriber).expect("SQL probe requires its own test process");
        count
    });
}

fn os_load_sample() -> Value {
    let pid = sysinfo::get_current_pid().unwrap();
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), true,
        sysinfo::ProcessRefreshKind::nothing().with_memory().with_cpu());
    let process = system.process(pid).unwrap();
    #[cfg(windows)]
    let native_handles = { let mut count = 0; unsafe {
        assert_ne!(windows_sys::Win32::System::Threading::GetProcessHandleCount(
            windows_sys::Win32::System::Threading::GetCurrentProcess(), &mut count), 0);
    } Some(count) };
    #[cfg(not(windows))]
    let native_handles: Option<u32> = None;
    let sqlx_completed_queries=SQL_QUERY_PROBE.get().map(|count|count.load(Ordering::Relaxed));
    if let Some(count)=sqlx_completed_queries {assert!(count>0,"enabled SQL probe must observe actual query completions");}
    json!({"rss_bytes":process.memory(),"cpu_ms":process.accumulated_cpu_time(),"native_handles":native_handles,
        "sqlx_completed_queries":sqlx_completed_queries})
}

struct ConcurrentModel {
    url:String,calls:Arc<AtomicUsize>,release_long:Arc<Semaphore>,task:tokio::task::JoinHandle<()>,
    file_reads:Arc<AtomicUsize>,
    file_searches:Arc<AtomicUsize>,
}

impl ConcurrentModel {
    async fn new()->Self {
        Self::build(false).await
    }
    async fn with_files()->Self {Self::build(true).await}
    async fn build(files:bool)->Self {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/v1",listener.local_addr().unwrap());
        let calls=Arc::new(AtomicUsize::new(0));let release_long=Arc::new(Semaphore::new(0));
        let file_reads=Arc::new(AtomicUsize::new(0));let reads=file_reads.clone();
        let file_searches=Arc::new(AtomicUsize::new(0));let searches=file_searches.clone();
        let counter=calls.clone();let gate=release_long.clone();
        let task=tokio::spawn(async move {
            let mut connections=tokio::task::JoinSet::new();
            loop {tokio::select! {
                accepted=listener.accept()=>{
                    let (mut socket,_)=accepted.unwrap();let counter=counter.clone();let gate=gate.clone();let reads=reads.clone();let searches=searches.clone();
                    connections.spawn(async move {
                        let mut bytes=Vec::new();let mut chunk=[0u8;4096];
                        let header_end=loop {
                            let count=socket.read(&mut chunk).await.unwrap();if count==0 {return;}
                            bytes.extend_from_slice(&chunk[..count]);
                            if let Some(index)=bytes.windows(4).position(|window|window==b"\r\n\r\n") {break index+4;}
                        };
                        let headers=String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                        let length=headers.lines().filter_map(|line|line.split_once(':')).find(|(key,_)|key.eq_ignore_ascii_case("content-length"))
                            .map(|(_,value)|value.trim().parse::<usize>().unwrap()).unwrap_or(0);
                        while bytes.len()<header_end+length {let count=socket.read(&mut chunk).await.unwrap();assert!(count>0);bytes.extend_from_slice(&chunk[..count]);}
                        if headers.starts_with("GET ") {
                            let body=r#"{"object":"list","data":[{"id":"cleanup-fixture","object":"model"}]}"#;
                            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                            let _=socket.write_all(response.as_bytes()).await;return;
                        }
                        let call=counter.fetch_add(1,Ordering::SeqCst)+1;
                        if call==1&&!files {gate.acquire().await.unwrap().forget();}
                        let (message,reason)=if files {
                            let request:Value=serde_json::from_slice(&bytes[header_end..header_end+length]).unwrap();
                            let result=request["messages"].as_array().unwrap().iter().rev()
                                .find(|message|message["role"]=="tool"&&message["tool_call_id"]=="steady-file-read");
                            if let Some(result)=result {
                                assert!(result["content"].to_string().contains("retirement-file-proof"),"the real original file result must reach the model");
                                reads.fetch_add(1,Ordering::SeqCst);gate.acquire().await.unwrap().forget();
                                (json!({"role":"assistant","content":"The original file was read once."}),"stop")
                            } else {
                                let available=request["tools"].as_array().unwrap().iter().any(|tool|tool["function"]["name"]=="read_file");
                                let (name,id,arguments)=if available {("read_file","steady-file-read",json!({"path":"steady-proof.txt"}))}
                                    else {searches.fetch_add(1,Ordering::SeqCst);("ToolSearch","steady-find-file-read",json!({"query":"workspace.files/read"}))};
                                (json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function",
                                    "function":{"name":name,"arguments":arguments.to_string()}}]}),"tool_calls")
                            }
                        } else {(json!({"role":"assistant","content":format!("The original task {call} completed once.")}),"stop")};
                        let delta=json!({"id":format!("steady-{call}"),"object":"chat.completion.chunk","created":1,"model":"cleanup-fixture",
                            "choices":[{"index":0,"delta":message,"finish_reason":null}]});
                        let finish=json!({"id":format!("steady-{call}"),"object":"chat.completion.chunk","created":1,"model":"cleanup-fixture",
                            "choices":[{"index":0,"delta":{},"finish_reason":reason}]});
                        let body=format!("data: {delta}\n\ndata: {finish}\n\ndata: [DONE]\n\n");
                        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                        let _=socket.write_all(response.as_bytes()).await;
                    });
                }
                done=connections.join_next(),if !connections.is_empty()=>{done.unwrap().unwrap();}
            }}
        });
        Self {url,calls,release_long,task,file_reads,file_searches}
    }
    async fn close(self) {self.task.abort();let _=self.task.await;}
}

async fn new_session(fixture:&Fixture)->String {
    let binding=fixture.owner.canonical().store().get_live_session(&fixture.host.options.conversation_id.clone().into()).await.unwrap().agent_binding;
    let model=fixture.host.options.model.as_ref().unwrap();
    api(&fixture._router,"/api/agent-sessions",json!({"preset_id":binding.preset_revision_ref.preset_id,
        "model":{"provider_id":model.provider_id,"model":model.use_model.as_deref().unwrap_or(&model.model)},
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}]})).await["agent_session_id"].as_str().unwrap().into()
}

async fn start(fixture:&Fixture,session:&str,key:&str)->String {
    api(&fixture._router,&format!("/api/agent-sessions/{session}/turns"),
        json!({"idempotency_key":key,"input":{"content":"Complete this original task once."}})).await["operation_id"].as_str().unwrap().into()
}

async fn wait_retired(fixture:&Fixture,session:&str,operation:&str)->Weak<ConversationRuntimeHost> {
    let host=tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            if let Some(host)=HOSTS.get().unwrap().lock().unwrap().get(session).cloned() {break host;}
            tokio::task::yield_now().await;
        }
    }).await.expect("the real factory must publish its original host witness");
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let progress=fixture.owner.canonical().store().read_turn_progress(&session.into(),&operation.into()).await.unwrap();
            let carrier=fixture.services.agent_runtime_sessions.observe_runtime_carrier(session);
            if progress.status==nomifun_agent_session::TurnReceiptStatus::Completed && !carrier.registered
                && !carrier.teardown_pending && carrier.active_generation.is_none() && carrier.constructing_generation.is_none()
                && host.strong_count()==0 {break;}
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.expect("completion must automatically retire the real slot, admission and host without closing history");
    assert!(fixture.owner.canonical().store().get_live_session(&session.into()).await.is_ok());
    host
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn completed_runtime_fixed_load_reaches_a_steady_baseline_with_a_legitimate_long_turn() {
    enable_sql_query_probe();
    let server=ConcurrentModel::new().await;
    let fixture=Fixture::with_input_model("completed-runtime-fixed-load",&server.url).await;
    // The shared fixture seeds an unexecuted cancellation test Turn. Remove
    // only that setup carrier before measuring API-driven workload below.
    let setup_session=fixture.host.options.conversation_id.clone();
    nomifun_agent_execution::AgentExecutionSessionPort::cancel_turn_for_execution(fixture.owner.as_ref(),&fixture.host.options.user_id,
        &setup_session,"steady-setup-stop","cleanup-retry-turn").await.unwrap();
    fixture.services.agent_runtime_sessions.retire_runtime_if_matches(&setup_session,&fixture.runtime).await.unwrap();
    assert_eq!(server.calls.load(Ordering::SeqCst),0);
    let long=new_session(&fixture).await;let long_operation=start(&fixture,&long,"steady-long").await;
    tokio::time::timeout(Duration::from_secs(15),async {while server.calls.load(Ordering::SeqCst)!=1 {tokio::task::yield_now().await;}}).await.unwrap();
    let original_long=fixture.services.agent_runtime_sessions.observe_runtime_carrier(&long);
    assert!(original_long.registered&&original_long.active_generation.is_some());
    let active_baseline=fixture.services.background_tasks.lifecycle_test_counts().0;
    let mut hosts=Vec::new();let mut history=Vec::new();let mut retained_counts=Vec::new();
    eprintln!("LIFECYCLE_LOAD runtime baseline {}", os_load_sample());
    for round in 0..3 {
        for index in 0..3 {
            let session=new_session(&fixture).await;
            let operation=start(&fixture,&session,&format!("steady-{round}-{index}")).await;
            hosts.push(wait_retired(&fixture,&session,&operation).await);history.push((session,operation));
        }
        tokio::time::timeout(Duration::from_secs(15),async {while fixture.services.background_tasks.lifecycle_test_counts().0>active_baseline {tokio::task::yield_now().await;}}).await
            .expect("finished per-Turn observers and retirement waiters must stop accumulating");
        retained_counts.push(fixture.services.background_tasks.lifecycle_test_counts().1);
        let remaining=fixture.services.agent_runtime_sessions.registered_runtime_sessions().unwrap();
        assert_eq!(remaining,vec![long.clone()]);
        let current_long=fixture.services.agent_runtime_sessions.observe_runtime_carrier(&long);
        assert_eq!(current_long.instance_id,original_long.instance_id);assert_eq!(current_long.active_generation,original_long.active_generation);
        assert_eq!(fixture.owner.canonical().store().head(&long.clone().into()).await.unwrap().active_turn_id.as_deref(),Some(long_operation.as_str()));
        assert!(hosts.iter().all(|host|host.strong_count()==0));
        assert_eq!(server.calls.load(Ordering::SeqCst),1+history.len(),"retirement cannot restart old model work");
        eprintln!("LIFECYCLE_LOAD runtime round={round} os={} registered=1 disposed_hosts={} tasks={:?} model_calls={}",
            os_load_sample(), hosts.len(), fixture.services.background_tasks.lifecycle_test_counts(), server.calls.load(Ordering::SeqCst));
    }
    assert!(retained_counts[2]<=retained_counts[0]+2,"fixed-load retained task handles grew per round: {retained_counts:?}");
    let calls=server.calls.load(Ordering::SeqCst);
    for (session,operation) in &history {
        let view=fixture.owner.canonical().store().turn_history_for_operation(&PrincipalRef {principal_kind:"user".into(),principal_id:fixture.host.options.user_id.clone()},
            &session.as_str().into(),&operation.as_str().into()).await.unwrap().unwrap();
        assert_eq!(view.projection["state"],"completed");
    }
    assert_eq!(server.calls.load(Ordering::SeqCst),calls,"history reload has no model/Runtime admission authority");
    server.release_long.add_permits(1);hosts.push(wait_retired(&fixture,&long,&long_operation).await);
    assert!(fixture.services.agent_runtime_sessions.registered_runtime_sessions().unwrap().is_empty());
    assert_eq!(server.calls.load(Ordering::SeqCst),10);assert!(hosts.iter().all(|host|host.strong_count()==0));
    eprintln!("LIFECYCLE_LOAD runtime settled {} registered=0 disposed_hosts=10", os_load_sample());
    fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn autowork_source_cancelled_recovery_stops_the_real_root_without_model_or_attempt_replay() {
    use nomifun_db::{IAgentExecutionRepository,IRequirementRepository};
    use nomifun_agent_execution::{AgentExecutionAutomationPort,AutomationExecutionRequest,AutomationExecutionSource};
    let server=ConcurrentModel::new().await;
    let fixture=Fixture::with_input_model("autowork-source-cancelled-recovery",&server.url).await;
    let owner=fixture.host.options.user_id.clone();let setup=fixture.host.options.conversation_id.clone();
    nomifun_agent_execution::AgentExecutionSessionPort::cancel_turn_for_execution(fixture.owner.as_ref(),&owner,&setup,
        "source-recovery-setup-stop","cleanup-retry-turn").await.unwrap();
    fixture.services.agent_runtime_sessions.retire_runtime_if_matches(&setup,&fixture.runtime).await.unwrap();
    let lead=new_session(&fixture).await;
    fixture.services.requirement_service.create(nomifun_api_types::CreateRequirementRequest {title:"Original source Stop".into(),
        content:"Keep the existing task identity".into(),tag:"restart-source".into(),order_key:None,status:None,created_by:None,attachments:vec![]}).await.unwrap();
    let requirements=nomifun_db::SqliteRequirementRepository::new(fixture.pool().clone());
    let claim=requirements.claim_next_for_runner("restart-source",Some(&lead),None,30_000,nomifun_common::now_ms()).await.unwrap().unwrap().row;
    let projection=fixture.owner.get_session(&owner,&lead).await.unwrap();
    let source=AutomationExecutionSource {requirement_id:claim.requirement_id.clone(),claim_generation:claim.claim_generation,
        operation_id:format!("autowork:source-recovery:{}",claim.claim_generation)};
    let admission=fixture.voice_execution.admit_automation(&owner,AutomationExecutionRequest {source:source.clone(),lead_session_id:lead.clone(),
        goal:"Complete the original task once.".into(),workspace:Some(projection.extra["workspace"].as_str().unwrap().into())}).await.unwrap();
    tokio::time::timeout(Duration::from_secs(15),async {while server.calls.load(Ordering::SeqCst)!=1 {tokio::task::yield_now().await;}}).await
        .expect("the original real Engine root must be admitted before the source Stop race");
    let before=fixture.voice_execution.get(&owner,&admission.execution_id).await.unwrap();
    assert_eq!(before.attempts.len(),1);let attempt_id=before.attempts[0].attempt_id.clone();
    let original=fixture.owner.canonical().store().head(&lead.clone().into()).await.unwrap().active_turn_id.unwrap();
    assert_eq!(original,format!("turn:user:{owner}:{lead}:{attempt_id}:initial-turn"));
    // Simulate the persisted half of Stop immediately before process loss:
    // no cancel_automation API/scheduler signal has been called yet.
    requirements.resolve_claim_exact(&claim.requirement_id,claim.claim_generation,claim.claim_token.as_deref().unwrap(),Some(&lead),None,
        &nomifun_db::RequirementClaimResolution::Cancelled {completion_note:Some("user stopped the original source".into())},nomifun_common::now_ms()).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(15),fixture.voice_execution.recover()).await.unwrap().unwrap();
    let repository=nomifun_db::SqliteAgentExecutionRepository::new(fixture.pool().clone());
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let receipt=fixture.owner.canonical().store().read_turn_receipt(&lead.clone().into(),&original.as_str().into()).await.unwrap();
            let carrier=fixture.services.agent_runtime_sessions.observe_runtime_carrier(&lead);
            if receipt.status==nomifun_agent_session::TurnReceiptStatus::Cancelled&&!carrier.registered&&!carrier.teardown_pending
                &&carrier.active_generation.is_none(){break;}
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.expect("startup recovery must send exact native Stop to the original automation root");
    let stopped=fixture.voice_execution.get(&owner,&admission.execution_id).await.unwrap();
    assert_eq!(stopped.execution.status,nomifun_common::AgentExecutionStatus::Cancelled);
    assert_eq!(stopped.attempts.len(),1);assert_eq!(stopped.attempts[0].attempt_id,attempt_id);
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            if repository.list_pending_conversation_cleanups(Some(&admission.execution_id),None,100).await.unwrap().cleanups.is_empty() {break;}
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }).await.expect("the original exact cleanup worker must acknowledge its native Stop after the receipt converges");
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(&lead).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(count,1);assert_eq!(server.calls.load(Ordering::SeqCst),1);
    fixture.voice_execution.recover().await.unwrap();
    let repeated=fixture.voice_execution.get(&owner,&admission.execution_id).await.unwrap();
    assert_eq!(repeated.attempts.len(),1);assert_eq!(repeated.execution.event_sequence,stopped.execution.event_sequence);
    assert_eq!(requirements.get_by_requirement_id(&claim.requirement_id).await.unwrap().unwrap().attempt_count,claim.attempt_count);
    assert_eq!(server.calls.load(Ordering::SeqCst),1);assert!(fixture.owner.canonical().store().get_live_session(&lead.clone().into()).await.is_ok());
    server.release_long.add_permits(1);fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn completed_workspace_file_calls_retire_exact_host_caches_without_closing_history() {
    enable_sql_query_probe();
    let server=ConcurrentModel::with_files().await;
    let fixture=Fixture::with_input_model("completed-workspace-cache-fixed-load",&server.url).await;
    let setup=fixture.host.options.conversation_id.clone();
    nomifun_agent_execution::AgentExecutionSessionPort::cancel_turn_for_execution(fixture.owner.as_ref(),&fixture.host.options.user_id,
        &setup,"workspace-cache-setup-stop","cleanup-retry-turn").await.unwrap();
    fixture.services.agent_runtime_sessions.retire_runtime_if_matches(&setup,&fixture.runtime).await.unwrap();
    let baseline=fixture.host.session_host.cached_workspace_host_count();
    eprintln!("LIFECYCLE_LOAD files baseline {} cache={baseline}", os_load_sample());
    let mut hosts=Vec::new();let mut files=Vec::new();
    for round in 0..3 {
        for index in 0..2 {
            let ordinal=round*2+index+1;let session=new_session(&fixture).await;
            let projection=fixture.owner.get_session(&fixture.host.options.user_id,&session).await.unwrap();
            let root=std::path::PathBuf::from(projection.extra["workspace"].as_str().unwrap());
            std::fs::create_dir_all(&root).unwrap();let file=root.join("steady-proof.txt");
            std::fs::write(&file,"retirement-file-proof: preserve these user bytes after native/cache retirement").unwrap();
            let operation=api(&fixture._router,&format!("/api/agent-sessions/{session}/turns"),json!({"idempotency_key":format!("workspace-cache-{round}-{index}"),
                "input":{"content":"Read the file steady-proof.txt once and answer from its content."}})).await["operation_id"].as_str().unwrap().to_owned();
            tokio::time::timeout(Duration::from_secs(15),async {while server.file_reads.load(Ordering::SeqCst)<ordinal {tokio::task::yield_now().await;}}).await
                .expect("the actual read_file result must be consumed before completion");
            assert_eq!(fixture.host.session_host.cached_workspace_host_count(),baseline+1,
                "a live original file invocation materializes its real workspace owner");
            server.release_long.add_permits(1);hosts.push(wait_retired(&fixture,&session,&operation).await);
            tokio::time::timeout(Duration::from_secs(15),async {while fixture.host.session_host.cached_workspace_host_count()!=baseline {tokio::task::yield_now().await;}}).await
                .expect("native retirement must also retire its exact idle file/artifact/VCS owner cache");
            let calls:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events result JOIN agent_events started ON started.event_id=result.causation_event_id \
                WHERE result.session_id=? AND result.kind='tool/result-recorded' AND started.kind='tool/call-started' \
                  AND json_extract(started.inline_json,'$.action_id')='workspace.files/read'")
                .bind(&session).fetch_one(fixture.pool()).await.unwrap();assert_eq!(calls,1,"cache cleanup cannot execute the file tool again");
            assert!(fixture.owner.canonical().store().get_live_session(&session.into()).await.is_ok());
            assert!(std::fs::read_to_string(&file).unwrap().contains("retirement-file-proof"));files.push(file);
        }
        assert_eq!(fixture.host.session_host.cached_workspace_host_count(),baseline);
        assert!(fixture.services.agent_runtime_sessions.registered_runtime_sessions().unwrap().is_empty());
        assert!(hosts.iter().all(|host|host.strong_count()==0));
        eprintln!("LIFECYCLE_LOAD files round={round} os={} cache={baseline} disposed_hosts={} model_calls={} file_reads={}",
            os_load_sample(), hosts.len(), server.calls.load(Ordering::SeqCst), server.file_reads.load(Ordering::SeqCst));
    }
    assert_eq!(server.file_reads.load(Ordering::SeqCst),6);
    let discoveries=server.file_searches.load(Ordering::SeqCst);assert!(discoveries<=6);
    assert_eq!(server.calls.load(Ordering::SeqCst),12+discoveries,"only the original read/result steps and applicable formal tool discovery may run");
    assert!(files.iter().all(|file|file.is_file()));fixture.finish().await;server.close().await;
}
