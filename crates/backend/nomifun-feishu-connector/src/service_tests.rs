//! Offline service integration: real SQLite and the actual CLI adapter/runtime,
//! with a deterministic executor that never starts a program or contacts Feishu.
use super::*;
use async_trait::async_trait;
use crate::runtime::{CliCommand,CliExecutor,CliObserver,CliOutput};
use serde_json::{Value,json};
use std::{path::{Path,PathBuf},sync::atomic::{AtomicBool,AtomicUsize,Ordering},time::Duration};
use tokio::{sync::Semaphore,task::JoinHandle};

#[derive(Clone)]
struct RecordedCall { args:Vec<String>,mutating:bool,input:Option<Value> }

struct FixtureExecutor {
    calls:Mutex<Vec<RecordedCall>>,
    scopes:Mutex<Vec<String>>,
    users:Mutex<Vec<FeishuActor>>,
    requested_scopes:Mutex<Vec<String>>,
    document_creates:AtomicUsize,
    logouts:AtomicUsize,
    base_batches:AtomicUsize,
    timeout_document:AtomicBool,
    document_deletes:AtomicUsize,
    document_pages:AtomicBool,
    old_token_expired:AtomicBool,
    timeout_create:AtomicBool,
    hold_create:AtomicBool,
    create_entered:Semaphore,
    create_release:Semaphore,
    create_returned:Semaphore,
    hold_auth_cleanup:AtomicBool,
    auth_poll_entered:Semaphore,
    auth_cancel_seen:Semaphore,
    auth_cleanup_release:Semaphore,
    hold_preflight:AtomicBool,
    preflight_entered:Semaphore,
    preflight_release:Semaphore,
    confirmed_records:Mutex<HashSet<String>>,
}
impl FixtureExecutor {
    fn new()->Self{
        Self{calls:Mutex::new(Vec::new()),scopes:Mutex::new(vec![
            "search:docs:read".into(),"base:record:read".into(),"base:field:read".into()]),
            requested_scopes:Mutex::new(Vec::new()),
            users:Mutex::new(vec![FeishuActor{app_id:"cli_fixture".into(),user_id:"ou_owner".into(),name:"Owner".into()}]),
            document_creates:AtomicUsize::new(0),logouts:AtomicUsize::new(0),base_batches:AtomicUsize::new(0),old_token_expired:AtomicBool::new(false),
            timeout_document:AtomicBool::new(false),document_deletes:AtomicUsize::new(0),document_pages:AtomicBool::new(false),
            timeout_create:AtomicBool::new(false),hold_create:AtomicBool::new(false),
            create_entered:Semaphore::new(0),create_release:Semaphore::new(0),create_returned:Semaphore::new(0),
            hold_auth_cleanup:AtomicBool::new(false),auth_poll_entered:Semaphore::new(0),
            auth_cancel_seen:Semaphore::new(0),auth_cleanup_release:Semaphore::new(0),
            hold_preflight:AtomicBool::new(false),preflight_entered:Semaphore::new(0),preflight_release:Semaphore::new(0),
            confirmed_records:Mutex::new(HashSet::new())}
    }
    fn grant(&self,scope:&str){let mut scopes=lock(&self.scopes);if !scopes.iter().any(|s|s==scope){scopes.push(scope.into())}}
    fn calls(&self)->Vec<RecordedCall>{lock(&self.calls).clone()}
    fn envelope(data:Value)->CliOutput{CliOutput::fixture(0,json!({"ok":true,"identity":"user","data":data}).to_string(),"")}
    fn matrix(&self,ids:Vec<String>)->CliOutput{
        let confirmed=lock(&self.confirmed_records);
        let rows=ids.iter().map(|id|json!([[if confirmed.contains(id){"Contacted"}else{"New"}]])).collect::<Vec<_>>();
        Self::envelope(json!({"fields":["Status"],"field_id_list":["fld_status"],"field_type_list":["select"],
            "record_id_list":ids,"data":rows,"has_more":false}))
    }
}
fn contains(args:&[String],pair:&[&str])->bool{
    args.windows(pair.len()).any(|w|w.iter().map(String::as_str).eq(pair.iter().copied()))
}
#[async_trait]
impl CliExecutor for FixtureExecutor {
    async fn execute(&self,_executable:&Path,command:CliCommand,cancel:CancellationToken,_observer:Option<CliObserver>)->Result<CliOutput,FeishuConnectorError>{
        let args=command.args.clone();
        let input=command.input_file.as_ref().or(command.stdin.as_ref())
            .map(|bytes|serde_json::from_slice(bytes).expect("adapter writes valid JSON"));
        lock(&self.calls).push(RecordedCall{args:args.clone(),mutating:command.mutating,input});
        if args==["--version"]{return Ok(CliOutput::fixture(0,"lark-cli 1.0.98",""))}
        if args==["--help"]{return Ok(CliOutput::fixture(0,"auth docs task base calendar",""))}
        if args.first().map(String::as_str)==Some("schema"){
            return Ok(CliOutput::fixture(0,r#"{"inputSchema":{},"_meta":{}}"#,""))
        }
        if contains(&args,&["profile","list"]){
            return Ok(CliOutput::fixture(0,r#"[{"name":"work","appId":"cli_fixture","brand":"feishu","user":"Owner"}]"#,""))
        }
        if contains(&args,&["update","--check","--json"]){
            assert!(!command.mutating);
            return Ok(CliOutput::fixture(0,json!({"ok":true,"current_version":"1.0.98",
                "latest_version":"1.0.99","action":"update_available"}).to_string(),""))
        }
        if contains(&args,&["drive","+search"]){
            assert!(!command.mutating);
            let entry=|id:&str,wiki:bool,kind:&str|json!({"entity_type":if wiki{"WIKI"}else{"DOC"},"title_highlighted":"Fixture document",
                "summary_highlighted":"Not a body preview","result_meta":{"token":id,"doc_types":kind,
                    "url":format!("https://example.feishu.cn/{}/{}",if wiki{"wiki"}else if kind=="BITABLE"{"base"}else{"docx"},id),"update_time":"1793000000"}});
            if contains(&args,&["--doc-types","bitable"]){
                return Ok(Self::envelope(json!({"results":[entry("base_real",false,"BITABLE")],"has_more":false})))
            }
            if self.document_pages.load(Ordering::SeqCst){
                let next=args.windows(2).find(|pair|pair[0]=="--page-token");
                let entries=if next.is_none(){(0..20).map(|n|entry(&format!("wiki_{n}"),true,"DOCX")).collect::<Vec<_>>()}
                    else{assert_eq!(next.unwrap()[1],"documents-next");(20..25).map(|n|entry(&format!("resource_{n}"),n<23,"DOCX")).collect()};
                return Ok(Self::envelope(json!({"results":entries,"has_more":next.is_none(),"page_token":"documents-next"})))
            }
            return Ok(Self::envelope(json!({"results":[entry("doc_real",false,"DOCX")],"has_more":false})))
        }
        if contains(&args,&["drive","+delete"]){
            assert!(command.mutating);self.document_deletes.fetch_add(1,Ordering::SeqCst);
            if self.timeout_document.load(Ordering::SeqCst){return Ok(CliOutput{started:true,timed_out:true,..Default::default()})}
            return Ok(Self::envelope(json!({"deleted":true,"file_token":"doc_real","type":"docx"})))
        }
        if contains(&args,&["docs","+create"]){
            assert!(command.mutating);assert!(command.stdin.is_none());
            self.document_creates.fetch_add(1,Ordering::SeqCst);
            if self.hold_create.load(Ordering::SeqCst){
                self.create_entered.add_permits(1);
                tokio::select!{
                    _=cancel.cancelled()=>return Ok(CliOutput{started:true,cancelled:true,..Default::default()}),
                    permit=self.create_release.acquire()=>permit.unwrap().forget(),
                }
            }
            if self.timeout_create.load(Ordering::SeqCst){
                return Ok(CliOutput{started:true,timed_out:true,..Default::default()})
            }
            self.create_returned.add_permits(1);
            return Ok(Self::envelope(json!({"document":{"document_id":"doc_real","url":"https://example.feishu.cn/docx/doc_real"}})))
        }
        if contains(&args,&["auth","status"]){
            if self.old_token_expired.load(Ordering::SeqCst){
                return Ok(CliOutput::fixture(1,"",json!({"ok":false,"error":{"type":"authentication","subtype":"token_expired","message":"Expired fixture token"}}).to_string()))
            }
            if lock(&self.users).is_empty(){
                return Ok(CliOutput::fixture(0,json!({"appId":"cli_fixture","identity":"user",
                    "identities":{"user":{"available":false,"status":"not_logged_in"}}}).to_string(),""))
            }
            return Ok(CliOutput::fixture(0,json!({"appId":"cli_fixture","identity":"user","verified":true,
                "identities":{"user":{"available":true,"status":"ready","verified":true,
                    "openId":"ou_owner","userName":"Owner","scope":lock(&self.scopes).clone()}}}).to_string(),""))
        }
        if contains(&args,&["auth","list"]){
            assert_eq!(args,["--profile","work","auth","list","--json"]);
            assert!(!command.mutating);
            let users=lock(&self.users).iter().map(|user|json!({"appId":user.app_id,"userOpenId":user.user_id,
                "userName":user.name,"tokenStatus":"valid"})).collect::<Vec<_>>();
            return Ok(CliOutput::fixture(0,serde_json::to_string(&users).unwrap(),""))
        }
        if contains(&args,&["auth","logout"]){
            assert_eq!(args,["--profile","work","auth","logout","--json"]);
            assert!(command.mutating);self.logouts.fetch_add(1,Ordering::SeqCst);
            lock(&self.users).clear();self.old_token_expired.store(false,Ordering::SeqCst);
            return Ok(CliOutput::fixture(0,r#"{"ok":true,"loggedOut":true}"#,""))
        }
        if contains(&args,&["auth","login"]){
            assert!(!command.mutating);
            if args.iter().any(|arg|arg=="--no-wait"){
                *lock(&self.requested_scopes)=args.windows(2).find(|pair|pair[0]=="--scope")
                    .map(|pair|pair[1].split_whitespace().map(str::to_owned).collect()).unwrap_or_default();
                return Ok(CliOutput::fixture(0,json!({"verification_url":"https://accounts.feishu.cn/device?user_code=test-only",
                    "device_code":"test-only-device-code","expires_in":600}).to_string(),""))
            }
            assert!(args.iter().any(|arg|arg=="--device-code"));
            self.auth_poll_entered.add_permits(1);
            if self.hold_auth_cleanup.load(Ordering::SeqCst){
                cancel.cancelled().await;
                self.auth_cancel_seen.add_permits(1);
                self.auth_cleanup_release.acquire().await.unwrap().forget();
                return Ok(CliOutput{started:true,cancelled:true,..Default::default()})
            }
            let granted=lock(&self.requested_scopes).clone();for scope in &granted{self.grant(scope)}
            *lock(&self.users)=vec![FeishuActor{app_id:"cli_fixture".into(),user_id:"ou_owner".into(),name:"Owner".into()}];
            self.old_token_expired.store(false,Ordering::SeqCst);
            return Ok(CliOutput::fixture(0,json!({"event":"authorization_complete","user_open_id":"ou_owner",
                "user_name":"Owner","scope":granted,"missing":[]}).to_string(),""))
        }
        if contains(&args,&["base","+field-list"]){
            return Ok(Self::envelope(json!({"fields":[{"id":"fld_status","name":"Status","type":"select",
                "options":["New","Contacted"]}]})))
        }
        if contains(&args,&["base","+record-list"]){
            if self.hold_preflight.load(Ordering::SeqCst){
                self.preflight_entered.add_permits(1);
                tokio::select!{
                    _=cancel.cancelled()=>return Ok(CliOutput{started:true,cancelled:true,..Default::default()}),
                    permit=self.preflight_release.acquire()=>permit.unwrap().forget(),
                }
            }
            return Ok(self.matrix(vec!["rec_one".into(),"rec_two".into(),"rec_three".into()]))
        }
        if contains(&args,&["base","+record-get"]){
            let ids=args.windows(2).filter(|w|w[0]=="--record-id").map(|w|w[1].clone()).collect();
            return Ok(self.matrix(ids))
        }
        if contains(&args,&["base","+record-batch-update"]){
            assert!(command.mutating);self.base_batches.fetch_add(1,Ordering::SeqCst);
            return Ok(Self::envelope(json!({"ignored_fields":[]})))
        }
        // A fixture must fail loudly if discovery installs, authorization starts,
        // reconciliation searches by name, or any new command is introduced.
        panic!("Unexpected offline test command: {}",args.join(" "));
    }
}

struct Fixture {
    _directory:tempfile::TempDir,
    database:nomifun_db::Database,
    service:Arc<FeishuConnectorService>,
    executor:Arc<FixtureExecutor>,
    workers:Arc<Mutex<Vec<JoinHandle<()>>>>,
    shutdown:CancellationToken,
    shared_root:PathBuf,
}
impl Fixture {
    async fn new(enabled:bool)->Self{
        let directory=tempfile::tempdir().unwrap();
        let shared_root=directory.path().join("shared-user-tools");
        let package=if cfg!(windows){shared_root.join("node_modules/@larksuite/cli")}
            else{shared_root.join("lib/node_modules/@larksuite/cli")};
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(package.join("package.json"),r#"{"name":"@larksuite/cli"}"#).unwrap();
        std::fs::write(package.join("bin").join(if cfg!(windows){"lark-cli.exe"}else{"lark-cli"}),b"test-only CLI fixture").unwrap();
        let owner=nomifun_common::UserId::new();
        let database=nomifun_db::init_database_memory_with_owner(owner.clone()).await.unwrap();
        let store=Arc::new(FeishuConnectorStore::new(database.pool().clone()));
        let executor=Arc::new(FixtureExecutor::new());
        let runtime=Arc::new(CliRuntime::with_executor_and_root(directory.path().join("data"),shared_root.clone(),executor.clone()));
        let workers:Arc<Mutex<Vec<JoinHandle<()>>>>=Arc::new(Mutex::new(Vec::new()));
        let retained=workers.clone();
        let spawn:ConnectorSpawner=Arc::new(move|work|{lock(&retained).push(tokio::spawn(work));true});
        let shutdown=CancellationToken::new();
        let service=FeishuConnectorService::new(owner.into_string(),store,runtime,spawn,shutdown.clone(),false).await.unwrap();
        if enabled{
            service.update_connection(ConnectionUpdate{name:"Work".into(),enabled:true,profile:Some("work".into())}).await.unwrap();
        }
        Self{_directory:directory,database,service,executor,workers,shutdown,shared_root}
    }
    async fn join_workers(&self){
        loop{
            let workers=std::mem::take(&mut *lock(&self.workers));
            if workers.is_empty(){break}
            for worker in workers{tokio::time::timeout(Duration::from_secs(5),worker).await.expect("offline connector worker completed").unwrap()}
        }
    }
    async fn close(self){
        self.shutdown.cancel();self.join_workers().await;self.database.close().await;
    }
}
fn request(items:Vec<FeishuMutation>)->ExecuteRequest{
    ExecuteRequest{request_id:Uuid::now_v7().to_string(),expected_actor:FeishuActor{app_id:"cli_fixture".into(),user_id:"ou_owner".into(),name:"Owner".into()},profile:"work".into(),items}
}
fn doc_request()->ExecuteRequest{
    request(vec![FeishuMutation::DocumentCreate{
        client_item_id:Uuid::now_v7().to_string(),title:"Review the connector implementation".into()}])
}

#[tokio::test]
async fn feishu_http_face_requires_the_exact_installation_owner_before_diagnosis_or_writes(){
    use axum::{body::Body,http::{Request,StatusCode},middleware};
    use nomifun_auth::{CurrentUser,InstanceOwnerState,require_instance_owner_middleware};
    use tower::ServiceExt;
    let f=Fixture::new(false).await;
    let router=crate::feishu_connector_routes(f.service.clone()).route_layer(middleware::from_fn_with_state(
        InstanceOwnerState::new(Arc::from(f.service.owner.as_str())),require_instance_owner_middleware));
    let missing=router.clone().oneshot(Request::builder().uri("/api/feishu-connector/status").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(missing.status(),StatusCode::FORBIDDEN);
    assert!(f.executor.calls().is_empty(),"unauthenticated requests never reach even the diagnostic executor");
    let mut other=Request::builder().method("POST").uri("/api/feishu-connector/operations")
        .header("content-type","application/json").body(Body::from(serde_json::to_vec(&doc_request()).unwrap())).unwrap();
    other.extensions_mut().insert(CurrentUser{id:nomifun_common::UserId::new(),username:"Owner".into()});
    assert_eq!(router.clone().oneshot(other).await.unwrap().status(),StatusCode::FORBIDDEN);
    assert!(f.executor.calls().is_empty(),"a matching display name cannot borrow the installation owner's authority");
    let mut owner=Request::builder().uri("/api/feishu-connector/status").body(Body::empty()).unwrap();
    owner.extensions_mut().insert(CurrentUser{id:nomifun_common::UserId::parse(&f.service.owner).unwrap(),username:"Renamed owner".into()});
    let response=router.clone().oneshot(owner).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    let body=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
    let value:Value=serde_json::from_slice(&body).unwrap();
    assert_eq!(value["data"]["phase"],"disabled");
    let removed_recent=Request::builder().method("POST").uri("/api/feishu-connector/resources/recent")
        .header("content-type","application/json").body(Body::from(r#"{"url":"https://example.feishu.cn/docx/doc_real"}"#)).unwrap();
    assert_eq!(router.clone().oneshot(removed_recent).await.unwrap().status(),StatusCode::NOT_FOUND);
    let mut retired_list=Request::builder().uri("/api/feishu-connector/operations").body(Body::empty()).unwrap();
    retired_list.extensions_mut().insert(CurrentUser{id:nomifun_common::UserId::parse(&f.service.owner).unwrap(),username:"Owner".into()});
    assert_eq!(router.oneshot(retired_list).await.unwrap().status(),StatusCode::METHOD_NOT_ALLOWED);
    assert!(f.executor.calls().iter().all(|call|!call.mutating));
    f.close().await;
}
fn base_request()->ExecuteRequest{
    request(["rec_one","rec_two","rec_three"].into_iter().map(|id|
        FeishuMutation::BaseUpdate{client_item_id:Uuid::now_v7().to_string(),base_token:"base_fixture".into(),table_id:"table_fixture".into(),
            record_id:id.into(),fields:BTreeMap::from([("fld_status".into(),json!(["Contacted"]))]),
            before:BTreeMap::from([("fld_status".into(),json!(["New"]))])}).collect())
}
use uuid::Uuid;

#[tokio::test]
async fn cold_status_observes_existing_runtime_without_installing_or_reading_account(){
    let f=Fixture::new(false).await;
    let status=f.service.status().await.unwrap();
    assert_eq!(status.phase,ConnectionPhase::Disabled);
    assert_eq!(status.runtime.phase,RuntimePhase::Ready);
    assert!(status.actor.is_none());
    let calls=f.executor.calls();
    assert_eq!(calls.len(),1,"only the version/identity probe");
    assert!(calls.iter().all(|c|!c.mutating&&!c.args.iter().any(|a|matches!(a.as_str(),"install"|"auth"|"config"))));
    assert!(!f.shared_root.join(".nomifun-lark-cli.install.lock").exists());
    assert!(f.service.store.connection(&f.service.owner).await.unwrap().is_none(),"visiting disabled connector does not write a connection");
    f.close().await;
}

#[tokio::test]
async fn disabled_profile_discovery_is_explicit_read_only_and_never_reads_user_authorization(){
    let f=Fixture::new(false).await;
    let profiles=f.service.profiles().await.unwrap();
    assert_eq!(profiles.len(),1);
    assert_eq!(profiles[0].name,"work");
    assert_eq!(profiles[0].app_id,"cli_fixture");
    let calls=f.executor.calls();
    assert_eq!(calls.len(),2,"only the runtime probe and one explicit profile list");
    assert_eq!(calls.iter().filter(|call|contains(&call.args,&["profile","list"])).count(),1);
    assert!(calls.iter().all(|call|!call.mutating&&!call.args.iter().any(|arg|matches!(arg.as_str(),"auth"|"install"|"config"))));
    assert!(f.service.store.connection(&f.service.owner).await.unwrap().is_none(),"discovery does not enable or create a connection");
    assert!(!f.shared_root.join(".nomifun-lark-cli.install.lock").exists());
    assert_eq!(f.service.status().await.unwrap().phase,ConnectionPhase::Disabled);
    assert_eq!(f.executor.calls().len(),calls.len(),"normal status polling does not rediscover profiles");
    f.close().await;
}

#[tokio::test]
async fn enabled_status_polls_identity_without_launching_profile_discovery(){
    let f=Fixture::new(true).await;
    assert_eq!(f.service.status().await.unwrap().phase,ConnectionPhase::Ready);
    assert_eq!(f.service.status().await.unwrap().phase,ConnectionPhase::Ready);
    let calls=f.executor.calls();
    assert_eq!(calls.iter().filter(|call|contains(&call.args,&["auth","status"])).count(),2);
    assert!(!calls.iter().any(|call|contains(&call.args,&["profile","list"])));
    f.close().await;
}

#[tokio::test]
async fn explicit_update_check_reports_registry_and_compatibility_without_installing_or_authenticating(){
    let f=Fixture::new(false).await;
    let checked=f.service.check_update().await.unwrap();
    assert_eq!(checked.installed_version.as_deref(),Some("1.0.98"));
    assert_eq!(checked.latest_version.as_deref(),Some("1.0.99"));
    assert_eq!(checked.compatible_version,"1.0.98");
    assert!(checked.update_available);
    assert!(f.executor.calls().iter().all(|call|!call.mutating&&!call.args.iter().any(|arg|matches!(arg.as_str(),"install"|"auth"|"--force"|"--skills-layout"))));
    assert!(!f.service.connection().await.unwrap().enabled);
    f.close().await;
}

#[tokio::test]
async fn an_expired_user_token_does_not_prevent_starting_a_fresh_login_for_the_configured_application(){
    let f=Fixture::new(true).await;
    f.executor.old_token_expired.store(true,Ordering::SeqCst);
    assert_eq!(f.service.status().await.unwrap().phase,ConnectionPhase::Expired);
    let flow=f.service.start_authorization(AuthStartRequest{kind:AuthKind::Login,
        scopes:vec!["docx:document:create".into()],profile:Some("work".into()),app_id:None,app_secret:None}).await.unwrap();
    f.join_workers().await;
    assert_eq!(f.service.authorization(&flow.id).unwrap().phase,AuthPhase::Complete);
    assert!(f.executor.calls().iter().any(|call|contains(&call.args,&["auth","login"])&&call.args.iter().any(|arg|arg=="--no-wait")));
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),0,"reauthorization never submits a business draft");
    f.close().await;
}

fn logout_request()->LogoutRequest{
    LogoutRequest{profile:"work".into(),expected_actor:Some(doc_request().expected_actor)}
}

#[tokio::test]
async fn logout_targets_only_the_selected_single_user_profile_and_allows_a_fresh_login(){
    let f=Fixture::new(true).await;
    let connection=f.service.connection().await.unwrap();
    f.service.logout(logout_request()).await.unwrap();f.join_workers().await;
    assert_eq!(f.executor.logouts.load(Ordering::SeqCst),1);
    let status=f.service.status().await.unwrap();assert_eq!(status.phase,ConnectionPhase::NeedsLogin);assert!(status.actor.is_none());
    assert_eq!(status.connection.id,connection.id);assert_eq!(status.connection.profile,connection.profile);
    assert_eq!(status.connection.name,connection.name);assert!(status.connection.enabled);
    assert_eq!(f.executor.calls().iter().filter(|call|contains(&call.args,&["auth","list"])).count(),2,"logout verifies the original profile before and after its one invocation");
    let flow=f.service.start_authorization(AuthStartRequest{kind:AuthKind::Login,scopes:vec!["offline_access".into()],
        profile:Some("work".into()),app_id:None,app_secret:None}).await.unwrap();f.join_workers().await;
    assert_eq!(f.service.authorization(&flow.id).unwrap().phase,AuthPhase::Complete);
    assert_eq!(f.service.status().await.unwrap().phase,ConnectionPhase::Ready);
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),0);
    f.close().await;
}

#[tokio::test]
async fn logout_refuses_changed_identity_profile_and_multi_user_connections_without_touching_tokens(){
    let f=Fixture::new(true).await;
    let mut input=logout_request();input.profile="other".into();assert_eq!(f.service.logout(input).await.unwrap_err().code,"FEISHU_CONFLICT");
    let mut input=logout_request();input.expected_actor.as_mut().unwrap().user_id="ou_other".into();
    assert_eq!(f.service.logout(input).await.unwrap_err().code,"FEISHU_CONFLICT");
    lock(&f.executor.users).push(FeishuActor{app_id:"cli_fixture".into(),user_id:"ou_other".into(),name:"Other user".into()});
    let error=f.service.logout(logout_request()).await.unwrap_err();assert_eq!(error.code,"FEISHU_CONFLICT");assert!(error.message.contains("切换连接"));
    assert_eq!(f.executor.logouts.load(Ordering::SeqCst),0);assert_eq!(lock(&f.executor.users).len(),2);
    f.close().await;
}

#[tokio::test]
async fn logout_supports_an_expired_account_without_an_actor_and_is_a_noop_after_signout(){
    let f=Fixture::new(true).await;f.executor.old_token_expired.store(true,Ordering::SeqCst);
    let input=LogoutRequest{profile:"work".into(),expected_actor:None};
    f.service.logout(input.clone()).await.unwrap();f.service.logout(input).await.unwrap();
    assert_eq!(f.executor.logouts.load(Ordering::SeqCst),1);
    assert_eq!(f.service.status().await.unwrap().phase,ConnectionPhase::NeedsLogin);
    f.close().await;
}

#[tokio::test]
async fn logout_refuses_a_durably_queued_operation_without_a_live_worker(){
    let f=Fixture::new(true).await;let input=doc_request();let connection=f.service.connection().await.unwrap();
    let queued=f.service.store.create_submitted(&f.service.owner,&connection.id,&input.expected_actor,"work",&input).await.unwrap();
    let error=f.service.logout(logout_request()).await.unwrap_err();assert_eq!(error.related_operation_id,Some(queued.id));
    assert_eq!(f.executor.logouts.load(Ordering::SeqCst),0);assert!(!f.executor.calls().iter().any(|call|contains(&call.args,&["auth","list"])));
    f.close().await;
}

#[tokio::test]
async fn cancelled_authorization_keeps_its_owner_fence_until_original_cli_cleanup_finishes(){
    let f=Fixture::new(true).await;
    let retained=doc_request();
    f.executor.hold_auth_cleanup.store(true,Ordering::SeqCst);
    let input=AuthStartRequest{kind:AuthKind::Login,scopes:vec!["docx:document:create".into()],profile:Some("work".into()),app_id:None,app_secret:None};
    let first=f.service.start_authorization(input.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3),f.executor.auth_poll_entered.acquire()).await.unwrap().unwrap().forget();
    assert_eq!(f.service.authorization(&first.id).unwrap().phase,AuthPhase::WaitingUser);
    assert_eq!(f.service.cancel_authorization(&first.id).await.unwrap().phase,AuthPhase::Cancelling);
    tokio::time::timeout(Duration::from_secs(3),f.executor.auth_cancel_seen.acquire()).await.unwrap().unwrap().forget();
    let conflict=f.service.start_authorization(input.clone()).await.unwrap_err();
    assert_eq!(conflict.related_flow_id.as_deref(),Some(first.id.as_str()));
    assert!(f.service.update_connection(ConnectionUpdate{name:"Changed".into(),enabled:true,profile:Some("other".into())}).await.is_err());
    assert!(f.service.execute(retained.clone()).await.is_err());
    assert!(f.service.logout(logout_request()).await.is_err());
    assert_eq!(f.executor.logouts.load(Ordering::SeqCst),0);
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),0);
    f.executor.auth_cleanup_release.add_permits(1);f.join_workers().await;
    assert_eq!(f.service.authorization(&first.id).unwrap().phase,AuthPhase::Cancelled);
    f.executor.hold_auth_cleanup.store(false,Ordering::SeqCst);
    let next=f.service.start_authorization(input).await.unwrap();f.join_workers().await;
    assert_eq!(f.service.authorization(&next.id).unwrap().phase,AuthPhase::Complete);
    assert!(f.service.store.get_by_request(&f.service.owner,&retained.request_id).await.unwrap().is_none(),"authorization does not submit local input");
    f.close().await;
}

#[tokio::test]
async fn scope_repair_never_submits_local_input_and_exact_request_is_idempotent(){
    let f=Fixture::new(true).await;let input=doc_request();
    let error=f.service.execute(input.clone()).await.unwrap_err();
    assert_eq!(error.code,"FEISHU_PERMISSION");
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),0);
    assert!(f.service.store.get_by_request(&f.service.owner,&input.request_id).await.unwrap().is_none());
    f.executor.grant("docx:document:create");f.service.status().await.unwrap();
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),0);
    let submitted=f.service.execute(input.clone()).await.unwrap();f.join_workers().await;
    assert_eq!(f.service.operation(&submitted.id).await.unwrap().status,OperationStatus::Succeeded);
    assert_eq!(f.service.execute(input).await.unwrap().id,submitted.id);f.join_workers().await;
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),1);
    f.close().await;
}

