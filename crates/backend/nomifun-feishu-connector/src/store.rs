//! Durable Feishu-connector configuration and business receipts.
//!
//! No Agent transcript, binding or checkpoint is stored here. A mutation may
//! start only after its exact item has entered Running in a committed writer
//! transaction. Only submitted work is durable; unsubmitted forms stay in memory.
//! Unknown targets retain their SQL fence; observations never rewrite attempts.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection, SqlitePool};
use uuid::Uuid;

use crate::{error::FeishuConnectorError, types::*};

const MAX_OPERATION_BYTES: usize = 1024 * 1024;
const MAX_ITEMS: usize = 200;
const PENDING: &str = "('queued','running','unknown','accepted_pending_confirmation')";

#[derive(Clone)]
pub struct FeishuConnectorStore { pool: SqlitePool }

impl FeishuConnectorStore {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    pub async fn connection(&self, owner: &str) -> Result<Option<FeishuConnection>, FeishuConnectorError> {
        let mut conn = self.pool.acquire().await.map_err(FeishuConnectorError::storage)?;
        load_connection(&mut conn, owner).await
    }

    pub async fn active_operation_id(&self, owner: &str, connection_id: &str) -> Result<Option<String>, FeishuConnectorError> {
        sqlx::query_scalar("SELECT operation_id FROM feishu_operations WHERE owner_id=? AND connection_id=? AND status IN ('queued','running') LIMIT 1")
            .bind(owner).bind(connection_id).fetch_optional(&self.pool).await.map_err(FeishuConnectorError::storage)
    }

    pub async fn save_connection(&self, owner: &str, value: &FeishuConnection) -> Result<(), FeishuConnectorError> {
        require_uuid(owner)?; require_uuid(&value.id)?;
        if value.name.trim().is_empty() || value.name.len() > 200 {
            return Err(FeishuConnectorError::invalid("Connection name must be nonempty and at most 200 bytes"));
        }
        if value.profile.as_ref().is_some_and(|v| v.is_empty() || v.len() > 255)
            || value.executable.as_ref().is_some_and(|v| !std::path::Path::new(v).is_absolute()) {
            return Err(FeishuConnectorError::invalid("Invalid CLI profile or executable path"));
        }
        let mut tx = self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        sqlx::query("UPDATE users SET user_id=user_id WHERE user_id=?").bind(owner)
            .execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE user_id=?").bind(owner)
            .fetch_one(&mut *tx).await.map_err(FeishuConnectorError::storage)? != 1 {
            return Err(not_found("Installation owner does not exist"));
        }
        if let Some(old) = load_connection(&mut tx, owner).await? {
            if old.id != value.id { return Err(FeishuConnectorError::conflict("Connection identity cannot be replaced", None)); }
            if old.profile!=value.profile || old.executable!=value.executable {
                if let Some(operation)=sqlx::query_scalar::<_,String>("SELECT operation_id FROM feishu_operations WHERE owner_id=? AND connection_id=? AND status IN ('queued','running') LIMIT 1")
                    .bind(owner).bind(&old.id).fetch_optional(&mut *tx).await.map_err(FeishuConnectorError::storage)?{
                    return Err(FeishuConnectorError::conflict("Wait for the active Feishu operation before changing CLI identity",Some(operation)));
                }
            }
        }
        let now = now_ms();
        sqlx::query("INSERT INTO feishu_connections(connection_id,owner_id,name,enabled,profile,executable,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(owner_id) DO UPDATE SET name=excluded.name,enabled=excluded.enabled,profile=excluded.profile,executable=excluded.executable,updated_at=excluded.updated_at")
            .bind(&value.id).bind(owner).bind(value.name.trim()).bind(value.enabled)
            .bind(&value.profile).bind(&value.executable).bind(now).bind(now)
            .execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        if !value.enabled {
            stop_unstarted_tx(&mut tx,owner,&value.id,
                "Feishu connection was disabled before this item's committed dispatch fence; no CLI invocation started.").await?;
        }
        tx.commit().await.map_err(FeishuConnectorError::storage)
    }

    /// Withdrawal is local evidence only while this connection is disabled.
    /// It never creates an attempt or changes an already-crossed start fence.
    pub async fn stop_unstarted(&self,owner:&str,connection_id:&str,message:&str)->Result<usize,FeishuConnectorError>{
        let mut tx=self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        sqlx::query("UPDATE feishu_connections SET updated_at=updated_at WHERE owner_id=? AND connection_id=?")
            .bind(owner).bind(connection_id).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        let stopped=stop_unstarted_tx(&mut tx,owner,connection_id,message).await?;
        tx.commit().await.map_err(FeishuConnectorError::storage)?;
        Ok(stopped)
    }

