use super::*;
use crate::EmptyExecutionCreationFence;

struct Fixture { owner: PrincipalRef, session: AgentSessionId, attempt: String, execution: String, step: String }

async fn fixture(store:&AgentSessionStore,ready:bool)->Fixture {
    let user=Uuid::now_v7().to_string();let execution=Uuid::now_v7().to_string();
    let step=Uuid::now_v7().to_string();let attempt=Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,created_at,updated_at) VALUES(?,?,'empty creation','running','fixed','automatic','disabled','{\"mode\":\"explicit\"}',1,1)")
        .bind(&execution).bind(&user).execute(store.test_pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,status,introduced_in_revision,created_at,updated_at) VALUES(?,?,'child','frozen original task','agent','cancelled',0,1,1)")
        .bind(&step).bind(&execution).execute(store.test_pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,created_at,updated_at) VALUES(?,?,?,0,'cancelled','initial',1,1)")
        .bind(&attempt).bind(&execution).bind(&step).execute(store.test_pool()).await.unwrap();
    let owner=PrincipalRef {principal_kind:"user".into(),principal_id:user};
    let mut live=live_session(session_id());live.owner_ref=owner.clone();
    let session=live.agent_session_id.clone();let key=format!("user:{}:open:{attempt}",owner.principal_id);
    let mut request=create_request(live,&key);
    request.producer_id="session_api".into();request.operation_id=format!("open:{key}").into();
    request.correlation_id=format!("open:{key}").into();request.opening_event_id=Some(format!("opening:{key}").into());
    request.activation_event_id=Some(format!("active-set:{key}").into());
    store.create_session(request).await.unwrap();
    if ready {store.append_event(&append(&session,&format!("ready:{key}"),"runtime_supervisor",&format!("{key}:ready"),
        "session/ready",&format!("session:{}",session.as_ref()),Some(format!("opening:{key}").into()),
        json!({"resolved_snapshot_ref":snapshot_ref()}))).await.unwrap();}
    Fixture {owner,session,attempt,execution,step}
}

async fn input(store:&AgentSessionStore,f:&Fixture)->Result<(crate::SessionEventAppendResult,crate::SessionEventAppendResult),SessionStoreError> {
    store.start_turn(&f.session,"session_api".into(),"legitimate-input".into(),"legitimate-turn".into(),
        StrictJsonValue(json!({"content":"real work admitted by another writer"}))).await
}

#[tokio::test]
async fn empty_execution_creation_stop_before_link_fences_partial_or_ready_and_replays_original_identity() {
    for ready in [false,true] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,ready).await;
        let EmptyExecutionCreationFence::Fenced {command,session}=store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),2).await.unwrap() else {panic!("untouched creation must be fenced")};
        assert_eq!(session.agent_session_id,f.session);
        let EmptyExecutionCreationFence::Fenced {command:again,..}=store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),3).await.unwrap() else {panic!("original fence must be replayable")};
        assert_eq!(command.operation_id,again.operation_id);
        assert!(input(&store,&f).await.is_err(),"fence-winning writer excludes later legitimate input");
        store.complete_delete(&command,4).await.unwrap();
        assert!(matches!(store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),5).await.unwrap(),EmptyExecutionCreationFence::Missing));
    }
}

#[tokio::test]
async fn empty_execution_creation_input_winner_is_never_discarded() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,true).await;
    input(&store,&f).await.unwrap();let cursor=store.current_cursor(&f.session).await.unwrap();
    assert!(matches!(store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),3).await.unwrap(),EmptyExecutionCreationFence::NotEmpty {..}));
    assert_eq!(store.current_cursor(&f.session).await.unwrap(),cursor);
    assert!(store.get_live_session(&f.session).await.is_ok());
}