#[tokio::test]
async fn changed_expected_identity_or_profile_cannot_borrow_the_current_login(){
    let f=Fixture::new(true).await;f.executor.grant("docx:document:create");
    let mut input=doc_request();input.expected_actor.user_id="ou_someone_else".into();
    assert!(f.service.execute(input).await.is_err());
    let mut input=doc_request();input.profile="different-profile".into();
    assert!(f.service.execute(input).await.is_err());
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),0);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM feishu_operations").fetch_one(f.database.pool()).await.unwrap(),0);
    f.close().await;
}

#[tokio::test]
async fn accepted_worker_outlives_the_request_waiter_and_repeated_request_does_not_resend(){
    let f=Fixture::new(true).await;f.executor.grant("docx:document:create");f.executor.hold_create.store(true,Ordering::SeqCst);
    let input=doc_request();let requester=f.service.clone();let sent=input.clone();
    let request=tokio::spawn(async move{requester.execute(sent).await});
    let admitted=tokio::time::timeout(Duration::from_secs(3),request).await.unwrap().unwrap().unwrap();
    let id=admitted.id;tokio::time::timeout(Duration::from_secs(3),f.executor.create_entered.acquire()).await.unwrap().unwrap().forget();
    assert_eq!(f.service.operation(&id).await.unwrap().items[0].status,ItemStatus::Running);
    assert!(f.service.logout(logout_request()).await.is_err());assert_eq!(f.executor.logouts.load(Ordering::SeqCst),0);
    assert_eq!(f.service.execute(input).await.unwrap().id,id);
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),1);
    f.executor.create_release.add_permits(1);f.join_workers().await;
    let settled=f.service.operation(&id).await.unwrap();
    assert_eq!(settled.status,OperationStatus::Succeeded);assert_eq!(settled.items[0].attempts.len(),1);
    f.close().await;
}

