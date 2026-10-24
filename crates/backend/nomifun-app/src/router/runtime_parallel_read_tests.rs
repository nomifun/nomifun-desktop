//! Exercise the installed HTTP/Host pipeline with a single parallel read burst.
use super::*;

const READS: usize = 8;

#[derive(Clone)]
struct AdmissionBarrier {
    remaining: usize,
    held: tokio_util::sync::CancellationToken,
    release: tokio_util::sync::CancellationToken,
    consuming: tokio_util::sync::CancellationToken,
}
static ADMISSION_BARRIERS: std::sync::OnceLock<Mutex<BTreeMap<String, AdmissionBarrier>>> = std::sync::OnceLock::new();

pub(in crate::router::unified_runtime_host) async fn admission_cutpoint(session: &str) {
    let barrier = {
        let mut barriers = ADMISSION_BARRIERS.get_or_init(Default::default).lock().unwrap();
        barriers.get_mut(session).and_then(|barrier| {
            if barrier.remaining == 0 { return None; }
            barrier.remaining -= 1;
            (barrier.remaining == 0).then(|| barrier.clone())
        })
    };
    if let Some(barrier) = barrier { barrier.held.cancel(); barrier.release.cancelled().await; }
}

pub(in crate::router::unified_runtime_host) fn result_consuming(session: &str, call_id: &str) {
    if !call_id.starts_with("parallel-read-") { return; }
    if let Some(barrier) = ADMISSION_BARRIERS.get_or_init(Default::default).lock().unwrap().get(session) {
        barrier.consuming.cancel();
    }
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn eight_parallel_native_reads_return_to_the_original_model_without_cleanup_or_cancellation() {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=format!("http://{}/v1",listener.local_addr().unwrap());
    let requests=Arc::new(Mutex::new(Vec::<Value>::new()));
    let observed=Arc::new(AtomicUsize::new(0));
    let router=axum::Router::new().route("/v1/chat/completions",axum::routing::post({
        let requests=requests.clone();let observed=observed.clone();
        move |axum::Json(body):axum::Json<Value>| {
            let requests=requests.clone();let observed=observed.clone();
            async move {
                let ordinal={let mut saved=requests.lock().unwrap();saved.push(body.clone());saved.len()};
                let (delta,reason)=if returned(&body,"parallel-read-0").is_some() {
                    for index in 0..READS {
                        let result=returned(&body,&format!("parallel-read-{index}")).expect("the next model sees every original read result");
                        assert_eq!(result["content"],format!("native-module-{index}"));
                    }
                    observed.store(READS,Ordering::SeqCst);
                    if available(&body,"update_plan") && returned(&body,"parallel-read-plan").is_none() {
                        call("update_plan","parallel-read-plan",json!({"explanation":"All eight original parallel reads returned their actual native contents.",
                            "plan":[{"step":"Read all eight native modules and confirm their contents","status":"completed"}],
                            "requirements":[{"id":"original","description":"Read all eight requested modules in one batch and confirm their actual results",
                                "source":{"input":0,"quote":"Read parallel-0.txt"}}]}))
                    } else if available(&body,"report_completion") {
                        let definition=body["tools"].as_array().unwrap().iter().find(|tool|tool["function"]["name"]=="report_completion").unwrap();
                        let properties=&definition["function"]["parameters"]["properties"];
                        let eligible=properties["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"].as_array().unwrap();
                        let ids=(0..READS).map(|index|format!("parallel-read-{index}")).collect::<Vec<_>>();
                        for id in &ids {assert!(eligible.contains(&json!(id)),"each original read is real eligible completion evidence");}
                        let mut report=json!({"summary":"Read all eight native modules: native-module-0, native-module-1, native-module-2, native-module-3, native-module-4, native-module-5, native-module-6, native-module-7.",
                            "criteria":[{"requirement_ids":["original"],"disposition":"supported","evidence_call_ids":ids,
                                "rationale":"All eight real read owners returned the exact requested module contents in the original batch."}]});
                        for field in ["observed_tool_error_count","observed_command_failure_count"] {
                            if !properties[field]["const"].is_null() {report[field]=properties[field]["const"].clone();}
                            else if let Some(value)=properties[field]["enum"].as_array().and_then(|values|values.first()) {report[field]=value.clone();}
                        }
                        call("report_completion",&format!("parallel-read-completion-{ordinal}"),report)
                    } else {
                        (json!({"role":"assistant","content":"Read all eight native modules from their original parallel calls."}),"stop")
                    }
                } else if !available(&body,"read_file") {
                    call("ToolSearch",&format!("parallel-read-discovery-{ordinal}"),json!({"query":"workspace.files/read"}))
                } else {
                    (json!({"role":"assistant","tool_calls":(0..READS).map(|index|json!({"index":index,
                        "id":format!("parallel-read-{index}"),"type":"function","function":{"name":"read_file",
                        "arguments":json!({"path":format!("parallel-{index}.txt")}).to_string()}})).collect::<Vec<_>>()}),"tool_calls")
                };
                let chunk=json!({"id":format!("parallel-read-{ordinal}"),"object":"chat.completion.chunk","created":1,"model":body["model"],
                    "choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                let end=json!({"id":format!("parallel-read-{ordinal}"),"object":"chat.completion.chunk","created":1,"model":body["model"],
                    "choices":[{"index":0,"delta":{},"finish_reason":reason}]});
                ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")).into_response()
            }
        }
    }));
    let server=tokio::spawn(async move {axum::serve(listener,router).await.unwrap();});
    let fixture=Fixture::with_input_model("native-eight-parallel-reads",&url).await;
    let (session,workspace)=create_parent(&fixture).await;
    for index in 0..READS {std::fs::write(workspace.join(format!("parallel-{index}.txt")),format!("native-module-{index}")).unwrap();}
    // Suspend a later admission after its real canonical commit, while its
    // original borrowed future still owns Host.active. The first native read
    // can return independently. Its result consumer must keep polling the
    // sibling admission in order to release that same lock.
    let barrier=AdmissionBarrier {remaining:2,held:tokio_util::sync::CancellationToken::new(),
        release:tokio_util::sync::CancellationToken::new(),consuming:tokio_util::sync::CancellationToken::new()};
    ADMISSION_BARRIERS.get_or_init(Default::default).lock().unwrap().insert(session.as_ref().into(),barrier.clone());
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({
        "idempotency_key":"parallel-eight-original","input":{"content":"Read parallel-0.txt through parallel-7.txt in one parallel batch, then confirm you received all eight native module contents."}})).await;
    let input=nomifun_agent_contracts::EventId::from(accepted["message_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(10),barrier.held.cancelled()).await.expect("the second original read admission holds its real Host lock");
    tokio::time::timeout(Duration::from_secs(10),barrier.consuming.cancelled()).await.expect("the first real owner returned and its original result reached the consumer");
    let before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_completed' AND json_extract(inline_json,'$.event.result.call_id') LIKE 'parallel-read-%'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(before,0,"the first result is waiting on the sibling's exact Host lock");
    barrier.release.cancel();
    let operation=tokio::time::timeout(Duration::from_secs(20),async {loop {
        if let Some(claim)=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap() {
            let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&claim.operation_id).await.unwrap();
            if receipt.status==nomifun_agent_session::TurnReceiptStatus::Completed {break claim.operation_id;}
            assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"parallel reads must return to their original model: {receipt:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("eight already-returned native reads must complete without invoking cleanup to wake the original caller");
    assert_eq!(observed.load(Ordering::SeqCst),READS);
    let records:Vec<String>=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' ORDER BY seq")
        .bind(session.as_ref()).fetch_all(fixture.pool()).await.unwrap();
    let records=records.iter().map(|raw|serde_json::from_str::<Value>(raw).unwrap()["event"].clone()).collect::<Vec<_>>();
    for index in 0..READS {
        let id=format!("parallel-read-{index}");
        for kind in ["tool_started","host_tool_settled","tool_completed"] {
            assert_eq!(records.iter().filter(|record|record["event"]==kind&&
                (record["call_id"]==id||record["result"]["call_id"]==id)).count(),1,"one original {kind} for {id}");
        }
    }
    assert!(records.iter().any(|record|record["event"]=="tool_results_ordered"&&record["call_ids"]==json!((0..READS).map(|index|format!("parallel-read-{index}")).collect::<Vec<_>>())));
    assert!(!records.iter().any(|record|record["event"]=="turn_cancelled"));
    assert_eq!(fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Completed);
    ADMISSION_BARRIERS.get_or_init(Default::default).lock().unwrap().remove(session.as_ref());
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;
    server.abort();let _=server.await;
}