    /// Presentation metadata from the same admitted-item facts that enforce
    /// write exclusion. This is not another pending-operation ledger.
    pub async fn pending_targets(&self,owner:&str,keys:&[String])->Result<BTreeMap<String,(String,ItemStatus)>,FeishuConnectorError>{
        if keys.len()>MAX_ITEMS || keys.iter().any(|key|key.len()!=64||!key.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))){
            return Err(FeishuConnectorError::invalid("Pending-target lookup requires at most 200 canonical target digests"));
        }
        if keys.is_empty(){return Ok(BTreeMap::new())}
        let placeholders=vec!["?";keys.len()].join(",");
        let sql=format!("SELECT item.target_key,item.operation_id,item.status FROM feishu_operation_items item JOIN feishu_operations operation ON operation.operation_id=item.operation_id WHERE operation.owner_id=? AND item.status IN {PENDING} AND item.target_key IN ({placeholders})");
        let mut query=sqlx::query(&sql).bind(owner);
        for key in keys{query=query.bind(key)}
        let mut found=BTreeMap::new();
        for row in query.fetch_all(&self.pool).await.map_err(FeishuConnectorError::storage)?{
            found.insert(row.try_get("target_key").map_err(FeishuConnectorError::storage)?,
                (row.try_get("operation_id").map_err(FeishuConnectorError::storage)?,
                 parse_state(&row.try_get::<String,_>("status").map_err(FeishuConnectorError::storage)?)?));
        }
        Ok(found)
    }

    /// Commit exact reviewed input and its cross-request target fences together.
    /// A caller may recover a lost response by request identity without dispatch.
    pub async fn create_submitted(
        &self, owner: &str, connection_id: &str, actor: &FeishuActor, profile: &str,
        request: &ExecuteRequest,
    ) -> Result<FeishuOperation, FeishuConnectorError> {
        require_uuid(owner)?; require_uuid(connection_id)?; require_uuid(&request.request_id)?;
        if request.items.is_empty() || request.items.len() > MAX_ITEMS || profile.is_empty() || profile.len() > 255
            || actor.app_id.is_empty() || actor.user_id.is_empty()
            || request.profile != profile || request.expected_actor.app_id != actor.app_id
            || request.expected_actor.user_id != actor.user_id {
            return Err(FeishuConnectorError::invalid("Invalid submitted input, profile or expected actor"));
        }
        bounded_json(&request.items)?;
        let mut clients = BTreeSet::new(); let mut targets = BTreeSet::new();
        for item in &request.items {
            let client = item.client_item_id();
            if client.is_empty() || client.len() > 255 || !clients.insert(client) {
                return Err(FeishuConnectorError::invalid("Every item needs a distinct client item identity"));
            }
            if !targets.insert(target_key(actor, profile, item)?) {
                return Err(FeishuConnectorError::invalid("A request cannot mutate the same target twice"));
            }
        }
        let digest = hash_json(&json!({"connection":connection_id,"actor":{"app_id":actor.app_id,"user_id":actor.user_id},"profile":profile,"items":request.items}))?;
        let mut tx = self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        sqlx::query("UPDATE feishu_connections SET updated_at=updated_at WHERE owner_id=? AND connection_id=?")
            .bind(owner).bind(connection_id).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        if let Some((id,existing_digest)) = sqlx::query_as::<_,(String,String)>("SELECT operation_id,digest FROM feishu_operations WHERE owner_id=? AND request_id=?")
            .bind(owner).bind(&request.request_id).fetch_optional(&mut *tx).await.map_err(FeishuConnectorError::storage)? {
            if existing_digest != digest { return Err(FeishuConnectorError::conflict("Request identity already belongs to different immutable input", Some(id))); }
            return required_operation(&mut tx,owner,&id).await;
        }
        let connection=require_connection(&mut tx, owner, connection_id, true).await?;
        if connection.profile.as_deref()!=Some(profile) {return Err(FeishuConnectorError::conflict("The selected CLI profile changed before submission",None))}
        for mutation in &request.items {
            let target=target_key(actor,profile,mutation)?;
            check_overlap(&mut tx,&target,"").await?;
            if matches!(mutation,FeishuMutation::DocumentCreate{..}) {
                if let Some(id)=sqlx::query_scalar::<_,String>("SELECT item.operation_id FROM feishu_operation_items item JOIN feishu_operations op ON op.operation_id=item.operation_id WHERE op.owner_id=? AND item.client_item_id=? AND json_extract(item.mutation_json,'$.kind')=? AND item.status<>'not_executed' LIMIT 1")
                    .bind(owner).bind(mutation.client_item_id()).bind(mutation.kind()).fetch_optional(&mut *tx).await.map_err(FeishuConnectorError::storage)? {
                    return Err(FeishuConnectorError::conflict("This creation identity already has submitted work; open the original result",Some(id)))
                }
            }
        }
        let id = new_id();
        sqlx::query("INSERT INTO feishu_operations(operation_id,owner_id,connection_id,request_id,actor_json,profile,digest,status) VALUES (?,?,?,?,?,?,?,'queued')")
            .bind(&id).bind(owner).bind(connection_id).bind(&request.request_id).bind(bounded_json(actor)?)
            .bind(profile).bind(&digest).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        for (ordinal, mutation) in request.items.iter().enumerate() {
            sqlx::query("INSERT INTO feishu_operation_items(item_id,operation_id,client_item_id,ordinal,mutation_json,target_key,status) VALUES (?,?,?,?,?,?,'queued')")
                .bind(new_id()).bind(&id).bind(mutation.client_item_id()).bind(ordinal as i64)
                .bind(bounded_json(mutation)?).bind(target_key(actor, profile, mutation)?)
                .execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        }
        let result=required_operation(&mut tx,owner,&id).await?;
        tx.commit().await.map_err(FeishuConnectorError::storage)?;
        Ok(result)
    }

    pub async fn get_by_request(&self,owner:&str,request_id:&str)->Result<Option<FeishuOperation>,FeishuConnectorError>{
        require_uuid(request_id)?;
        let mut tx=self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        let id=sqlx::query_scalar::<_,String>("SELECT operation_id FROM feishu_operations WHERE owner_id=? AND request_id=?")
            .bind(owner).bind(request_id).fetch_optional(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        let result=match id {Some(id)=>load_operation(&mut tx,owner,&id).await?,None=>None};
        tx.commit().await.map_err(FeishuConnectorError::storage)?;
        Ok(result)
    }

    pub async fn get(&self, owner: &str, id: &str) -> Result<Option<FeishuOperation>, FeishuConnectorError> {
        let mut tx = self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        let result=load_operation(&mut tx,owner,id).await?;
        tx.commit().await.map_err(FeishuConnectorError::storage)?;
        Ok(result)
    }

    /// Called before dispatch. Its committed Running row is the start fence.
    pub async fn start_attempt(&self, owner: &str, id: &str, item_id: &str) -> Result<OperationAttempt, FeishuConnectorError> {
        let mut tx = self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        lock_operation(&mut tx, owner, id).await?;
        let op = required_operation(&mut tx, owner, id).await?;
        require_operation_connection(&mut tx,owner,&op).await?;
        let item = find_item(&op,item_id)?;
        if item.status != ItemStatus::Queued { return Err(FeishuConnectorError::conflict("This item is not queued; original execution must not be replayed", Some(id.into()))); }
        // Historical unknown inputs remain readable evidence, never executable variants.
        serde_json::from_value::<FeishuMutation>(item.mutation.clone())
            .map_err(|_|FeishuConnectorError::conflict("This historical input cannot be executed by the current workbench",Some(id.into())))?;
        check_overlap(&mut tx, &item.target_key, &item.id).await?;
        let number = u32::try_from(item.attempts.len()+1).map_err(FeishuConnectorError::storage)?;
        let started_at=now_ms();
        let attempt = OperationAttempt { id:new_id(), number, client_token:new_id(), mutation:item.mutation.clone(),
            status:ItemStatus::Running, started_at:stamp(started_at)?, finished_at:None, message:None, result:None };
        sqlx::query("INSERT INTO feishu_operation_receipts(receipt_id,item_id,record_kind,number,client_token,mutation_json,status,occurred_at) VALUES (?,? ,'attempt',?,?,?,'running',?)")
            .bind(&attempt.id).bind(item_id).bind(i64::from(number)).bind(&attempt.client_token)
            .bind(bounded_json(&attempt.mutation)?).bind(started_at).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        sqlx::query("UPDATE feishu_operation_items SET status='running',message=NULL WHERE item_id=? AND status='queued'")
            .bind(item_id).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        update_summary(&mut tx,id).await?;
        tx.commit().await.map_err(FeishuConnectorError::storage)?;
        Ok(attempt)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_attempt(
        &self, owner: &str, id: &str, item_id: &str, attempt_id: &str, status: ItemStatus,
        message: Option<&str>, result: Option<&Value>, remote_id: Option<&str>, remote_url: Option<&str>,
    ) -> Result<FeishuOperation, FeishuConnectorError> {
        if matches!(status,ItemStatus::Queued|ItemStatus::Running) {
            return Err(FeishuConnectorError::invalid("Only a terminal original outcome may settle an attempt"));
        }
        let result_json = result.map(bounded_json).transpose()?;
        let mut tx = self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        lock_operation(&mut tx,owner,id).await?;
        let op = required_operation(&mut tx,owner,id).await?;
        let item = find_item(&op,item_id)?;
        let attempt = item.attempts.last().ok_or_else(|| FeishuConnectorError::invalid("Item has no original attempt"))?;
        if attempt.id != attempt_id { return Err(FeishuConnectorError::conflict("The result belongs to a different attempt",Some(id.into()))); }
        if attempt.status != ItemStatus::Running {
            if attempt.status == status && attempt.message.as_deref() == message && attempt.result.as_ref() == result {
                return Ok(op);
            }
            return Err(FeishuConnectorError::conflict("An original terminal receipt is immutable; append a correlated observation",Some(id.into())));
        }
        if status==ItemStatus::AcceptedPendingConfirmation && !(item.mutation.get("kind").and_then(Value::as_str)==Some("base_update")) {
            return Err(FeishuConnectorError::invalid("Only a Base update has batch-accepted confirmation semantics"));
        }
        if status==ItemStatus::Succeeded && (item.mutation.get("kind").and_then(Value::as_str)==Some("base_update")) {
            return Err(FeishuConnectorError::invalid("Base batch responses require accepted-pending-confirmation followed by a target readback"));
        }
        sqlx::query("UPDATE feishu_operation_receipts SET status=?,settled_at=?,message=?,data_json=? WHERE receipt_id=? AND record_kind='attempt' AND status='running'")
            .bind(wire(&status)?).bind(now_ms()).bind(message).bind(result_json).bind(attempt_id)
            .execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        sqlx::query("UPDATE feishu_operation_items SET status=?,message=?,remote_id=?,remote_url=? WHERE item_id=?")
            .bind(wire(&status)?).bind(message).bind(remote_id).bind(remote_url).bind(item_id)
            .execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        update_summary(&mut tx,id).await?;
        let result = required_operation(&mut tx,owner,id).await?;
        tx.commit().await.map_err(FeishuConnectorError::storage)?;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn append_observation(
        &self, owner: &str, id: &str, item_id: &str, kind: ObservationKind, message: &str,
        data: Option<&Value>, confirmed: bool, remote_id: Option<&str>, remote_url: Option<&str>,
    ) -> Result<FeishuOperation, FeishuConnectorError> {
        let mut tx = self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        lock_operation(&mut tx,owner,id).await?;
        let op = required_operation(&mut tx,owner,id).await?;
        let item = find_item(&op,item_id)?;
        if confirmed {
            if !matches!(item.status,ItemStatus::Unknown|ItemStatus::AcceptedPendingConfirmation) {
                return Err(FeishuConnectorError::conflict("Only an unresolved original operation can be confirmed",Some(id.into())));
            }
            match kind {
                ObservationKind::OriginalReceipt => {
                    let attempt = item.attempts.last().ok_or_else(|| FeishuConnectorError::invalid("No original attempt identity"))?;
                    if data.and_then(|v|v.get("attempt_id")).and_then(Value::as_str) != Some(attempt.id.as_str()) || remote_id.is_none_or(|value|value.trim().is_empty()) {
                        return Err(FeishuConnectorError::invalid("An original receipt must name the exact attempt and returned remote identity"));
                    }
                }
                ObservationKind::TargetReadback if target_readback_confirms(&item.mutation,data,remote_id) => {},
                _ => return Err(FeishuConnectorError::invalid("This observation cannot prove creation or target state")),
            }
        }
        sqlx::query("INSERT INTO feishu_operation_receipts(receipt_id,item_id,record_kind,message,data_json,occurred_at) VALUES (?,?,'observation',?,?,?)")
            .bind(new_id()).bind(item_id).bind(message)
            .bind(bounded_json(&json!({"kind":kind,"confirmed":confirmed,"data":data}))?).bind(now_ms())
            .execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        if confirmed {
            sqlx::query("UPDATE feishu_operation_items SET status='succeeded',message=?,remote_id=COALESCE(?,remote_id),remote_url=COALESCE(?,remote_url) WHERE item_id=?")
                .bind(message).bind(remote_id).bind(remote_url).bind(item_id)
                .execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        }
        update_summary(&mut tx,id).await?;
        let result = required_operation(&mut tx,owner,id).await?;
        tx.commit().await.map_err(FeishuConnectorError::storage)?;
        Ok(result)
    }

    /// Startup-only, after acquiring the installation's single-server lock.
    /// Running has no final proof and becomes Unknown. Queued has not crossed
    /// start_attempt's committed dispatch fence and is explicitly NotExecuted.
    pub async fn recover_interrupted(&self, owner:&str)->Result<usize,FeishuConnectorError>{
        let mut tx=self.pool.begin().await.map_err(FeishuConnectorError::storage)?;
        sqlx::query("UPDATE feishu_operations SET status=status WHERE owner_id=?")
            .bind(owner).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        let ids=sqlx::query_scalar::<_,String>("SELECT operation_id FROM feishu_operations WHERE owner_id=? AND status IN ('queued','running')")
            .bind(owner).fetch_all(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
        let mut recovered=0;
        for id in ids {
            let op=required_operation(&mut tx,owner,&id).await?;
            for item in op.items {
                let (status,message)=match item.status {
                    ItemStatus::Running=>(ItemStatus::Unknown,"Host restarted without an original terminal receipt; do not replay."),
                    ItemStatus::Queued=>(ItemStatus::NotExecuted,"Host restarted before the item's committed dispatch fence."),
                    _=>continue,
                };
                if status==ItemStatus::Unknown {
                    sqlx::query("UPDATE feishu_operation_receipts SET status='unknown',settled_at=?,message=? WHERE item_id=? AND record_kind='attempt' AND status='running'")
                        .bind(now_ms()).bind(message).bind(&item.id).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
                }
                sqlx::query("UPDATE feishu_operation_items SET status=?,message=? WHERE item_id=?")
                    .bind(wire(&status)?).bind(message).bind(&item.id).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
                sqlx::query("INSERT INTO feishu_operation_receipts(receipt_id,item_id,record_kind,message,data_json,occurred_at) VALUES (?,?,'observation',?,?,?)")
                    .bind(new_id()).bind(&item.id).bind(message).bind(bounded_json(&json!({"kind":"inconclusive","confirmed":false,"data":null}))?).bind(now_ms()).execute(&mut *tx).await.map_err(FeishuConnectorError::storage)?;
                recovered+=1;
            }
            update_summary(&mut tx,&id).await?;
        }
        tx.commit().await.map_err(FeishuConnectorError::storage)?;
        Ok(recovered)
    }
}

async fn stop_unstarted_tx(conn:&mut SqliteConnection,owner:&str,connection_id:&str,message:&str)->Result<usize,FeishuConnectorError>{
    if message.trim().is_empty() || message.len()>4096 {
        return Err(FeishuConnectorError::invalid("A local withdrawal needs a nonempty message of at most 4096 bytes"));
    }
    if require_connection(conn,owner,connection_id,false).await?.enabled {
        return Err(FeishuConnectorError::conflict("Disable the Feishu connection before withdrawing unstarted items",None));
    }
    let rows=sqlx::query_as::<_,(String,String)>("SELECT item.operation_id,item.item_id FROM feishu_operation_items item JOIN feishu_operations operation ON operation.operation_id=item.operation_id WHERE operation.owner_id=? AND operation.connection_id=? AND item.status='queued'")
        .bind(owner).bind(connection_id).fetch_all(&mut *conn).await.map_err(FeishuConnectorError::storage)?;
    let mut operations=BTreeSet::new();
    for (operation,item) in &rows {
        sqlx::query("UPDATE feishu_operation_items SET status='not_executed',message=? WHERE item_id=? AND status='queued'")
            .bind(message).bind(item).execute(&mut *conn).await.map_err(FeishuConnectorError::storage)?;
        sqlx::query("INSERT INTO feishu_operation_receipts(receipt_id,item_id,record_kind,message,data_json,occurred_at) VALUES (?,?,'observation',?,?,?)")
            .bind(new_id()).bind(item).bind(message)
            .bind(bounded_json(&json!({"kind":"error","confirmed":false,"data":{"local":true,"reason":"connection_disabled","not_executed":true}}))?).bind(now_ms())
            .execute(&mut *conn).await.map_err(FeishuConnectorError::storage)?;
        operations.insert(operation);
    }
    for operation in operations { update_summary(conn,operation).await?; }
    Ok(rows.len())
}

async fn load_connection(conn:&mut SqliteConnection,owner:&str)->Result<Option<FeishuConnection>,FeishuConnectorError>{
    let row=sqlx::query("SELECT connection_id,name,enabled,profile,executable,created_at,updated_at FROM feishu_connections WHERE owner_id=?")
        .bind(owner).fetch_optional(&mut *conn).await.map_err(FeishuConnectorError::storage)?;
    row.map(|r|Ok(FeishuConnection{id:r.try_get("connection_id").map_err(FeishuConnectorError::storage)?,
        name:r.try_get("name").map_err(FeishuConnectorError::storage)?,enabled:r.try_get::<i64,_>("enabled").map_err(FeishuConnectorError::storage)?==1,
        profile:r.try_get("profile").map_err(FeishuConnectorError::storage)?,executable:r.try_get("executable").map_err(FeishuConnectorError::storage)?,
        created_at:stamp(r.try_get("created_at").map_err(FeishuConnectorError::storage)?)?,updated_at:stamp(r.try_get("updated_at").map_err(FeishuConnectorError::storage)?)?})).transpose()
}

async fn require_connection(conn:&mut SqliteConnection,owner:&str,id:&str,enabled:bool)->Result<FeishuConnection,FeishuConnectorError>{
    let connection=load_connection(conn,owner).await?.filter(|v|v.id==id).ok_or_else(||not_found("Feishu connection not found for this owner"))?;
    if enabled&&!connection.enabled{return Err(FeishuConnectorError::conflict("Feishu connection is disabled; original receipts are retained",None))}
    Ok(connection)
}
async fn require_operation_connection(conn:&mut SqliteConnection,owner:&str,op:&FeishuOperation)->Result<(),FeishuConnectorError>{
    let connection=require_connection(conn,owner,&op.connection_id,true).await?;
    if connection.profile.as_deref()!=Some(op.profile.as_str()){
        return Err(FeishuConnectorError::conflict("The selected CLI profile changed after submission; inspect the original operation",Some(op.id.clone())));
    }
    Ok(())
}
async fn lock_operation(conn:&mut SqliteConnection,owner:&str,id:&str)->Result<(),FeishuConnectorError>{
    if sqlx::query("UPDATE feishu_operations SET status=status WHERE owner_id=? AND operation_id=?")
        .bind(owner).bind(id).execute(&mut *conn).await.map_err(FeishuConnectorError::storage)?.rows_affected()!=1 {
        return Err(not_found("Feishu operation not found"));
    }
    Ok(())
}
async fn required_operation(conn:&mut SqliteConnection,owner:&str,id:&str)->Result<FeishuOperation,FeishuConnectorError>{
    load_operation(conn,owner,id).await?.ok_or_else(||not_found("Feishu operation not found"))
}
fn find_item<'a>(op:&'a FeishuOperation,id:&str)->Result<&'a OperationItem,FeishuConnectorError>{
    op.items.iter().find(|i|i.id==id).ok_or_else(||not_found("Feishu operation item not found"))
}
async fn check_overlap(conn:&mut SqliteConnection,key:&str,item_id:&str)->Result<(),FeishuConnectorError>{
    let sql=format!("SELECT operation_id FROM feishu_operation_items WHERE target_key=? AND item_id<>? AND status IN {PENDING} LIMIT 1");
    if let Some(id)=sqlx::query_scalar::<_,String>(&sql).bind(key).bind(item_id).fetch_optional(&mut *conn).await.map_err(FeishuConnectorError::storage)?{
        return Err(FeishuConnectorError::conflict("This target has an admitted or unresolved Feishu operation; inspect the original result before another write",Some(id)));
    }
    Ok(())
}
async fn update_summary(conn:&mut SqliteConnection,id:&str)->Result<(),FeishuConnectorError>{
    let statuses=sqlx::query_scalar::<_,String>("SELECT status FROM feishu_operation_items WHERE operation_id=? ORDER BY ordinal")
        .bind(id).fetch_all(&mut *conn).await.map_err(FeishuConnectorError::storage)?;
    let statuses=statuses.iter().map(|s|parse_state::<ItemStatus>(s)).collect::<Result<Vec<_>,_>>()?;
    let status=aggregate(&statuses);
    sqlx::query("UPDATE feishu_operations SET status=? WHERE operation_id=?")
        .bind(wire(&status)?).bind(id).execute(&mut *conn).await.map_err(FeishuConnectorError::storage)?;
    Ok(())
}
fn aggregate(items:&[ItemStatus])->OperationStatus{
    if items.iter().any(|s|*s==ItemStatus::Running){return OperationStatus::Running}
    if items.iter().any(|s|*s==ItemStatus::Queued){return OperationStatus::Queued}
    if items.iter().all(|s|*s==ItemStatus::Succeeded){return OperationStatus::Succeeded}
    if items.iter().all(|s|*s==ItemStatus::NotExecuted){return OperationStatus::NotExecuted}
    if items.iter().all(|s|*s==ItemStatus::Unknown){return OperationStatus::Unknown}
    if items.iter().all(|s|*s==ItemStatus::AcceptedPendingConfirmation){return OperationStatus::AcceptedPendingConfirmation}
    OperationStatus::Partial
}

async fn load_operation(conn:&mut SqliteConnection,owner:&str,id:&str)->Result<Option<FeishuOperation>,FeishuConnectorError>{
    let row=sqlx::query("SELECT operation_id,request_id,connection_id,actor_json,profile,status FROM feishu_operations WHERE owner_id=? AND operation_id=?")
        .bind(owner).bind(id).fetch_optional(&mut *conn).await.map_err(FeishuConnectorError::storage)?;
    let Some(row)=row else{return Ok(None)};
    let mut attempts:BTreeMap<String,Vec<OperationAttempt>>=BTreeMap::new();
    let mut observations:BTreeMap<String,Vec<OperationObservation>>=BTreeMap::new();
    for r in sqlx::query("SELECT receipt.* FROM feishu_operation_receipts receipt JOIN feishu_operation_items item ON item.item_id=receipt.item_id WHERE item.operation_id=? ORDER BY receipt.occurred_at,receipt.id")
        .bind(id).fetch_all(&mut *conn).await.map_err(FeishuConnectorError::storage)?{
        let item_id:String=r.try_get("item_id").map_err(FeishuConnectorError::storage)?;
        match r.try_get::<String,_>("record_kind").map_err(FeishuConnectorError::storage)?.as_str(){
            "attempt"=>attempts.entry(item_id).or_default().push(OperationAttempt{
                id:r.try_get("receipt_id").map_err(FeishuConnectorError::storage)?,
                number:u32::try_from(r.try_get::<i64,_>("number").map_err(FeishuConnectorError::storage)?).map_err(FeishuConnectorError::storage)?,
                client_token:r.try_get("client_token").map_err(FeishuConnectorError::storage)?,
                mutation:parse(&r.try_get::<String,_>("mutation_json").map_err(FeishuConnectorError::storage)?)?,
                status:parse_state(&r.try_get::<String,_>("status").map_err(FeishuConnectorError::storage)?)?,
                started_at:stamp(r.try_get("occurred_at").map_err(FeishuConnectorError::storage)?)?,
                finished_at:r.try_get::<Option<i64>,_>("settled_at").map_err(FeishuConnectorError::storage)?.map(stamp).transpose()?,
                message:r.try_get("message").map_err(FeishuConnectorError::storage)?,
                result:r.try_get::<Option<String>,_>("data_json").map_err(FeishuConnectorError::storage)?.map(|v|parse(&v)).transpose()?,
            }),
            "observation"=>{
                let data:Value=parse(&r.try_get::<String,_>("data_json").map_err(FeishuConnectorError::storage)?)?;
                observations.entry(item_id).or_default().push(OperationObservation{
                    id:r.try_get("receipt_id").map_err(FeishuConnectorError::storage)?,
                    kind:serde_json::from_value(data.get("kind").cloned().unwrap_or(Value::Null)).map_err(FeishuConnectorError::storage)?,
                    created_at:stamp(r.try_get("occurred_at").map_err(FeishuConnectorError::storage)?)?,
                    message:r.try_get("message").map_err(FeishuConnectorError::storage)?,
                    data:data.get("data").filter(|v|!v.is_null()).cloned(),
                });
            }
            _=>return Err(FeishuConnectorError::storage("Unsupported connector receipt kind")),
        }
    }
    for values in attempts.values_mut(){values.sort_by_key(|attempt|attempt.number)}
    let mut items=Vec::new();
    for r in sqlx::query("SELECT * FROM feishu_operation_items WHERE operation_id=? ORDER BY ordinal")
        .bind(id).fetch_all(&mut *conn).await.map_err(FeishuConnectorError::storage)?{
        let item_id:String=r.try_get("item_id").map_err(FeishuConnectorError::storage)?;
        items.push(OperationItem{id:item_id.clone(),
            mutation:parse(&r.try_get::<String,_>("mutation_json").map_err(FeishuConnectorError::storage)?)?,
            target_key:r.try_get("target_key").map_err(FeishuConnectorError::storage)?,
            status:parse_state(&r.try_get::<String,_>("status").map_err(FeishuConnectorError::storage)?)?,
            remote_id:r.try_get("remote_id").map_err(FeishuConnectorError::storage)?,
            remote_url:r.try_get("remote_url").map_err(FeishuConnectorError::storage)?,
            message:r.try_get("message").map_err(FeishuConnectorError::storage)?,
            attempts:attempts.remove(&item_id).unwrap_or_default(),
            observations:observations.remove(&item_id).unwrap_or_default()});
    }
    if !attempts.is_empty()||!observations.is_empty(){return Err(FeishuConnectorError::storage("Feishu receipt has orphaned item facts"))}
    Ok(Some(FeishuOperation{id:row.try_get("operation_id").map_err(FeishuConnectorError::storage)?,
        request_id:row.try_get("request_id").map_err(FeishuConnectorError::storage)?,
        connection_id:row.try_get("connection_id").map_err(FeishuConnectorError::storage)?,
        actor:parse(&row.try_get::<String,_>("actor_json").map_err(FeishuConnectorError::storage)?)?,
        profile:row.try_get("profile").map_err(FeishuConnectorError::storage)?,
        status:parse_state(&row.try_get::<String,_>("status").map_err(FeishuConnectorError::storage)?)?,
        items}))
}

pub fn target_key(actor:&FeishuActor,profile:&str,item:&FeishuMutation)->Result<String,FeishuConnectorError>{
    // Known provider resources remain the same resource after an account or
    // profile switch. A new identity must not bypass an old unknown fence.
    // A creation has no remote object ID yet. Preserve its historical target-key
    // format; the separate creation-nonce check also spans actor/profile changes.
    let target=match item{
        FeishuMutation::DocumentCreate{client_item_id,..}=>
            json!(["feishu","create",actor.app_id,actor.user_id,profile,item.kind(),client_item_id]),
        FeishuMutation::DocumentDelete{document_id,..}=>json!(["feishu","document",canonical_resource_id(document_id)?]),
        FeishuMutation::BaseUpdate{base_token,table_id,record_id,..}=>json!(["feishu","base",canonical_resource_id(base_token)?,canonical_resource_id(table_id)?,canonical_resource_id(record_id)?]),
    };
    hash_json(&target)
}
fn canonical_resource_id(value:&str)->Result<String,FeishuConnectorError>{
    if value.is_empty()||value.len()>512||!value.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-".contains(&b)){
        return Err(FeishuConnectorError::invalid("Use an actual Feishu resource identity, not a URL, path, query string or whitespace alias"));
    }
    Ok(Uuid::parse_str(value).map(|id|id.to_string()).unwrap_or_else(|_|value.into()))
}
fn target_readback_confirms(mutation:&Value,data:Option<&Value>,remote_id:Option<&str>)->bool{
    let Some(data)=data else{return false};
    let id=data.get("id").or_else(||data.get("record_id")).and_then(Value::as_str);
    let Ok(mutation)=serde_json::from_value::<FeishuMutation>(mutation.clone()) else{return false};
    match &mutation{
        FeishuMutation::BaseUpdate{record_id,fields,..}=>{
            same_resource_id(id,record_id)&&same_resource_id(remote_id,record_id)
                &&data.get("fields").and_then(Value::as_object).is_some_and(|observed|
                    fields.iter().all(|(key,value)|observed.get(key)==Some(value)))
        }
        _=>false,
    }
}
fn same_resource_id(observed:Option<&str>,expected:&str)->bool{
    observed.and_then(|value|canonical_resource_id(value).ok()).zip(canonical_resource_id(expected).ok())
        .is_some_and(|(a,b)|a==b)
}
fn hash_json(value:&Value)->Result<String,FeishuConnectorError>{
    fn sorted(value:&Value)->Value{match value{
        Value::Object(map)=>{let ordered=map.iter().map(|(k,v)|(k.clone(),sorted(v))).collect::<BTreeMap<_,_>>();serde_json::to_value(ordered).expect("JSON map serializes")},
        Value::Array(values)=>Value::Array(values.iter().map(sorted).collect()),other=>other.clone(),
    }}
    let bytes=serde_json::to_vec(&sorted(value)).map_err(FeishuConnectorError::storage)?;
    Ok(format!("{:x}",Sha256::digest(bytes)))
}
fn bounded_json<T:Serialize+?Sized>(value:&T)->Result<String,FeishuConnectorError>{
    let text=serde_json::to_string(value).map_err(FeishuConnectorError::storage)?;
    if text.len()>MAX_OPERATION_BYTES{return Err(FeishuConnectorError::invalid("Feishu input or receipt exceeds its durable byte budget"))}
    Ok(text)
}
fn parse<T:DeserializeOwned>(value:&str)->Result<T,FeishuConnectorError>{serde_json::from_str(value).map_err(FeishuConnectorError::storage)}
fn parse_state<T:DeserializeOwned>(value:&str)->Result<T,FeishuConnectorError>{serde_json::from_value(Value::String(value.into())).map_err(FeishuConnectorError::storage)}
fn wire<T:Serialize>(value:&T)->Result<String,FeishuConnectorError>{serde_json::to_value(value).map_err(FeishuConnectorError::storage)?.as_str().map(str::to_owned).ok_or_else(||FeishuConnectorError::invalid("Expected named status"))}
fn now_ms()->i64{Utc::now().timestamp_millis()}
fn stamp(value:i64)->Result<String,FeishuConnectorError>{DateTime::<Utc>::from_timestamp_millis(value).map(|v|v.to_rfc3339()).ok_or_else(||FeishuConnectorError::storage("Invalid Feishu timestamp"))}
fn new_id()->String{Uuid::now_v7().to_string()}
fn require_uuid(value:&str)->Result<(),FeishuConnectorError>{
    match Uuid::parse_str(value){Ok(id)if id.get_version_num()==7&&id.to_string()==value=>Ok(()),_=>Err(FeishuConnectorError::invalid("Feishu identities must be canonical UUIDv7 values"))}
}
fn not_found(message:&str)->FeishuConnectorError{let mut error=FeishuConnectorError::invalid(message);error.code="FEISHU_NOT_FOUND".into();error}

#[cfg(test)]
#[path="store_tests.rs"]
mod tests;
