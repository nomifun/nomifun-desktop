use nomifun_db::{
    AppendNomiRemoteEventParams, CreateRemoteBindingParams, GetOrCreateRemoteSessionParams,
    IRemoteBindingRepository, RemoteOpenResult, SqliteRemoteBindingRepository,
    TransitionNomiRemoteSessionParams, UpdateRemoteBindingParams, init_database_memory,
    validate_id_schema_contract,
};
use sha2::{Digest, Sha256};

fn binding_json(version: i64) -> String {
    format!(r#"{{"binding_version":{version},"preset":"nomi.default"}}"#)
}

fn binding_digest(json: &str) -> String {
    hex::encode(Sha256::digest(json.as_bytes()))
}

async fn exact_cancel_fixture()->(nomifun_db::Database,SqliteRemoteBindingRepository,String,String,String) {
    let database=init_database_memory().await.unwrap();let pool=database.pool();
    let owner:String=sqlx::query_scalar("SELECT owner_user_id FROM installation_identity").fetch_one(pool).await.unwrap();
    let session=nomifun_common::ConversationId::new().into_string();
    let binding=nomifun_common::generate_id();let json=binding_json(1);let digest=binding_digest(&json);
    let repository=SqliteRemoteBindingRepository::new(pool.clone());
    repository.create_binding(CreateRemoteBindingParams {remote_binding_id:binding.clone(),owner_user_id:owner.clone(),name:"exact Stop".into(),
        agent_binding_json:json,nomi_snapshot_json:r#"{"snapshot":"frozen"}"#.into(),provenance_json:r#"{"source":"test","version":1}"#.into(),agent_binding_digest:digest.clone(),binding_version:1}).await.unwrap();
    sqlx::query("INSERT INTO agent_sessions(agent_session_id,owner_ref_json,state,title,archived,pinned,agent_binding_json,remote_binding_id,remote_binding_version,next_seq,created_at) \
        VALUES(?,json_object('principal_kind','user','principal_id',?),'live','exact',0,0,json_object('binding_version',1),?,1,2,1)")
        .bind(&session).bind(&owner).bind(&binding).execute(pool).await.unwrap();
    let operation=format!("turn:user:{owner}:{session}:original");
    sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'running',?,0,7,0)")
        .bind(&session).bind(&operation).execute(pool).await.unwrap();
    repository.get_or_create_session(GetOrCreateRemoteSessionParams {owner_user_id:owner.clone(),remote_binding_id:binding,
        expected_binding_version:1,expected_agent_binding_digest:digest,open_idempotency_key:"open-exact".into(),agent_session_id:session.clone(),initial_input_digest:None}).await.unwrap();
    (database,repository,owner,session,operation)
}

fn exact_cancel(owner:&str,session:&str,operation:&str,key:&str,seq:u64)->AppendNomiRemoteEventParams {
    AppendNomiRemoteEventParams {owner_user_id:owner.into(),agent_session_id:session.into(),event_type:"session/cancel-requested".into(),
        payload_json:serde_json::json!({"operation_key_digest":binding_digest(key),"request_digest":binding_digest(&format!("{key}:{operation}:{seq}")),"outcome":"requested",
            "expected_operation_id":operation,"expected_head_seq":seq,"expected_binding_version":1}).to_string()}
}

#[tokio::test]
async fn exact_remote_stop_rejects_a_stale_turn_before_persisting_any_cancel_fence() {
    let (database,repository,owner,session,operation)=exact_cancel_fixture().await;
    let command=exact_cancel(&owner,&session,&operation,"stale",7);
    sqlx::query("UPDATE agent_session_heads SET active_turn_id='successor',last_seq=9 WHERE session_id=?").bind(&session).execute(database.pool()).await.unwrap();
    assert!(matches!(repository.append_cancel_request_exact(command,Some(&operation),7,1).await,Err(nomifun_db::DbError::Conflict(_))));
    assert_eq!(repository.current_event_cursor(&owner,&session).await.unwrap(),0);
    let mut tx=database.pool().begin().await.unwrap();
    assert!(nomifun_db::remote_session_admitted_tx(&mut tx,&session).await.unwrap());tx.commit().await.unwrap();
    database.close().await;
}

#[tokio::test]
async fn exact_remote_stop_writer_fences_all_later_claims_and_replay_keeps_its_original_target() {
    let (database,repository,owner,session,operation)=exact_cancel_fixture().await;
    let command=exact_cancel(&owner,&session,&operation,"original",7);
    assert!(repository.append_cancel_request_exact(command.clone(),Some(&operation),7,1).await.unwrap().inserted);
    let mut tx=database.pool().begin().await.unwrap();
    assert!(!nomifun_db::remote_session_admitted_tx(&mut tx,&session).await.unwrap());
    assert!(nomifun_db::ensure_remote_session_admitted_tx(&mut tx,&session).await.is_err());tx.commit().await.unwrap();
    sqlx::query("UPDATE agent_session_heads SET active_turn_id=NULL,last_seq=10,status='ready' WHERE session_id=?").bind(&session).execute(database.pool()).await.unwrap();
    assert!(!repository.append_cancel_request_exact(command,Some(&operation),7,1).await.unwrap().inserted,"same request replays its captured target after settlement");
    let different=exact_cancel(&owner,&session,"successor","original",10);
    assert!(repository.append_cancel_request_exact(different,Some("successor"),10,1).await.is_err(),"a retry key cannot retarget a successor");
    database.close().await;
}

#[tokio::test]
async fn rejecting_one_remote_stop_key_cannot_clear_another_exact_stop_fence() {
    let (database,repository,owner,session,operation)=exact_cancel_fixture().await;
    repository.append_cancel_request_exact(exact_cancel(&owner,&session,&operation,"first",7),Some(&operation),7,1).await.unwrap();
    repository.append_cancel_request_exact(exact_cancel(&owner,&session,&operation,"second",7),Some(&operation),7,1).await.unwrap();
    repository.append_event_once(AppendNomiRemoteEventParams {owner_user_id:owner.clone(),agent_session_id:session.clone(),event_type:"session/cancel-rejected".into(),
        payload_json:serde_json::json!({"operation_key_digest":binding_digest("second"),"request_digest":binding_digest(&format!("second:{operation}:7")),"outcome":"rejected"}).to_string()}).await.unwrap();
    let mut tx=database.pool().begin().await.unwrap();assert!(!nomifun_db::remote_session_admitted_tx(&mut tx,&session).await.unwrap());tx.commit().await.unwrap();
    database.close().await;
}

#[tokio::test]
async fn a_legacy_unknown_remote_cancel_does_not_retarget_or_block_a_new_explicit_exact_stop() {
    let (database,repository,owner,session,operation)=exact_cancel_fixture().await;
    let legacy=|kind:&str,outcome:&str|AppendNomiRemoteEventParams {owner_user_id:owner.clone(),agent_session_id:session.clone(),event_type:kind.into(),
        payload_json:serde_json::json!({"operation_key_digest":binding_digest("legacy"),"request_digest":binding_digest("legacy request"),"outcome":outcome}).to_string()};
    repository.append_event_once(legacy("session/cancel-requested","requested")).await.unwrap();
    repository.append_event_once(legacy("session/cancel-unknown","unknown")).await.unwrap();
    assert!(repository.append_cancel_request_exact(exact_cancel(&owner,&session,&operation,"new exact",7),Some(&operation),7,1).await.unwrap().inserted);
    let old=repository.find_event_by_operation_key(&owner,&session,"session/cancel-requested",&binding_digest("legacy")).await.unwrap().unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&old.payload_json).unwrap().get("expected_operation_id").is_none());
    assert!(repository.find_event_by_operation_key(&owner,&session,"session/cancel-unknown",&binding_digest("legacy")).await.unwrap().is_some());
    let mut tx=database.pool().begin().await.unwrap();assert!(!nomifun_db::remote_session_admitted_tx(&mut tx,&session).await.unwrap());tx.commit().await.unwrap();
    database.close().await;
}