#[tokio::test]
async fn disabled_inflight_work_keeps_its_real_outcome_and_reenable_does_not_replay(){
    for times_out in [false,true]{
        let f=Fixture::new(true).await;f.executor.grant("docx:document:create");f.executor.hold_create.store(true,Ordering::SeqCst);f.executor.timeout_create.store(times_out,Ordering::SeqCst);
        let input=doc_request();let submitted=f.service.execute(input.clone()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3),f.executor.create_entered.acquire()).await.unwrap().unwrap().forget();
        f.service.update_connection(ConnectionUpdate{name:"Work".into(),enabled:false,profile:Some("work".into())}).await.unwrap();
        assert_eq!(f.service.operation(&submitted.id).await.unwrap().items[0].status,ItemStatus::Running);
        f.executor.create_release.add_permits(1);f.join_workers().await;
        let result=f.service.operation(&submitted.id).await.unwrap();
        assert_eq!(result.items[0].status,if times_out{ItemStatus::Unknown}else{ItemStatus::Succeeded});
        assert_eq!(result.items[0].attempts.len(),1);
        f.service.update_connection(ConnectionUpdate{name:"Work".into(),enabled:true,profile:Some("work".into())}).await.unwrap();
        f.service.execute(input).await.unwrap();f.join_workers().await;
        assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),1);f.close().await;
    }
}

