use super::*;
use nomifun_common::UserId;
use nomifun_db::{Database, init_database_memory_with_owner};

async fn fixture()->(Database,OfficeHubStore,String,OfficeConnection,OfficeActor){
    let owner=UserId::new();let db=init_database_memory_with_owner(owner.clone()).await.unwrap();
    let store=OfficeHubStore::new(db.pool().clone());
    let connection=OfficeConnection{id:new_id(),name:"Feishu".into(),enabled:true,profile:Some("nomifun".into()),executable:None,created_at:String::new(),updated_at:String::new()};
    store.save_connection(owner.as_str(),&connection).await.unwrap();
    let actor=OfficeActor{app_id:"cli_test".into(),user_id:"ou_owner".into(),name:"Owner".into()};
    (db,store,owner.into_string(),connection,actor)
}
fn task(id:&str,assignee:&str)->OfficeMutation{
    OfficeMutation::TaskCreate{client_item_id:id.into(),title:format!("Task {id}"),assignee_id:assignee.into(),
        assignee_name:Some(assignee.into()),due:None,tasklist_id:None}
}
fn base(id:&str)->OfficeMutation{
    OfficeMutation::BaseUpdate{client_item_id:id.into(),base_token:"base-real".into(),table_id:"table-real".into(),record_id:"record-real".into(),
        fields:BTreeMap::from([("fldStatus".into(),json!(["Contacted"]))]),before:BTreeMap::from([("fldStatus".into(),json!(["New"]))])}
}
fn request(actor:&OfficeActor,items:Vec<OfficeMutation>)->ExecuteRequest{
    ExecuteRequest{request_id:new_id(),expected_actor:actor.clone(),profile:"nomifun".into(),items}
}
async fn submit(store:&OfficeHubStore,owner:&str,c:&OfficeConnection,a:&OfficeActor,items:Vec<OfficeMutation>)->OfficeOperation{
    store.create_submitted(owner,&c.id,a,c.profile.as_deref().unwrap(),&request(a,items)).await.unwrap()
}
async fn finish(store:&OfficeHubStore,owner:&str,op:&OfficeOperation,index:usize,status:ItemStatus)->OfficeOperation{
    let item=&op.items[index];let attempt=store.start_attempt(owner,&op.id,&item.id).await.unwrap();
    store.finish_attempt(owner,&op.id,&item.id,&attempt.id,status,Some("original receipt"),Some(&json!({"receipt":attempt.id})),None,None).await.unwrap()
}

#[tokio::test]
async fn submitted_request_is_exact_owner_scoped_and_idempotent_independent_of_display_names(){
    let (db,store,owner,c,a)=fixture().await;
    let mut req=request(&a,vec![task("one","ou_a")]);
    let op=store.create_submitted(&owner,&c.id,&a,"nomifun",&req).await.unwrap();
    assert_eq!(op.status,OperationStatus::Queued);
    assert!(op.items[0].attempts.is_empty());
    let mut renamed=a.clone();renamed.name="renamed display label".into();req.expected_actor=renamed.clone();
    assert_eq!(store.create_submitted(&owner,&c.id,&renamed,"nomifun",&req).await.unwrap().id,op.id);
    assert_eq!(store.get_by_request(&owner,&req.request_id).await.unwrap().unwrap().id,op.id);
    assert!(store.get_by_request(&new_id(),&req.request_id).await.unwrap().is_none());
    req.items=vec![task("one","ou_b")];
    assert_eq!(store.create_submitted(&owner,&c.id,&a,"nomifun",&req).await.unwrap_err().related_operation_id,Some(op.id));
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM office_operations").fetch_one(db.pool()).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM office_operation_receipts").fetch_one(db.pool()).await.unwrap(),0);
    db.close().await;
}