#[tokio::test]
async fn failed_remote_projection_keeps_its_failure_while_accepting_an_exact_native_cleanup_stop() {
    let (database,repository,owner,session,operation)=exact_cancel_fixture().await;
    repository.set_session_state(&owner,&session,"failed").await.unwrap();
    assert!(repository.append_cancel_request_exact(exact_cancel(&owner,&session,&operation,"stop failed native owner",7),Some(&operation),7,1).await.unwrap().inserted);
    assert_eq!(repository.get_session(&owner,&session).await.unwrap().unwrap().state,"failed");
    let mut tx=database.pool().begin().await.unwrap();assert!(!nomifun_db::remote_session_admitted_tx(&mut tx,&session).await.unwrap());tx.commit().await.unwrap();
    database.close().await;
}

#[tokio::test]
async fn remote_binding_cas_open_idempotency_and_cursor_events() {
    let database = init_database_memory().await.expect("database");
    validate_id_schema_contract(database.pool())
        .await
        .expect("schema contract");
    let pool = database.pool();
    let owner: String = sqlx::query_scalar("SELECT user_id FROM users ORDER BY id LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("installation owner");
    let conversation_id = nomifun_common::ConversationId::new();
    sqlx::query(
        "INSERT INTO agent_sessions \
         (agent_session_id, owner_ref_json, state, title, archived, pinned, \
          agent_binding_json, next_seq, created_at) \
         VALUES (?, json_object('principal_kind','user','principal_id',?), \
                 'live', 'remote', 0, 0, json_object('binding_version',1), 1, 1)",
    )
    .bind(conversation_id.as_str())
    .bind(&owner)
    .execute(pool)
    .await
    .expect("canonical AgentSession");

    let repository = SqliteRemoteBindingRepository::new(pool.clone());
    let remote_binding_id = nomifun_common::generate_id();
    let agent_binding_json = binding_json(1);
    let agent_binding_digest = binding_digest(&agent_binding_json);
    let binding = repository
        .create_binding(CreateRemoteBindingParams {
            remote_binding_id: remote_binding_id.clone(),
            owner_user_id: owner.clone(),
            name: "Remote test".into(),
            agent_binding_json: agent_binding_json.clone(),
            nomi_snapshot_json: r#"{"snapshot":"frozen"}"#.into(),
            provenance_json: r#"{"source":"test","version":1}"#.into(),
            agent_binding_digest: agent_binding_digest.clone(),
            binding_version: 1,
        })
        .await
        .expect("binding");
    assert_eq!(binding.binding_version, 1);

    let opened = repository
        .get_or_create_session(GetOrCreateRemoteSessionParams {
            owner_user_id: owner.clone(),
            remote_binding_id: remote_binding_id.clone(),
            expected_binding_version: 1,
            expected_agent_binding_digest: agent_binding_digest.clone(),
            open_idempotency_key: "open-1".into(),
            agent_session_id: conversation_id.as_str().to_owned(),
            initial_input_digest: None,
        })
        .await
        .expect("open");
    let session = match opened {
        RemoteOpenResult::Created(row) => row,
        RemoteOpenResult::Existing(_) => panic!("first open must create"),
    };
    assert_eq!(session.agent_session_id, conversation_id.as_str());
    assert_eq!(session.nomi_snapshot_json, r#"{"snapshot":"frozen"}"#);
    assert_eq!(session.provenance_json, r#"{"source":"test","version":1}"#);
    assert_eq!(session.state, "opening");

    let retry = repository
        .get_or_create_session(GetOrCreateRemoteSessionParams {
            owner_user_id: owner.clone(),
            remote_binding_id: remote_binding_id.clone(),
            expected_binding_version: 1,
            expected_agent_binding_digest: agent_binding_digest.clone(),
            open_idempotency_key: "open-1".into(),
            agent_session_id: conversation_id.as_str().to_owned(),
            initial_input_digest: None,
        })
        .await
        .expect("idempotent retry");
    assert!(matches!(retry, RemoteOpenResult::Existing(_)));

    let updated_json = binding_json(2);
    let updated_digest = binding_digest(&updated_json);
    let updated = repository
        .update_binding_cas(UpdateRemoteBindingParams {
            owner_user_id: owner.clone(),
            remote_binding_id: remote_binding_id.clone(),
            expected_binding_version: 1,
            expected_agent_binding_digest: agent_binding_digest.clone(),
            name: "Remote test v2".into(),
            agent_binding_json: updated_json,
            nomi_snapshot_json: r#"{"snapshot":"frozen-v2"}"#.into(),
            provenance_json: r#"{"source":"test","version":2}"#.into(),
            agent_binding_digest: updated_digest,
            binding_version: 2,
        })
        .await
        .expect("CAS update");
    assert_eq!(updated.binding_version, 2);
    assert!(repository
        .update_binding_cas(UpdateRemoteBindingParams {
            owner_user_id: owner.clone(),
            remote_binding_id,
            expected_binding_version: 1,
            expected_agent_binding_digest: agent_binding_digest,
            name: "stale".into(),
            agent_binding_json: binding_json(2),
            nomi_snapshot_json: r#"{"snapshot":"stale"}"#.into(),
            provenance_json: r#"{"source":"stale"}"#.into(),
            agent_binding_digest: binding_digest(&binding_json(2)),
            binding_version: 2,
        })
        .await
        .is_err());

    let first = repository
        .append_event(AppendNomiRemoteEventParams {
            owner_user_id: owner.clone(),
            agent_session_id: conversation_id.as_str().to_owned(),
            event_type: "opened".into(),
            payload_json: r#"{"ok":true}"#.into(),
        })
        .await
        .expect("first event");
    let second = repository
        .append_event(AppendNomiRemoteEventParams {
            owner_user_id: owner.clone(),
            agent_session_id: conversation_id.as_str().to_owned(),
            event_type: "ready".into(),
            payload_json: r#"{"state":"ready"}"#.into(),
        })
        .await
        .expect("second event");
    assert_eq!((first.seq, second.seq), (1, 2));
    let page = repository
        .read_events(&owner, conversation_id.as_str(), 0, 10)
        .await
        .expect("event page");
    assert_eq!(page.next_cursor, 2);
    assert_eq!(
        page.events.iter().map(|event| event.seq).collect::<Vec<_>>(),
        vec![1, 2]
    );

    let transition_input = TransitionNomiRemoteSessionParams {
        owner_user_id: owner.clone(),
        agent_session_id: conversation_id.as_str().to_owned(),
        expected_state: "opening".into(),
        next_state: "ready".into(),
        event_type: "session/ready".into(),
        payload_json: r#"{"operation_key_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","outcome":"ready"}"#.into(),
    };
    let transitioned = repository
        .transition_session_state_and_append_event(transition_input.clone())
        .await
        .expect("atomic state/event transition");
    assert!(transitioned.changed);
    assert_eq!(transitioned.session.state, "ready");
    assert_eq!(transitioned.event.event_type, "session/ready");

    let transition_replay = repository
        .transition_session_state_and_append_event(transition_input)
        .await
        .expect("atomic transition replay");
    assert!(!transition_replay.changed);
    assert_eq!(transition_replay.event.event_id, transitioned.event.event_id);

    let transition_payload_conflict = repository
        .transition_session_state_and_append_event(
            TransitionNomiRemoteSessionParams {
                owner_user_id: owner.clone(),
                agent_session_id: conversation_id.as_str().to_owned(),
                expected_state: "ready".into(),
                next_state: "ready".into(),
                event_type: "session/ready".into(),
                payload_json: r#"{"operation_key_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","outcome":"different"}"#.into(),
            },
        )
        .await
        .expect_err("reusing a transition key with a different payload must conflict");
    assert!(matches!(
        transition_payload_conflict,
        nomifun_db::DbError::Conflict(_)
    ));

    let event_input = AppendNomiRemoteEventParams {
        owner_user_id: owner.clone(),
        agent_session_id: conversation_id.as_str().to_owned(),
        event_type: "session/cancel-requested".into(),
        payload_json: r#"{"operation_key_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","request_digest":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}"#.into(),
    };
    let first_once = repository
        .append_event_once(event_input.clone())
        .await
        .expect("first operation event");
    let replay_once = repository
        .append_event_once(AppendNomiRemoteEventParams {
            owner_user_id: owner.clone(),
            agent_session_id: conversation_id.as_str().to_owned(),
            event_type: "session/cancel-requested".into(),
            payload_json: r#"{"request_digest":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","operation_key_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"#.into(),
        })
        .await
        .expect("replayed operation event");
    assert!(first_once.inserted);
    assert!(!replay_once.inserted);
    assert_eq!(first_once.event.event_id, replay_once.event.event_id);

    let event_payload_conflict = repository
        .append_event_once(AppendNomiRemoteEventParams {
            owner_user_id: owner.clone(),
            agent_session_id: conversation_id.as_str().to_owned(),
            event_type: "session/cancel-requested".into(),
            payload_json: r#"{"operation_key_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","request_digest":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"}"#.into(),
        })
        .await
        .expect_err("reusing an event key with a different payload must conflict");
    assert!(matches!(
        event_payload_conflict,
        nomifun_db::DbError::Conflict(_)
    ));

    let accepted = repository
        .append_event_once(AppendNomiRemoteEventParams {
            owner_user_id: owner.clone(),
            agent_session_id: conversation_id.as_str().to_owned(),
            event_type: "turn/accepted".into(),
            payload_json: r#"{"operation_key_digest":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","message_id":"m1","outcome":"accepted"}"#.into(),
        })
        .await
        .expect("accepted turn event");
    assert!(accepted.inserted);
    let completed = repository
        .append_event_once(AppendNomiRemoteEventParams {
            owner_user_id: owner.clone(),
            agent_session_id: conversation_id.as_str().to_owned(),
            event_type: "turn/completed".into(),
            payload_json: r#"{"outcome":"completed","message_id":"m1","operation_key_digest":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","result_ok":true}"#.into(),
        })
        .await
        .expect("accepted turn may settle to completed");
    assert!(completed.inserted);

    repository
        .append_event_once(AppendNomiRemoteEventParams {
            owner_user_id: owner.clone(),
            agent_session_id: conversation_id.as_str().to_owned(),
            event_type: "turn/unknown".into(),
            payload_json: r#"{"operation_key_digest":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","outcome":"unknown"}"#.into(),
        })
        .await
        .expect("unknown terminal event");
    let late_completion = repository
        .append_event_once(AppendNomiRemoteEventParams {
            owner_user_id: owner.clone(),
            agent_session_id: conversation_id.as_str().to_owned(),
            event_type: "turn/completed".into(),
            payload_json: r#"{"operation_key_digest":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","outcome":"completed","result_ok":true}"#.into(),
        })
        .await
        .expect_err("late completion must not overwrite an unknown terminal fact");
    assert!(matches!(late_completion, nomifun_db::DbError::Conflict(_)));

    let event = repository
        .find_event_by_operation_key(
            &owner,
            conversation_id.as_str(),
            "session/ready",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .await
        .expect("find transition event")
        .expect("transition event exists");
    assert_eq!(event.event_id, transitioned.event.event_id);
    assert_eq!(
        repository
            .current_event_cursor(&owner, conversation_id.as_str())
            .await
            .expect("current event cursor"),
        7
    );

    let ready = repository
        .set_session_state(&owner, conversation_id.as_str(), "ready")
        .await
        .expect("state");
    assert_eq!(ready.state, "ready");
    let cancelled = repository
        .set_session_state(&owner, conversation_id.as_str(), "cancelled")
        .await
        .expect("ready -> cancelled");
    assert_eq!(cancelled.state, "cancelled");
    let terminal_replay = repository
        .set_session_state(&owner, conversation_id.as_str(), "cancelled")
        .await
        .expect("cancelled replay");
    assert_eq!(terminal_replay.state, "cancelled");
    let illegal_transition = repository
        .set_session_state(&owner, conversation_id.as_str(), "ready")
        .await
        .expect_err("terminal state must not regress");
    assert!(matches!(illegal_transition, nomifun_db::DbError::Conflict(_)));
}