#[tokio::test]
async fn multi_document_changes_are_rejected_before_any_write(){
    let f=Fixture::new(true).await;f.executor.grant("docx:document:create");
    let mut input=doc_request();input.items.extend(doc_request().items);
    assert!(f.service.execute(input).await.is_err());
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),0);f.close().await;
}

#[tokio::test]
async fn aborted_http_waiter_before_admission_does_not_cancel_owned_execution(){
    let f=Fixture::new(true).await;f.executor.grant("base:record:update");
    let input=base_request();
    f.executor.hold_preflight.store(true,Ordering::SeqCst);
    let sent=input.clone();let requester=f.service.clone();let waiter=tokio::spawn(async move{requester.execute(sent).await});
    tokio::time::timeout(Duration::from_secs(3),f.executor.preflight_entered.acquire()).await.unwrap().unwrap().forget();
    assert!(f.service.store.get_by_request(&f.service.owner,&input.request_id).await.unwrap().is_none());
    waiter.abort();assert!(waiter.await.unwrap_err().is_cancelled());
    f.executor.preflight_release.add_permits(1);f.join_workers().await;
    let finished=f.service.operation_by_request(&input.request_id).await.unwrap();
    assert_eq!(finished.status,OperationStatus::AcceptedPendingConfirmation);assert!(finished.items.iter().all(|item|item.attempts.len()==1));
    f.service.execute(input).await.unwrap();f.join_workers().await;
    assert_eq!(f.executor.base_batches.load(Ordering::SeqCst),1);f.close().await;
}