#[tokio::test]
async fn empty_execution_creation_any_link_or_wrong_expected_session_preserves_original() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,true).await;
    assert!(matches!(store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&session_id()),3).await.unwrap(),EmptyExecutionCreationFence::NotEmpty {..}));
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES(?,?,'attempt',?,?,0,1,1)")
        .bind(f.session.as_ref()).bind(&f.execution).bind(&f.step).bind(&f.attempt).execute(store.test_pool()).await.unwrap();
    assert!(matches!(store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),3).await.unwrap(),EmptyExecutionCreationFence::NotEmpty {..}));
    assert!(store.get_live_session(&f.session).await.is_ok());
}

#[tokio::test]
async fn empty_execution_creation_does_not_accept_a_later_or_foreign_initialization_fact() {
    for column in ["idempotency_key","producer_id","correlation_id","causation_event_id"] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,true).await;
        // A corrupted/spliced initialization has the same kind and payload,
        // but must not grant deletion authority for this original Attempt.
        let value=if column=="causation_event_id" {"(SELECT event_id FROM agent_events WHERE session_id=? AND seq=3)"}else{"'other-initialization'"};
        let sql=format!("UPDATE agent_events SET {column}={value} WHERE session_id=? AND seq=2");
        let query=sqlx::query(&sql);
        let query=if column=="causation_event_id" {query.bind(f.session.as_ref())}else {query};
        query.bind(f.session.as_ref()).execute(store.test_pool()).await.unwrap();
        assert!(matches!(store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),3).await.unwrap(),EmptyExecutionCreationFence::NotEmpty {..}),"{column}");
        assert!(store.get_live_session(&f.session).await.is_ok());
    }
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn empty_execution_creation_racing_input_has_exactly_one_admission_winner() {
    let directory=tempfile::tempdir().unwrap();let path=directory.path().join("empty-creation-race.sqlite");
    let initialized=nomifun_db::init_database(&path).await.unwrap();
    initialized.close().await;
    let options=sqlx::sqlite::SqliteConnectOptions::new().filename(&path).foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal).busy_timeout(std::time::Duration::from_secs(5));
    let pool=sqlx::sqlite::SqlitePoolOptions::new().min_connections(2).max_connections(2).connect_with(options).await.unwrap();
    let store=AgentSessionStore::from_pool(pool).await.unwrap();let f=fixture(&store,true).await;
    let (input,fence)=tokio::join!(input(&store,&f),store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),3));
    match fence.unwrap() {
        EmptyExecutionCreationFence::Fenced {..}=>assert!(input.is_err()),
        EmptyExecutionCreationFence::NotEmpty {..}=>assert!(input.is_ok()),
        EmptyExecutionCreationFence::Missing=>panic!("known creation cannot disappear"),
        EmptyExecutionCreationFence::AwaitingAttemptDisposition {..}=>panic!("fixture has a durable cancellation"),
    }
    store.test_pool().close().await;
}

#[tokio::test]
async fn empty_execution_creation_lost_owner_cannot_delete_a_successors_queued_reservation() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,true).await;
    sqlx::query("UPDATE agent_execution_attempts SET status='queued' WHERE attempt_id=?").bind(&f.attempt).execute(store.test_pool()).await.unwrap();
    sqlx::query("UPDATE agent_execution_steps SET status='pending' WHERE step_id=?").bind(&f.step).execute(store.test_pool()).await.unwrap();
    let cursor=store.current_cursor(&f.session).await.unwrap();
    assert!(matches!(store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),3).await.unwrap(),EmptyExecutionCreationFence::AwaitingAttemptDisposition {..}));
    assert_eq!(store.current_cursor(&f.session).await.unwrap(),cursor);
    assert!(store.get_live_session(&f.session).await.is_ok());
    sqlx::query("UPDATE agent_execution_attempts SET status='cancelled' WHERE attempt_id=?").bind(&f.attempt).execute(store.test_pool()).await.unwrap();
    assert!(matches!(store.fence_empty_execution_creation(&f.owner,&f.attempt,Some(&f.session),4).await.unwrap(),EmptyExecutionCreationFence::Fenced {..}));
}