#[tokio::test]
async fn pending_base_target_stays_fenced_after_identity_switch_and_is_visible_to_the_owner(){
    let (db,store,owner,mut c,a)=fixture().await;
    let op=submit(&store,&owner,&c,&a,vec![base("one")]).await;
    let op=finish(&store,&owner,&op,0,ItemStatus::Unknown).await;
    let key=op.items[0].target_key.clone();
    assert_eq!(store.pending_targets(&owner,&[key.clone()]).await.unwrap()[&key].0,op.id);
    c.profile=Some("another".into());store.save_connection(&owner,&c).await.unwrap();
    let other=OfficeActor{app_id:"another-app".into(),user_id:"ou_other".into(),name:"Other".into()};
    let mut req=request(&other,vec![base("different-client")]);req.profile="another".into();
    assert_eq!(store.create_submitted(&owner,&c.id,&other,"another",&req).await.unwrap_err().related_operation_id,Some(op.id));
    assert!(store.pending_targets(&new_id(),&[key]).await.unwrap().is_empty());
    db.close().await;
}

#[tokio::test]
async fn creation_identity_cannot_duplicate_success_or_unknown_but_not_executed_can_be_resubmitted(){
    let (db,store,owner,c,a)=fixture().await;
    for (id,status,allowed) in [("success",ItemStatus::Succeeded,false),("unknown",ItemStatus::Unknown,false),("not-executed",ItemStatus::NotExecuted,true)]{
        let op=submit(&store,&owner,&c,&a,vec![task(id,"ou_a")]).await;
        let op=finish(&store,&owner,&op,0,status).await;
        let result=store.create_submitted(&owner,&c.id,&a,"nomifun",&request(&a,vec![task(id,"ou_a")])).await;
        if allowed{assert!(result.is_ok())}else{assert_eq!(result.unwrap_err().related_operation_id,Some(op.id))}
    }
    db.close().await;
}

#[tokio::test]
async fn creation_nonce_cannot_be_reused_by_switching_actor_or_profile(){
    let (db,store,owner,mut c,a)=fixture().await;
    for (nonce,status) in [("old-success",ItemStatus::Succeeded),("old-unknown",ItemStatus::Unknown)]{
        let op=submit(&store,&owner,&c,&a,vec![task(nonce,"ou_a")]).await;finish(&store,&owner,&op,0,status).await;
        c.profile=Some("other".into());store.save_connection(&owner,&c).await.unwrap();
        let other=OfficeActor{app_id:"other-app".into(),user_id:"ou_other".into(),name:"Other".into()};
        let mut req=request(&other,vec![task(nonce,"ou_other")]);req.profile="other".into();
        assert_eq!(store.create_submitted(&owner,&c.id,&other,"other",&req).await.unwrap_err().related_operation_id,Some(op.id));
        c.profile=Some("nomifun".into());store.save_connection(&owner,&c).await.unwrap();
    }
    db.close().await;
}

#[tokio::test]
async fn partial_outcomes_and_each_original_input_survive_new_request_retry(){
    let (db,store,owner,c,a)=fixture().await;
    let op=submit(&store,&owner,&c,&a,vec![task("success","ou_a"),task("retry","unresolved"),task("unknown","ou_a")]).await;
    finish(&store,&owner,&op,0,ItemStatus::Succeeded).await;
    finish(&store,&owner,&op,1,ItemStatus::NotExecuted).await;
    let original=finish(&store,&owner,&op,2,ItemStatus::Unknown).await;
    assert_eq!(original.status,OperationStatus::Partial);
    let next=submit(&store,&owner,&c,&a,vec![task("retry","ou_resolved")]).await;
    assert_ne!(next.id,original.id);assert_ne!(next.request_id,original.request_id);
    finish(&store,&owner,&next,0,ItemStatus::Succeeded).await;
    let retained=store.get(&owner,&original.id).await.unwrap().unwrap();
    assert_eq!(retained.items[1].mutation,serde_json::to_value(task("retry","unresolved")).unwrap());
    assert_eq!(retained.items[1].attempts[0].status,ItemStatus::NotExecuted);
    let attempt=&retained.items[2].attempts[0];
    assert!(store.finish_attempt(&owner,&retained.id,&retained.items[2].id,&attempt.id,ItemStatus::Succeeded,None,None,None,None).await.is_err());
    assert!(store.start_attempt(&owner,&retained.id,&retained.items[2].id).await.is_err());
    db.close().await;
}