#[tokio::test]
async fn timed_out_create_stays_unknown_and_reconciliation_never_searches_or_writes(){
    let f=Fixture::new(true).await;f.executor.grant("docx:document:create");f.executor.timeout_create.store(true,Ordering::SeqCst);
    let input=doc_request();let submitted=f.service.execute(input.clone()).await.unwrap();f.join_workers().await;
    let unknown=f.service.operation(&submitted.id).await.unwrap();
    assert_eq!(unknown.items[0].status,ItemStatus::Unknown);assert!(unknown.items[0].remote_id.is_none());
    let before=f.executor.calls().len();f.service.reconcile(&submitted.id).await.unwrap();
    let twice=f.service.reconcile(&submitted.id).await.unwrap();
    assert_eq!(twice.items[0].status,ItemStatus::Unknown);assert_eq!(twice.items[0].attempts.len(),1);assert_eq!(twice.items[0].observations.len(),2);
    assert!(f.executor.calls()[before..].iter().all(|call|!call.mutating&&contains(&call.args,&["auth","status"])));
    f.service.execute(input).await.unwrap();f.join_workers().await;
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),1);f.close().await;
}

#[tokio::test]
async fn base_rows_share_one_native_batch_and_need_exact_readback_before_success(){
    let f=Fixture::new(true).await;f.executor.grant("base:record:update");
    let input=base_request();let submitted=f.service.execute(input.clone()).await.unwrap();f.join_workers().await;
    let accepted=f.service.operation(&submitted.id).await.unwrap();
    assert_eq!(accepted.status,OperationStatus::AcceptedPendingConfirmation);
    assert!(accepted.items.iter().all(|i|i.status==ItemStatus::AcceptedPendingConfirmation&&i.attempts.len()==1));
    assert_eq!(f.executor.base_batches.load(Ordering::SeqCst),1);
    let writes=f.executor.calls().into_iter().filter(|c|c.mutating).collect::<Vec<_>>();assert_eq!(writes.len(),1);
    assert_eq!(writes[0].input.as_ref().unwrap()["update_records"].as_object().unwrap().len(),3);
    assert!(f.service.reconcile(&submitted.id).await.unwrap().items.iter().all(|i|i.status==ItemStatus::AcceptedPendingConfirmation));
    lock(&f.executor.confirmed_records).insert("rec_one".into());
    let partly=f.service.reconcile(&submitted.id).await.unwrap();assert_eq!(partly.status,OperationStatus::Partial);
    assert_eq!(partly.items.iter().filter(|i|i.status==ItemStatus::Succeeded).count(),1);
    assert!(partly.items.iter().all(|i|i.attempts[0].status==ItemStatus::AcceptedPendingConfirmation));
    lock(&f.executor.confirmed_records).extend(["rec_two".into(),"rec_three".into()]);
    let confirmed=f.service.reconcile(&submitted.id).await.unwrap();assert_eq!(confirmed.status,OperationStatus::Succeeded);
    assert!(confirmed.items.iter().all(|i|i.attempts.len()==1));
    f.service.execute(input).await.unwrap();f.join_workers().await;
    assert_eq!(f.executor.base_batches.load(Ordering::SeqCst),1);f.close().await;
}

