//! Opt-in Runtime/HTTP Stop over a real, naturally bounded native read.
//! The local Chat provider is scripted; no real-model, input-effect, held-key
//! cancellation, crash, or platform-wide cleanup claim is made.
use super::*;
use nomifun_agent_contracts::{AgentSessionId, OperationId, SessionEventRecord, digest_bytes};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::c_void,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::{Child, Command, Stdio},
    time::Instant,
};

type NativeCheck<T> = Result<T, String>;
type Handle = *mut c_void;
const BASELINE: &str = "runtime-stop-baseline";
const VERIFY: &str = "runtime-stop-verify";
const BLOCKED: &str = "runtime-stop-must-not-dispatch";
#[repr(C)]
#[derive(Default)]
struct FileTime { low: u32, high: u32 }
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn GetProcessId(process: Handle) -> u32;
    fn QueryFullProcessImageNameW(process: Handle, flags: u32, image: *mut u16, length: *mut u32) -> i32;
    fn GetProcessTimes(process: Handle, created: *mut FileTime, exited: *mut FileTime,
        kernel: *mut FileTime, user: *mut FileTime) -> i32;
    fn WaitForSingleObject(process: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
}
#[link(name = "user32")]
unsafe extern "system" {
    fn IsWindow(window: Handle) -> i32;
    fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
}
fn check(value: bool, reason: &str) -> NativeCheck<()> {
    if value { Ok(()) } else { Err(reason.into()) }
}
fn native_identity(handle: Handle, expected: &Path) -> NativeCheck<Value> {
    let mut image = vec![0u16; 32768];
    let mut length = image.len() as u32;
    let (mut created, mut exited, mut kernel, mut user) =
        (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
    check(unsafe { WaitForSingleObject(handle, 0) } == 258, "exact native handle is not live")?;
    check(unsafe { QueryFullProcessImageNameW(handle, 0, image.as_mut_ptr(), &mut length) } != 0
        && unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } != 0,
        "exact native process identity unavailable")?;
    let path = PathBuf::from(String::from_utf16(&image[..length as usize]).map_err(|_|"native image encoding")?)
        .canonicalize().map_err(|_|"native image path")?;
    check(path.to_string_lossy().eq_ignore_ascii_case(&expected.canonicalize().map_err(|_|"expected image path")?.to_string_lossy()),
        "native process image differs from the supplied binary")?;
    let pid = unsafe { GetProcessId(handle) };
    check(pid > 0, "native process PID missing")?;
    Ok(json!({"pid":pid,"image":path,"creation_filetime":((u64::from(created.high)<<32)|u64::from(created.low)).to_string()}))
}
struct OwnedFixture { root: PathBuf, child: Child }
impl Drop for OwnedFixture {
    fn drop(&mut self) { let _ = std::fs::write(self.root.join("close.request"), b"close only this fixture"); }
}
impl OwnedFixture {
    fn verify(&self, window: u64) -> NativeCheck<()> {
        let mut pid = 0;
        let window = window as usize as Handle;
        check(unsafe { WaitForSingleObject(self.child.as_raw_handle(), 0) } == 258
            && unsafe { IsWindow(window) } != 0
            && unsafe { GetWindowThreadProcessId(window, &mut pid) } != 0 && pid == self.child.id(),
            "exact fixture Child no longer owns the target HWND")
    }
    async fn close(&mut self) -> NativeCheck<Value> {
        std::fs::write(self.root.join("close.request"), b"close only this fixture").map_err(|_|"fixture close request")?;
        let exit = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(exit) = self.child.try_wait().map_err(|_|"exact Child wait")? { break Ok::<_,String>(exit); }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.map_err(|_|"exact Child did not exit normally; no forced stop attempted")??;
        let receipt: Value = serde_json::from_slice(&std::fs::read(self.root.join("exit.json")).map_err(|_|"fixture exit receipt")?)
            .map_err(|_|"fixture exit JSON")?;
        check(exit.success() && receipt["pid"] == self.child.id() && receipt["graceful"] == true && receipt["control_file"] == true,
            "normal fixture exit does not identify the retained Child")?;
        Ok(json!({"pid":self.child.id(),"exact_child_waited":true,"exit_code":exit.code(),"graceful":true,"control_file":true}))
    }
}
async fn canonical(
    store: &AgentSessionStore, sid: &AgentSessionId,
) -> NativeCheck<(Vec<SessionEventRecord>, BTreeMap<String, Value>)> {
    // TurnOutput intentionally ends at the first terminal. This owner test
    // instead reads bounded immutable IDs and resolves their existing payloads,
    // so it can inspect legitimate settlement after the cancellation fence.
    let page = store.read_events(sid, None, 1000).await.map_err(|_|"canonical event page")?;
    check(page.events.len() < 1000, "canonical page truncated")?;
    let ids = page.events.iter().map(|event|event.event_id.clone()).collect::<Vec<_>>();
    let payloads = store.resolved_event_payloads(sid, &ids).await.map_err(|_|"canonical payload resolution")?;
    Ok((page.events, payloads))
}
fn progress<'a>(
    events: &'a [SessionEventRecord], payloads: &'a BTreeMap<String,Value>, operation: &OperationId,
) -> Vec<(u64, &'a Value)> {
    events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"
        && event.producer_id.as_ref()=="runtime_supervisor" && event.correlation_id.as_ref()==operation.as_ref())
        .filter_map(|event|payloads.get(event.event_id.as_ref()).map(|payload|(event.seq,&payload["event"]))).collect()
}
fn verification(value: &Value) -> NativeCheck<Value> {
    let output = json_payload(value, "predicates").ok_or("native verification payload missing")?;
    let predicates = output["predicates"].as_array().ok_or("native verification predicates missing")?;
    check(output["status"]=="unsatisfied" && output["stable"]==false
        && output["elapsed_ms"].as_u64().is_some_and(|ms|ms>=10_000)
        && output["samples"].as_u64().is_some_and(|samples|samples>=2)
        && predicates.len()==1 && predicates[0]["status"]=="unsatisfied" && predicates[0]["unknown_reason"].is_null(),
        "cancelled/unknown/early return cannot stand in for the naturally expired native verification")?;
    let observed: Value = serde_json::from_str(predicates[0]["observed_json"].as_str().ok_or("native matched element missing")?)
        .map_err(|_|"native observed element JSON")?;
    check(observed["label"]=="Owned calculation" && observed["value"]=="0", "native verification did not observe the untouched control")?;
    Ok(output)
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
#[ignore="explicit real worker/owned WinForms paths and a fresh artifact directory; no input or screenshots"]
async fn exact_http_stop_fences_runtime_while_the_real_read_settles_normally() {
    let fixture_exe=PathBuf::from(std::env::var_os("NOMIFUN_COMPUTER_FIXTURE_EXE").expect("explicit owned fixture"));
    let worker=PathBuf::from(std::env::var_os("NOMIFUN_COMPUTER_WORKER").expect("explicit real worker"));
    let root=PathBuf::from(std::env::var_os("NOMIFUN_COMPUTER_ARTIFACT_DIR").expect("fresh artifact directory"));
    assert!(worker.is_absolute()&&worker.is_file()&&fixture_exe.is_absolute()&&fixture_exe.is_file()&&root.is_absolute());
    std::fs::create_dir(&root).expect("artifact directory must be fresh");
    let child=Command::new(&fixture_exe).arg("functional").arg(&root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let mut fixture=OwnedFixture {root:root.clone(),child};
    let config=AppConfig {data_dir:root.join("data"),work_dir:root.join("work"),
        auth_policy:AuthPolicy::TrustLocalToken,local_trust_secret:Some(TRUST.into()),..Default::default()};
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let database=nomifun_db::init_database(&config.database_path()).await.unwrap();
    let mut app=AppServices::from_config(database,&config).await.unwrap();
    let browser_created=Arc::new(AtomicUsize::new(0));
    app.browser_resources=Some(Arc::new(nomifun_browser_platform::workspace::BrowserResourceService::new(
        Arc::new(BindingOnlyBrowser(browser_created.clone())))));
    let router=create_router(&app).await;
    let store=AgentSessionStore::from_pool(app.database.pool().clone()).await.unwrap();
    let upstream=wiremock::MockServer::start().await;
    let requests=Arc::new(AtomicUsize::new(0));
    let stage=Arc::new(AtomicUsize::new(0));
    let unexpected=Arc::new(AtomicUsize::new(0));
    let searches=Arc::new(Mutex::new(BTreeSet::new()));
    let trace=Arc::new(Mutex::new(Vec::<Value>::new()));
    let mut witness:Option<OwnedHandle>=None;
    let mut stop_task:Option<tokio::task::JoinHandle<Value>>=None;
    let started=Instant::now();
    let mut report=json!({"ok":false,"evidence_kind":"scripted_chat_real_native_runtime_http_stop",
        "scope":"read-only Computer cancellation through HTTP, Runtime, Kernel, real worker and canonical owned settlement",
        "model_was_scripted":true,"real_model":false,"input_calls":0,"screenshots":0,"forced_stop":false,"automatic_replay":false,
        "root":root,"test_executable_sha256":digest(&std::env::current_exe().unwrap()),
        "worker_path":worker,"worker_sha256":digest(&worker),"fixture_path":fixture_exe,"fixture_sha256":digest(&fixture_exe),
        "test_source_sha256":digest_bytes(include_bytes!("native_computer_stop.rs")).as_ref(),
        "shared_test_helpers_sha256":digest_bytes(include_bytes!("native_computer_chain.rs")).as_ref(),
        "integration_target_sha256":digest_bytes(include_bytes!("../native_coding_reliability.rs")).as_ref()});
    let outcome=std::panic::AssertUnwindSafe(async {
        tokio::time::timeout(Duration::from_secs(20),async {
            loop {
                if root.join("ready.json").is_file(){break;}
                check(unsafe{WaitForSingleObject(fixture.child.as_raw_handle(),0)}==258,"fixture exited before ready")?;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<_,String>(())
        }).await.map_err(|_|"fixture ready deadline")??;
        let ready:Value=serde_json::from_slice(&std::fs::read(root.join("ready.json")).map_err(|_|"fixture ready read")?)
            .map_err(|_|"fixture ready JSON")?;
        let pid=fixture.child.id();
        let window=ready["window"].as_str().ok_or("fixture HWND")?.parse::<u64>().map_err(|_|"fixture HWND integer")?;
        let identity=native_identity(fixture.child.as_raw_handle(),&fixture_exe)?;
        check(ready["pid"]==pid&&ready["creation_filetime"]==identity["creation_filetime"],"fixture native creation identity mismatch")?;
        fixture.verify(window)?;
        report["fixture_identity"]=identity;report["window_id"]=json!(window);report["dpi"]=ready["dpi"].clone();
        let baseline_action=nomifun_computer::surface::tool("computer/get_window_state").unwrap().clone();
        let verify_action=nomifun_computer::surface::tool("computer/verify_state").unwrap().clone();
        let (seen,steps,invalid,searched,recorded)=(requests.clone(),stage.clone(),unexpected.clone(),searches.clone(),trace.clone());
        let ax=json!({"pid":pid,"window_id":window,"include_screenshot":false,"tree_format":"elements","max_elements":100});
        let impossible=format!("never-written-{}",uuid::Uuid::new_v4());
        let verify=json!({"pid":pid,"window_id":window,"include_screenshot":false,"timeout_ms":10000,"stable_samples":1,
            "expect":[{"element":{"selector":{"label_contains":"Owned calculation"},"value_equals":impossible}}]});
        let expected_verify=verify.clone();
        wiremock::Mock::given(wiremock::matchers::method("POST")).and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(move |request:&wiremock::Request| {
                let body:Value=serde_json::from_slice(&request.body).unwrap();
                let number=seen.fetch_add(1,Ordering::SeqCst)+1;
                let current=steps.load(Ordering::SeqCst);
                recorded.lock().unwrap().push(json!({"request":number,"native_stage":current,
                    "tools":body["tools"].as_array().map(|tools|tools.iter().map(|tool|tool["function"]["name"].clone()).collect::<Vec<_>>())}));
                if current>=2 {invalid.fetch_add(1,Ordering::SeqCst);return stream(None,"Unexpected automatic continuation after the exact user Stop.");}
                let selected=if current==0{&baseline_action}else{&verify_action};
                let tools=body["tools"].as_array().expect("scripted request must retain advertised tools");
                if !tools.iter().any(|tool|tool["function"]["name"]==selected.model_tool) {
                    assert!(tools.iter().any(|tool|tool["function"]["name"]=="ToolSearch"));
                    assert!(searched.lock().unwrap().insert(selected.action_id.clone()),"one schema discovery per native stage; never hide retries");
                    return stream(Some((if current==0{"find-runtime-stop-read"}else{"find-runtime-stop-verify"},"ToolSearch",
                        json!({"query":selected.action_id}))),"");
                }
                assert_eq!(body["parallel_tool_calls"],false,"Computer surface must be serial even if the provider supplies a batch");
                steps.fetch_add(1,Ordering::SeqCst);
                if current==0 {return emit(BASELINE,&baseline_action.action_id,ax.clone());}
                let baseline=observed(&body,BASELINE,"elements");
                assert_eq!(baseline["pid"],pid);assert_eq!(baseline["window_id"],window);
                assert!(baseline["elements"].as_array().unwrap().iter().any(|element|element["label"]=="Owned calculation"&&element["value"]=="0"));
                let calls=json!([
                    {"index":0,"id":VERIFY,"type":"function","function":{"name":verify_action.model_tool,"arguments":verify.to_string()}},
                    {"index":1,"id":BLOCKED,"type":"function","function":{"name":baseline_action.model_tool,"arguments":ax.to_string()}}
                ]);
                stream_delta(json!({"tool_calls":calls}),"tool_calls")
            }).mount(&upstream).await;
        let provider=call(&router,"POST","/api/providers",json!({"platform":"custom","name":"Owned native Runtime Stop scripted provider",
            "base_url":format!("{}/v1",upstream.uri()),"auth_scheme":"bearer","credentials":{"api_keys":["isolated-script-only"]},"enabled":true,
            "initial_model":{"model":"runtime-stop-script","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text",
                "connection_role":"default","input_constraints":{"image_input":false},"context_limit":128000,"output_limit":4096}]}})).await;
        let model=json!({"provider_id":provider["provider_id"],"model":"runtime-stop-script"});
        let editor=call(&router,"POST","/api/agent-presets/from-template/chat.minimal",
            json!({"reuse_existing":false,"display_name":"Owned native read-only Runtime Stop","model":model})).await;
        let preset=editor["preset"]["preset_id"].as_str().ok_or("preset identity")?;
        let mut draft=editor["draft"].clone();
        draft["document"]["enabled_capabilities"]=json!([{"capability":{"id":"computer"},"action_allowlist":[
            nomifun_computer::surface::tool("computer/get_window_state").unwrap().action_id,
            nomifun_computer::surface::tool("computer/verify_state").unwrap().action_id]}]);
        draft["document"]["skill_bindings"]=json!([]);
        call(&router,"POST",&format!("/api/agent-presets/{preset}/revisions"),json!({
            "expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"controlled native read-only Runtime Stop"})).await;
        let session=call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset,"model":model,
            "resource_selections":[{"resource_kind":"computer","resource_id":"local-desktop"}]})).await;
        let sid=AgentSessionId::from(session["agent_session_id"].as_str().ok_or("Session identity")?.to_owned());
        let accepted=call(&router,"POST",&format!("/api/agent-sessions/{}/turns",sid.as_ref()),json!({
            "idempotency_key":"owned-runtime-stop-read","input":{"content":format!("Use only the exact owned test window pid={pid}, window_id={window}. Read its AX state without a screenshot, then issue the authorized bounded read-only verification. Do not use input, screenshots, other apps or retries; the owner will Stop this Turn.")}})).await;
        let operation=OperationId::from(accepted["operation_id"].as_str().ok_or("Turn operation")?.to_owned());
        let expected_input=accepted["message_id"].as_str().ok_or("accepted input identity")?.to_owned();
        let (accepted_fact,_)=store.turn_accepted_input(&sid,&operation).await.map_err(|_|"canonical accepted input")?;
        check(accepted_fact.event_id.as_ref()==expected_input,"Stop target differs from the canonical original input")?;
        report["session_id"]=json!(sid);report["operation_id"]=json!(operation);report["accepted_input_id"]=json!(expected_input);
        let status=tokio::time::timeout(Duration::from_secs(30),async {
            loop {
                fixture.verify(window)?;
                let status=call(&router,"GET","/api/computer/status",Value::Null).await;
                let (events,payloads)=canonical(&store,&sid).await?;
                let progress=progress(&events,&payloads,&operation);
                let invoked=progress.iter().filter(|(_,event)|event["event"]=="tool_started"&&event["call_id"]==VERIFY
                    &&event["capability_id"]=="computer"&&event["action_id"]==verify_action_id()).count();
                let already_settled=progress.iter().any(|(_,event)|event["event"]=="host_tool_settled"&&event["call_id"]==VERIFY);
                check(!already_settled,"verification settled before the in-flight Stop boundary")?;
                if invoked==1&&status["phase"]=="busy"&&status["running"]==true&&stage.load(Ordering::SeqCst)==2 {break Ok::<_,String>(status);}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.map_err(|_|"real verification admission deadline")??;
        let worker_pid=status["process_id"].as_u64().and_then(|pid|u32::try_from(pid).ok()).ok_or("native worker PID")?;
        let generation=status["generation"].as_str().ok_or("native worker generation")?.to_owned();
        let raw=unsafe{OpenProcess(0x00101000,0,worker_pid)};
        check(!raw.is_null(),"native worker handle unavailable")?;
        let handle=unsafe{OwnedHandle::from_raw_handle(raw)};
        report["worker_identity"]=native_identity(handle.as_raw_handle(),&worker)?;witness=Some(handle);
        check(status["bound_sessions"]==1,"one exact native session must own the in-flight read")?;
        report["before_stop"]=status;report["stop_requested_ms"]=json!(started.elapsed().as_millis());
        let request_count=requests.load(Ordering::SeqCst);
        let stop_router=router.clone();let stop_path=format!("/api/agent-sessions/{}/turns/cancel",sid.as_ref());
        stop_task=Some(tokio::spawn(async move {call(&stop_router,"POST",&stop_path,
            json!({"idempotency_key":"stop-owned-original-verification","expected_turn_id":expected_input})).await}));
        let (fence_seq,waiting)=tokio::time::timeout(Duration::from_secs(3),async {
            loop {
                fixture.verify(window)?;
                let (events,payloads)=canonical(&store,&sid).await?;
                let fences=events.iter().filter(|event|event.kind.0=="turn/cancelled"&&event.correlation_id.as_ref()==operation.as_ref()).collect::<Vec<_>>();
                let settled=progress(&events,&payloads,&operation).iter().any(|(_,event)|event["event"]=="host_tool_settled"&&event["call_id"]==VERIFY);
                check(!settled,"native result finished before cancellation/waiting was observed")?;
                let status=call(&router,"GET","/api/computer/status",Value::Null).await;
                check(status["process_id"]==worker_pid&&status["generation"]==generation&&status["running"]==true,
                    "Stop replaced or killed the native worker")?;
                if fences.len()==1&&status["phase"]=="waiting_for_current_operation" {break Ok::<_,String>((fences[0].seq,status));}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.map_err(|_|"canonical cancellation and native waiting boundary")??;
        report["cancel_fence_seq"]=json!(fence_seq);report["waiting"]=waiting;report["fence_observed_ms"]=json!(started.elapsed().as_millis());
        let stop=stop_task.take().unwrap().await.map_err(|_|"HTTP Stop task failed")?;
        check(stop["status"]=="cancelled"&&stop["target_operation_id"]==operation.as_ref()&&stop["duplicate"]==false,
            "HTTP Stop receipt differs from the original exact Turn")?;
        report["http_stop_receipt"]=stop;
        let settled=tokio::time::timeout(Duration::from_secs(45),async {
            loop {
                let (events,payloads)=canonical(&store,&sid).await?;
                let progress=progress(&events,&payloads,&operation);
                let matching=progress.iter().filter(|(_,event)|event["event"]=="host_tool_settled"&&event["call_id"]==VERIFY).collect::<Vec<_>>();
                check(matching.len()<=1,"duplicate native verification settlement")?;
                if let Some((settled_seq,settlement))=matching.first().map(|entry|**entry) {
                    let native_operation=settlement["operation_id"].as_str().ok_or("native settlement operation missing")?;
                    let projections=events.iter().filter(|event|event.kind.0=="tool/result-recorded"
                        &&payloads.get(event.event_id.as_ref()).is_some_and(|value|value["call_id"]==VERIFY
                            &&value["operation_id"]==native_operation)).collect::<Vec<_>>();
                    check(projections.len()<=1,"duplicate native verification result projection")?;
                    if let Some(projected_event)=projections.first() {
                        // Native settlement and its result projection commit
                        // separately. Cancellation/lease retirement is not a
                        // barrier for this second immutable fact.
                        check(settled_seq>fence_seq&&projected_event.seq>settled_seq,
                            "native result/projection does not follow the cancellation fence")?;
                        let projected=&payloads[projected_event.event_id.as_ref()];
                        check(settlement["error"].is_null()&&settlement["result"]["call_id"]==VERIFY
                            &&settlement["result"]["is_error"]==false&&projected["error"].is_null()
                            &&projected["output"]["call_id"]==VERIFY&&projected["output"]["is_error"]==false,
                            "original native settlement/projection shape differs")?;
                        verification(&settlement["result"]["output"])?;
                        verification(&projected["output"]["output"])?;
                        break Ok::<_,String>((settled_seq,(*settlement).clone()));
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.map_err(|_|"original native result settlement deadline")??;
        check(settled.0>fence_seq&&settled.1["error"].is_null()&&settled.1["result"]["call_id"]==VERIFY
            &&settled.1["result"]["is_error"]==false,"Stop lost or replaced the real native result")?;
        let native=verification(&settled.1["result"]["output"])?;
        report["native_verification"]=native;report["native_settlement"]=json!({"seq":settled.0,"event":settled.1});
        report["native_settled_ms"]=json!(started.elapsed().as_millis());
        tokio::time::timeout(Duration::from_secs(15),async {
            loop {
                let execution=call(&router,"GET",&format!("/api/agent-sessions/{}/execution",sid.as_ref()),Value::Null).await;
                if execution["state"]=="cancelled"&&execution["producer_lease_live"]==false {break;}
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.map_err(|_|"cancelled Runtime teardown observation")?;
        let (events,payloads)=canonical(&store,&sid).await?;
        let records=progress(&events,&payloads,&operation);
        let native_starts=records.iter().filter(|(_,event)|event["event"]=="tool_started"&&event["capability_id"]=="computer").collect::<Vec<_>>();
        check(native_starts.len()==2&&native_starts.iter().any(|(_,event)|event["call_id"]==BASELINE)
            &&native_starts.iter().any(|(_,event)|event["call_id"]==VERIFY),"native read admission count differs from baseline plus original verification")?;
        let baseline_settled=records.iter().filter(|(_,event)|event["event"]=="host_tool_settled"&&event["call_id"]==BASELINE).collect::<Vec<_>>();
        check(baseline_settled.len()==1&&baseline_settled[0].1["error"].is_null()
            &&baseline_settled[0].1["result"]["is_error"]==false,"baseline did not retain one successful native result")?;
        let baseline_ax=json_payload(&baseline_settled[0].1["result"]["output"],"elements").ok_or("baseline native AX payload missing")?;
        check(baseline_ax["pid"]==pid&&baseline_ax["window_id"]==window
            &&baseline_ax["elements"].as_array().is_some_and(|elements|elements.iter().any(|element|element["label"]=="Owned calculation"&&element["value"]=="0")),
            "baseline canonical result did not observe the exact untouched fixture")?;
        let proposals=records.iter().filter(|(_,event)|event["event"]=="tool_call_completed"&&event["call"]["call_id"]==VERIFY).collect::<Vec<_>>();
        check(proposals.len()==1&&proposals[0].1["call"]["arguments"]==expected_verify,"verification was replayed or its exact arguments changed")?;
        let dispatches=records.iter().filter(|(_,event)|event["event"]=="host_tool_dispatch"&&event["dispatch"]["call_id"]==VERIFY).collect::<Vec<_>>();
        check(dispatches.len()==1&&dispatches[0].1["dispatch"]["operation_id"]==settled.1["operation_id"],"native result lacks one original dispatch owner")?;
        check(records.iter().filter(|(_,event)|event["event"]=="tool_started"&&event["call_id"]==VERIFY).count()==1,
            "original verification owner was invoked more than once")?;
        check(records.iter().filter(|(_,event)|event["event"]=="tool_call_completed"&&event["call"]["call_id"]==BLOCKED).count()==1
            && !records.iter().any(|(_,event)|(event["event"]=="tool_started"&&event["call_id"]==BLOCKED)
                ||(event["event"]=="host_tool_dispatch"&&event["dispatch"]["call_id"]==BLOCKED)),
            "a proposed next tool was admitted after the user's cancellation fence")?;
        check(records.iter().filter(|(_,event)|event["event"]=="tool_call_completed"
            &&[BASELINE,VERIFY,BLOCKED].iter().any(|id|event["call"]["call_id"]==*id)).count()==3,
            "the native model-call proposals were duplicated or lost")?;
        let terminals=events.iter().filter(|event|event.correlation_id.as_ref()==operation.as_ref()
            &&matches!(event.kind.0.as_str(),"turn/completed"|"turn/cancelled"|"turn/failed")).collect::<Vec<_>>();
        check(terminals.len()==1&&terminals[0].kind.0=="turn/cancelled"&&terminals[0].seq==fence_seq,
            "normal native completion replaced or duplicated the canonical cancellation fence")?;
        let projections=events.iter().filter(|event|event.kind.0=="tool/result-recorded"
            &&payloads.get(event.event_id.as_ref()).is_some_and(|value|value["call_id"]==VERIFY&&value["operation_id"]==settled.1["operation_id"])).collect::<Vec<_>>();
        check(projections.len()==1,"original native result projection missing or duplicated")?;
        let projected=&payloads[projections[0].event_id.as_ref()];
        check(projected["error"].is_null()&&projected["output"]["is_error"]==false,"original durable result projection contains an error")?;
        verification(&projected["output"]["output"])?;
        check(store.read_turn_receipt(&sid,&operation).await.map_err(|_|"cancelled Turn receipt")?.status==TurnReceiptStatus::Cancelled,
            "canonical Turn did not remain cancelled")?;
        check(store.list_effects(&sid).await.map_err(|_|"canonical effect inventory")?.is_empty(),"pure observation created an input Effect")?;
        check(requests.load(Ordering::SeqCst)==request_count&&unexpected.load(Ordering::SeqCst)==0&&stage.load(Ordering::SeqCst)==2,
            "Stop resumed or retried a model/native request")?;
        check(!events.iter().any(|event|event.seq>fence_seq&&matches!(event.kind.0.as_str(),"turn/started"|"context/model-visible-applied"|"tool/call-started")),
            "a new model/tool/Turn entered after the exact cancellation fence")?;
        fixture.verify(window)?;
        check(unsafe{WaitForSingleObject(witness.as_ref().unwrap().as_raw_handle(),0)}==258,"original native worker exited before ordinary cleanup")?;
        check(browser_created.load(Ordering::SeqCst)==0,"the Computer-only fixture opened a Browser runtime")?;
        report["counts"]=json!({"scripted_model_requests":request_count,"native_model_call_proposals":3,"native_dispatched_reads":2,
            "original_verification_dispatches":1,"native_results_retained_after_stop":1,"cancelled_turn_fences":1,
            "proposed_successor_dispatched":0,"input_effects":0,"new_user_turns":1});
        report["canonical_evidence"]=json!({"events":events,"resolved_payloads":payloads});
        Ok::<(),String>(())
    }).catch_unwind().await;
    if let Some(task)=stop_task.take(){let _=task.await;}
    let shutdown=app.shutdown_nomi_core_host().await;
    let status=call(&router,"GET","/api/computer/status",Value::Null).await;
    let worker_exit:NativeCheck<Value>=async {
        let handle=witness.as_ref().ok_or("original worker handle unavailable")?.as_raw_handle();
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                match unsafe{WaitForSingleObject(handle,0)} {
                    0=>break Ok::<_,String>(()),
                    258=>tokio::time::sleep(Duration::from_millis(20)).await,
                    _=>break Err("original worker handle wait failed".into()),
                }
            }
        }).await.map_err(|_|"original worker did not exit normally")??;
        let mut code=0;check(unsafe{GetExitCodeProcess(handle,&mut code)}!=0&&code==0,"original worker nonzero or unavailable exit code")?;
        Ok(json!({"original_retained_handle_exited":true,"exit_code":code}))
    }.await;
    let fixture_exit=fixture.close().await;
    report["cleanup"]=json!({"normal_app_shutdown":shutdown.is_ok(),"worker_running":status["running"],
        "worker":worker_exit.as_ref().ok(),"worker_error":worker_exit.as_ref().err(),
        "fixture":fixture_exit.as_ref().ok(),"fixture_error":fixture_exit.as_ref().err()});
    report["model_request_trace"]=json!(trace.lock().unwrap().clone());
    report["elapsed_ms"]=json!(started.elapsed().as_millis());
    let passed=matches!(&outcome,Ok(Ok(())))&&shutdown.is_ok()&&status["running"]==false&&worker_exit.is_ok()&&fixture_exit.is_ok();
    report["ok"]=json!(passed);
    if let Ok(Err(reason))=&outcome {report["failure"]=json!(reason);}
    if outcome.is_err(){report["failure"]=json!("test assertion failed; native owners were closed normally");}
    write(&root,"runtime-stop-report.json",&report);
    assert!(passed,"read-only native Runtime Stop did not pass; inspect runtime-stop-report.json");
}
fn verify_action_id() -> &'static str {
    // Frozen identity, with no duplicate tool name/schema definition.
    &nomifun_computer::surface::tool("computer/verify_state").unwrap().action_id
}