#[tokio::test]
async fn base_acceptance_needs_exact_readback_and_observation_never_rewrites_original_receipt(){
    let (db,store,owner,c,a)=fixture().await;
    let op=submit(&store,&owner,&c,&a,vec![base("base")]).await;
    let attempt=store.start_attempt(&owner,&op.id,&op.items[0].id).await.unwrap();
    assert!(store.finish_attempt(&owner,&op.id,&op.items[0].id,&attempt.id,ItemStatus::Succeeded,None,None,None,None).await.is_err());
    store.finish_attempt(&owner,&op.id,&op.items[0].id,&attempt.id,ItemStatus::AcceptedPendingConfirmation,Some("batch accepted"),Some(&json!({"accepted":true})),None,None).await.unwrap();
    assert!(store.append_observation(&owner,&op.id,&op.items[0].id,ObservationKind::TargetReadback,"wrong record",Some(&json!({"id":"other","fields":{"fldStatus":["Contacted"]}})),true,Some("other"),None).await.is_err());
    let confirmed=store.append_observation(&owner,&op.id,&op.items[0].id,ObservationKind::TargetReadback,"exact values",Some(&json!({"id":"record-real","fields":{"fldStatus":["Contacted"]}})),true,Some("record-real"),None).await.unwrap();
    assert_eq!(confirmed.status,OperationStatus::Succeeded);
    assert_eq!(confirmed.items[0].attempts[0].status,ItemStatus::AcceptedPendingConfirmation);
    assert_eq!(confirmed.items[0].attempts[0].message.as_deref(),Some("batch accepted"));
    assert_eq!(confirmed.items[0].observations.len(),1);
    db.close().await;
}

#[tokio::test]
async fn restart_preserves_unknown_and_only_unstarted_work_becomes_not_executed(){
    let (db,store,owner,c,a)=fixture().await;
    let op=submit(&store,&owner,&c,&a,vec![task("started","ou_a"),task("queued","ou_a")]).await;
    store.start_attempt(&owner,&op.id,&op.items[0].id).await.unwrap();
    assert_eq!(store.recover_interrupted(&owner).await.unwrap(),2);assert_eq!(store.recover_interrupted(&owner).await.unwrap(),0);
    let restored=store.get(&owner,&op.id).await.unwrap().unwrap();
    assert_eq!(restored.items[0].status,ItemStatus::Unknown);assert_eq!(restored.items[0].attempts.len(),1);
    assert_eq!(restored.items[1].status,ItemStatus::NotExecuted);assert!(restored.items[1].attempts.is_empty());
    assert!(store.start_attempt(&owner,&op.id,&op.items[0].id).await.is_err());
    db.close().await;
}

#[tokio::test]
async fn disabling_withdraws_only_queued_work_without_fabricating_cli_attempts(){
    let (db,store,owner,mut c,a)=fixture().await;
    let op=submit(&store,&owner,&c,&a,vec![task("started","ou_a"),task("queued","ou_a")]).await;
    store.start_attempt(&owner,&op.id,&op.items[0].id).await.unwrap();
    c.enabled=false;store.save_connection(&owner,&c).await.unwrap();
    let op=store.get(&owner,&op.id).await.unwrap().unwrap();
    assert_eq!(op.items[0].status,ItemStatus::Running);assert!(op.items[0].observations.is_empty());
    assert_eq!(op.items[1].status,ItemStatus::NotExecuted);assert!(op.items[1].attempts.is_empty());assert_eq!(op.items[1].observations.len(),1);
    assert_eq!(store.stop_unstarted(&owner,&c.id,"repeat withdrawal").await.unwrap(),0);
    db.close().await;
}