#[tokio::test]
async fn connection_can_authorize_only_offline_access_without_business_read_or_write(){
    let f=Fixture::new(true).await;lock(&f.executor.scopes).clear();
    let flow=f.service.start_authorization(AuthStartRequest{kind:AuthKind::Login,scopes:vec!["offline_access".into()],profile:Some("work".into()),app_id:None,app_secret:None}).await.unwrap();
    f.join_workers().await;assert_eq!(f.service.authorization(&flow.id).unwrap().phase,AuthPhase::Complete);
    let status=f.service.status().await.unwrap();assert_eq!(status.actor.unwrap().user_id,"ou_owner");
    assert!(status.permissions.iter().all(|permission|!permission.granted));
    assert_eq!(*lock(&f.executor.requested_scopes),vec!["offline_access"]);
    assert!(f.executor.calls().iter().all(|call|!call.mutating));
    f.close().await;
}

#[tokio::test]
async fn base_save_cannot_omit_the_original_value_of_any_written_field(){
    let f=Fixture::new(true).await;f.executor.grant("base:record:update");
    for mode in [0,1]{
        let mut input=base_request();input.items.truncate(1);
        if let FeishuMutation::BaseUpdate{before,..}=&mut input.items[0]{
            before.clear();if mode==1{before.insert("unrelated_field".into(),json!("New"));}
        }
        assert!(f.service.execute(input).await.is_err());
    }
    assert_eq!(f.executor.base_batches.load(Ordering::SeqCst),0);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM feishu_operations").fetch_one(f.database.pool()).await.unwrap(),0);
    f.close().await;
}

