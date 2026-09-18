use std::{path::PathBuf,sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}}};
use nomifun_ai_agent::{AgentRuntimeSessions,AgentRuntimeHandle,types::AgentRuntimeBuildOptions};
use nomifun_common::{AgentKillReason,AppError};
use nomifun_conversation::{ConversationService,skill_resolver::SkillResolver};
use nomifun_api_types::WebSocketMessage;
use tokio::sync::Notify;

const USER:&str="0190f5fe-7c00-7a00-8000-000000000001";
#[derive(Default)]
struct Registry {
    events:Arc<Mutex<Vec<&'static str>>>,
    fail:Arc<AtomicBool>,
    hold:AtomicBool,
    entered:Arc<Notify>,
    release:Arc<Notify>,
}
#[async_trait::async_trait]
impl AgentRuntimeSessions for Registry {
    fn get_runtime(&self,_:&str)->Option<AgentRuntimeHandle> {None}
    async fn get_or_create_runtime(&self,_:&str,_:AgentRuntimeBuildOptions)->Result<AgentRuntimeHandle,AppError> {Err(AppError::Internal("unused fixture".into()))}
    fn terminate(&self,_:&str,_:Option<AgentKillReason>)->Result<(),AppError> {panic!("must use result-bearing retirement")}
    fn terminate_and_wait_result(&self,_:&str,reason:Option<AgentKillReason>)->std::pin::Pin<Box<dyn std::future::Future<Output=Result<(),AppError>>+Send>> {
        assert_eq!(reason,Some(AgentKillReason::ConfigurationChanged));
        let (events,fail,entered,release,hold)=(self.events.clone(),self.fail.clone(),self.entered.clone(),self.release.clone(),self.hold.load(Ordering::Acquire));
        Box::pin(async move {
            events.lock().unwrap().push("retire");entered.notify_one();
            if hold {release.notified().await;}
            if fail.load(Ordering::Acquire) {return Err(AppError::Internal("fixture retirement failed".into()));}
            Ok(())
        })
    }
    fn terminate_all(&self) {}
    fn active_runtime_count(&self)->usize {0}
}
struct Sink;
impl nomifun_realtime::UserEventSink for Sink {fn send_to_user(&self,_:&str,_:WebSocketMessage<serde_json::Value>) {}}
struct Skills;
#[async_trait::async_trait]
impl SkillResolver for Skills {
    async fn auto_inject_names(&self)->Vec<String> {vec![]}
    async fn resolve_skills(&self,_:&[String])->Vec<nomifun_skill_library::ResolvedAgentSkill> {vec![]}
    async fn link_workspace_skills(&self,_:&std::path::Path,_:&[&str],_:&[nomifun_skill_library::ResolvedAgentSkill])->usize {0}
}
async fn fixture(registry:Arc<Registry>)->(ConversationService,nomifun_db::Database,String) {
    let db=nomifun_db::init_database_memory_with_owner(nomifun_common::UserId::parse(USER).unwrap()).await.unwrap();
    let id=nomifun_common::generate_id();
    nomifun_db::sqlx::query("INSERT INTO conversations (conversation_id,user_id,name,type,extra,status,created_at,updated_at) VALUES (?,?,'preserve me','nomi','{}','pending',1,1)")
        .bind(&id).bind(USER).execute(db.pool()).await.unwrap();
    let service=ConversationService::new(Arc::<str>::from(USER),PathBuf::from("/unused-fixture"),Arc::new(Sink),Arc::new(Skills),registry,
        Arc::new(nomifun_db::SqliteConversationRepository::new(db.pool().clone())),
        Arc::new(nomifun_db::SqliteAgentMetadataRepository::new(db.pool().clone())),Arc::new(nomifun_conversation::NoExecutionConversationBoundary));
    (service,db,id)
}
#[tokio::test]
async fn checks_before_retirement_and_retires_before_resource_change() {
    let registry=Arc::new(Registry::default());
    let (service,db,id)=fixture(registry.clone()).await;
    let check=registry.events.clone();let work=registry.events.clone();
    service.with_idle_runtime_reconfiguration(USER,&id,move ||async move {check.lock().unwrap().push("check");Ok(())},move ||async move {work.lock().unwrap().push("work");Ok(())}).await.unwrap();
    assert_eq!(*registry.events.lock().unwrap(),vec!["check","retire","work"]);
    let row:(String,String,i64)=nomifun_db::sqlx::query_as("SELECT name,status,updated_at FROM conversations WHERE conversation_id=?").bind(&id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(row,("preserve me".into(),"pending".into(),1));
    registry.events.lock().unwrap().clear();
    let epoch=service.begin_runtime_build(&id).unwrap().expected_cancellation_epoch();
    assert!(service.with_idle_runtime_reconfiguration(USER,&id,||async {Err(AppError::Conflict("stale resource".into()))},||async {panic!("stale request must not change resources");#[allow(unreachable_code)] Ok(())}).await.is_err());
    assert!(registry.events.lock().unwrap().is_empty());
    assert_eq!(service.begin_runtime_build(&id).unwrap().expected_cancellation_epoch(),epoch,"stale requests must not advance admission epochs");
    registry.fail.store(true,Ordering::Release);
    assert!(service.with_idle_runtime_reconfiguration(USER,&id,||async {Ok(())},||async {panic!("failed retirement must not change resources");#[allow(unreachable_code)] Ok(())}).await.is_err());
}
#[tokio::test]
async fn starting_agent_is_rejected_without_cancelling_its_build() {
    let registry=Arc::new(Registry::default());
    let (service,_db,id)=fixture(registry.clone()).await;
    let build=service.begin_runtime_build(&id).unwrap();
    assert!(service.with_idle_runtime_reconfiguration(USER,&id,||async {Ok(())},||async {Ok(())}).await.is_err());
    assert!(!build.is_cancelled());
    assert!(registry.events.lock().unwrap().is_empty());
}
#[tokio::test]
async fn abandoned_http_caller_does_not_drop_reconfiguration_ownership() {
    let registry=Arc::new(Registry::default());registry.hold.store(true,Ordering::Release);
    let (service,_db,id)=fixture(registry.clone()).await;
    let done=Arc::new(Notify::new());
    let caller=tokio::spawn({let service=service.clone();let id=id.clone();let done=done.clone();async move {
        service.with_idle_runtime_reconfiguration(USER,&id,||async {Ok(())},move ||async move {done.notify_one();Ok(())}).await
    }});
    registry.entered.notified().await;
    caller.abort();assert!(caller.await.unwrap_err().is_cancelled());
    assert!(service.runtime_summary_for(&id).await.is_processing);
    assert!(service.begin_runtime_build(&id).is_err());
    registry.release.notify_one();done.notified().await;
    tokio::time::timeout(std::time::Duration::from_secs(1),async {
        while service.runtime_summary_for(&id).await.is_processing {tokio::task::yield_now().await;}
    }).await.unwrap();
}