#[tokio::test]
async fn old_unknown_inputs_remain_readable_but_cannot_execute_or_gain_false_confirmation(){
    let (db,store,owner,c,a)=fixture().await;
    let op=submit(&store,&owner,&c,&a,vec![task("old","ou_a")]).await;
    let legacy=json!({"kind":"document_update","client_item_id":"old","document_id":"doc-real","revision":"12","content":"legacy append"});
    let save=OfficeMutation::DocumentDelete{client_item_id:"current-delete".into(),document_id:"doc-real".into(),document_type:"docx".into(),title:"Document".into()};
    sqlx::query("UPDATE office_operation_items SET mutation_json=?,target_key=? WHERE item_id=?").bind(legacy.to_string()).bind(target_key(&a,"nomifun",&save).unwrap()).bind(&op.items[0].id).execute(db.pool()).await.unwrap();
    assert!(store.start_attempt(&owner,&op.id,&op.items[0].id).await.is_err());
    sqlx::query("UPDATE office_operation_items SET status='unknown' WHERE item_id=?").bind(&op.items[0].id).execute(db.pool()).await.unwrap();
    let read=store.get(&owner,&op.id).await.unwrap().unwrap();assert_eq!(read.items[0].mutation,legacy);
    assert_eq!(store.list(&owner,30,None).await.unwrap().items[0].items[0].mutation,legacy);
    assert_eq!(store.create_submitted(&owner,&c.id,&a,"nomifun",&request(&a,vec![save])).await.unwrap_err().related_operation_id,Some(op.id.clone()),"a document deletion cannot bypass an existing unknown fence");
    assert!(store.append_observation(&owner,&op.id,&op.items[0].id,ObservationKind::TargetReadback,"text exists",Some(&json!({"id":"doc-real","content":"legacy append"})),true,Some("doc-real"),None).await.is_err());
    db.close().await;
}

#[tokio::test]
async fn thin_navigation_and_unknown_receipts_survive_agent_reset_and_snapshot(){
    let dir=tempfile::tempdir().unwrap();let db=nomifun_db::init_database(&dir.path().join("source.db")).await.unwrap();
    let owner:String=sqlx::query_scalar("SELECT owner_user_id FROM installation_identity").fetch_one(db.pool()).await.unwrap();
    let store=OfficeHubStore::new(db.pool().clone());
    let c=OfficeConnection{id:new_id(),name:"Feishu".into(),enabled:true,profile:Some("nomifun".into()),executable:None,created_at:String::new(),updated_at:String::new()};
    store.save_connection(&owner,&c).await.unwrap();let a=OfficeActor{app_id:"cli_test".into(),user_id:"ou_a".into(),name:"A".into()};
    let op=submit(&store,&owner,&c,&a,vec![base("snapshot")]).await;let op=finish(&store,&owner,&op,0,ItemStatus::Unknown).await;
    let mut recent=OfficeItem::new("doc-real","document","Actual opened document");recent.description=Some("never cache this content".into());recent.fields.insert("remote-value".into(),json!("do not persist"));
    store.remember_recent(&owner,&c.id,&recent).await.unwrap();recent.title="Updated navigation title".into();store.remember_recent(&owner,&c.id,&recent).await.unwrap();
    let text:String=sqlx::query_scalar("SELECT recent_resources_json FROM office_connections").fetch_one(db.pool()).await.unwrap();
    assert!(!text.contains("never cache"));assert!(!text.contains("remote-value"));assert!(text.contains("Updated navigation title"));
    let report=nomifun_db::reset_agent_data(db.pool()).await.unwrap();
    for table in ["office_connections","office_operations","office_operation_items","office_operation_receipts"]{assert!(report.preserved_rows.contains_key(table))}
    let snapshot=dir.path().join("snapshot.db");db.snapshot_into(&snapshot).await.unwrap();
    let copied=nomifun_db::open_database_for_backup(&snapshot).await.unwrap();let copied_store=OfficeHubStore::new(copied.pool().clone());
    assert_eq!(copied_store.get(&owner,&op.id).await.unwrap().unwrap().items[0].status,ItemStatus::Unknown);
    let items=copied_store.recent(&owner,&c.id,20).await.unwrap();assert_eq!(items.len(),1);assert_eq!(items[0].title,recent.title);assert!(items[0].fields.is_empty());assert!(items[0].description.is_none());assert!(items[0].opened_at.is_some());assert!(items[0].modified_at.is_none());
    copied.close().await;db.close().await;
}

#[test]
fn canonical_resource_ids_reject_aliases_that_could_bypass_pending_fences(){
    for invalid in ["https://feishu.invalid/docx/doc-real"," doc-real","doc-real?x=1","../doc-real",""]{assert!(canonical_resource_id(invalid).is_err())}
    assert_eq!(canonical_resource_id("0190F5FE-7C00-7A00-8000-000000000214").unwrap(),"0190f5fe-7c00-7a00-8000-000000000214");
}