#[tokio::test]
async fn shutdown_does_not_discard_a_known_receipt_while_its_original_owner_retries_storage(){
    let f=Fixture::new(true).await;f.executor.grant("docx:document:create");f.executor.hold_create.store(true,Ordering::SeqCst);
    sqlx::query("CREATE TRIGGER qa_receipt_storage_failure BEFORE UPDATE ON feishu_operation_receipts WHEN NEW.status='succeeded' BEGIN SELECT RAISE(FAIL,'QA temporary storage failure'); END")
        .execute(f.database.pool()).await.unwrap();
    let submitted=f.service.execute(doc_request()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3),f.executor.create_entered.acquire()).await.unwrap().unwrap().forget();
    f.executor.create_release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(3),f.executor.create_returned.acquire()).await.unwrap().unwrap().forget();
    f.shutdown.cancel();tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(f.service.operation(&submitted.id).await.unwrap().items[0].status,ItemStatus::Running);
    sqlx::query("DROP TRIGGER qa_receipt_storage_failure").execute(f.database.pool()).await.unwrap();
    f.join_workers().await;
    let settled=f.service.operation(&submitted.id).await.unwrap();
    assert_eq!(settled.items[0].status,ItemStatus::Succeeded);
    assert_eq!(settled.items[0].remote_id.as_deref(),Some("doc_real"));
    assert_eq!(settled.items[0].attempts.len(),1);
    assert_eq!(f.executor.document_creates.load(Ordering::SeqCst),1);
    f.close().await;
}

#[tokio::test]
async fn document_deletion_preserves_its_single_result_and_never_reads_content(){
    let f=Fixture::new(true).await;
    let listed=f.service.query(FeishuQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:None}).await.unwrap();assert_eq!(listed.items[0].id,"doc_real");
    let input=request(vec![FeishuMutation::DocumentDelete{client_item_id:Uuid::now_v7().to_string(),document_id:"doc_real".into(),document_type:"docx".into(),title:"Fixture document".into()}]);
    let denied=f.service.execute(input.clone()).await.unwrap_err();assert_eq!(denied.code,"FEISHU_PERMISSION");
    assert_eq!(denied.missing_scopes,vec!["drive:drive.metadata:readonly","space:document:delete"]);
    assert_eq!(f.executor.document_deletes.load(Ordering::SeqCst),0);
    f.executor.grant("space:document:delete");f.executor.grant("drive:drive.metadata:readonly");
    let submitted=f.service.execute(input.clone()).await.unwrap();f.join_workers().await;
    let settled=f.service.operation(&submitted.id).await.unwrap();assert_eq!(settled.status,OperationStatus::Succeeded);
    assert_eq!(settled.items[0].attempts.len(),1);
    assert_eq!(settled.items[0].remote_id.as_deref(),Some("doc_real"));
    f.service.execute(input).await.unwrap();f.join_workers().await;
    assert_eq!(f.executor.document_deletes.load(Ordering::SeqCst),1);
    assert!(f.executor.calls().iter().all(|call|!contains(&call.args,&["docs","+fetch"])&&!call.args.iter().any(|arg|arg.contains("/blocks"))));
    f.close().await;
}

#[tokio::test]
async fn unknown_document_deletion_preserves_one_attempt_and_blocks_another_request(){
    let f=Fixture::new(true).await;f.executor.grant("space:document:delete");f.executor.grant("drive:drive.metadata:readonly");f.executor.timeout_document.store(true,Ordering::SeqCst);
    let input=request(vec![FeishuMutation::DocumentDelete{client_item_id:Uuid::now_v7().to_string(),document_id:"doc_real".into(),document_type:"docx".into(),title:"Fixture document".into()}]);
    let submitted=f.service.execute(input.clone()).await.unwrap();f.join_workers().await;
    let original=f.service.operation(&submitted.id).await.unwrap();assert_eq!(original.items[0].status,ItemStatus::Unknown);
    let listed=f.service.query(FeishuQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:None}).await.unwrap();
    assert_eq!(listed.items[0].pending_operation_id.as_deref(),Some(submitted.id.as_str()));
    assert_eq!(listed.items[0].pending_status,Some(ItemStatus::Unknown));
    f.service.execute(input.clone()).await.unwrap();
    let mut other=input;other.request_id=Uuid::now_v7().to_string();
    assert_eq!(f.service.execute(other).await.unwrap_err().related_operation_id,Some(submitted.id.clone()));
    let observed=f.service.reconcile(&submitted.id).await.unwrap();
    assert_eq!(observed.items[0].status,ItemStatus::Unknown);assert_eq!(observed.items[0].attempts.len(),1);
    assert_eq!(f.executor.document_deletes.load(Ordering::SeqCst),1);
    f.close().await;
}

#[tokio::test]
async fn creating_a_blank_document_returns_its_exact_resource_identity_and_link(){
    let f=Fixture::new(true).await;f.executor.grant("docx:document:create");
    let submitted=f.service.execute(request(vec![FeishuMutation::DocumentCreate{client_item_id:Uuid::now_v7().to_string(),title:"New document".into()}])).await.unwrap();
    f.join_workers().await;
    let settled=f.service.operation(&submitted.id).await.unwrap();
    assert_eq!(settled.status,OperationStatus::Succeeded);
    assert_eq!(settled.items[0].mutation["title"],"New document");
    assert_eq!(settled.items[0].remote_id.as_deref(),Some("doc_real"));
    assert_eq!(settled.items[0].remote_url.as_deref(),Some("https://example.feishu.cn/docx/doc_real"));
    assert!(f.executor.calls().iter().all(|call|!call.args.contains(&"--content".into())));
    f.close().await;
}

#[tokio::test]
async fn existing_wiki_documents_paginate_with_the_existing_login_and_server_type_filter(){
    let f=Fixture::new(true).await;f.executor.document_pages.store(true,Ordering::SeqCst);
    let status=f.service.status().await.unwrap();
    assert!(status.permissions.iter().find(|permission|permission.capability=="documents.browse").unwrap().granted);
    assert!(!lock(&f.executor.scopes).contains(&"space:document:retrieve".into()));
    let first=f.service.query(FeishuQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:None}).await.unwrap();
    assert_eq!(first.items.len(),20);assert!(first.items.iter().all(|item|item.kind=="wiki"));assert!(first.has_more);
    let second=f.service.query(FeishuQuery::Documents{query:String::new(),filter:DocumentFilter::All,cursor:first.next_cursor}).await.unwrap();
    assert_eq!(second.items.len(),5);assert!(!second.has_more);assert!(second.next_cursor.is_none());
    assert_eq!(second.items.iter().filter(|item|item.kind=="docx").count(),2);
    let ids=first.items.iter().chain(&second.items).map(|item|&item.id).collect::<HashSet<_>>();assert_eq!(ids.len(),25);
    let base=f.service.query(FeishuQuery::Documents{query:String::new(),filter:DocumentFilter::Base,cursor:None}).await.unwrap();
    assert_eq!(base.items.len(),1);assert_eq!(base.items[0].kind,"base");
    assert!(f.executor.calls().iter().all(|call|!call.mutating&&!contains(&call.args,&["drive","files","list"])&&!contains(&call.args,&["docs","+fetch"])));
    f.close().await;
}
