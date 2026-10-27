use std::{collections::{BTreeMap, HashMap, HashSet}, future::Future, pin::Pin, sync::{Arc, Mutex}};
use chrono::Utc;
use tokio_util::sync::CancellationToken;
use crate::{adapter, runtime::CliRuntime,
    store::OfficeHubStore, *};

pub type HubTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
pub type HubSpawner = Arc<dyn Fn(HubTask) -> bool + Send + Sync>;

pub(crate) struct AuthEntry {
    pub flow: AuthorizationFlow,
    pub cancel: CancellationToken,
    pub created: std::time::Instant,
    pub worker_active: bool,
}

pub struct OfficeHubService {
    pub(crate) owner: String,
    pub(crate) store: Arc<OfficeHubStore>,
    pub(crate) runtime: Arc<CliRuntime>,
    pub(crate) shutdown: CancellationToken,
    pub(crate) spawn: HubSpawner,
    pub(crate) flows: Arc<Mutex<HashMap<String, AuthEntry>>>,
    pub(crate) prepare_view: Arc<Mutex<Option<RuntimeInfo>>>,
    active: Mutex<HashSet<String>>,
    pub(crate) admission: tokio::sync::Mutex<()>,
    execution: tokio::sync::Mutex<()>,
    default_connection: OfficeConnection,
}

fn now() -> String { Utc::now().to_rfc3339() }
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl OfficeHubService {
    pub(crate) fn is_executing(&self) -> bool { !lock(&self.active).is_empty() }
    pub(crate) fn is_authorizing(&self) -> bool {
        lock(&self.flows).values().any(|entry| entry.worker_active || matches!(entry.flow.phase,
            AuthPhase::Starting|AuthPhase::WaitingUser|AuthPhase::WaitingAdmin|AuthPhase::Cancelling))
    }
    pub async fn new(
        owner: String, store: Arc<OfficeHubStore>, runtime: Arc<CliRuntime>,
        spawn: HubSpawner,
        shutdown: CancellationToken, recover: bool,
    ) -> Result<Arc<Self>, HubError> {
        if recover { store.recover_interrupted(&owner).await?; }
        Ok(Arc::new(Self { owner, store, runtime, shutdown, spawn,
            flows: Arc::new(Mutex::new(HashMap::new())), prepare_view: Arc::new(Mutex::new(None)),
            active: Mutex::new(HashSet::new()),
            admission: tokio::sync::Mutex::new(()), execution: tokio::sync::Mutex::new(()),
            default_connection: OfficeConnection { id: uuid::Uuid::now_v7().to_string(),
                name: "Feishu".into(), enabled: false, profile: None, executable: None,
                created_at: now(), updated_at: now() } }))
    }

    pub async fn connection(&self) -> Result<OfficeConnection, HubError> {
        Ok(self.store.connection(&self.owner).await?.unwrap_or_else(|| self.default_connection.clone()))
    }

    pub async fn update_connection(&self, input: ConnectionUpdate) -> Result<OfficeConnection, HubError> {
        let _admission=self.admission.lock().await;
        let mut value = self.connection().await?;
        let only_disabling=!input.enabled && input.profile==value.profile && input.name.trim()==value.name;
        if self.is_authorizing() || (self.is_executing() && !only_disabling) {
            return Err(HubError::conflict("Wait for the current office invocation to settle before changing its connection", None));
        }
        if let Some(profile) = &input.profile { validate_profile(profile)?; }
        value.name = input.name.trim().to_owned(); value.enabled = input.enabled;
        value.profile = input.profile; value.updated_at = now();
        self.store.save_connection(&self.owner, &value).await?;
        Ok(value)
    }

    pub async fn status(&self) -> Result<OfficeStatus, HubError> {
        let connection = self.connection().await?;
        let preparing = lock(&self.prepare_view).clone();
        let runtime = if let Some(view) = preparing.as_ref().filter(|v| v.phase == RuntimePhase::Preparing) {
            view.clone()
        } else {
            let observed=self.runtime.inspect(connection.executable.as_deref()).await;
            if observed.phase==RuntimePhase::Ready{*lock(&self.prepare_view)=None;}
            if observed.phase==RuntimePhase::Missing{
                preparing.filter(|view|view.phase==RuntimePhase::Error).unwrap_or(observed)
            }else{observed}
        };
        let mut result = OfficeStatus { provider: "feishu".into(), phase: ConnectionPhase::Disabled,
            actor: None, permissions: adapter::permissions(&[]),
            host_name: host_name(), host_os: std::env::consts::OS.into(), message: None,
            connection, runtime };
        // Visiting an unenabled workbench does not read the user's CLI account.
        if !result.connection.enabled { return Ok(result); }
        if result.runtime.phase != RuntimePhase::Ready {
            result.phase = ConnectionPhase::NotConfigured;
            result.message = result.runtime.message.clone(); return Ok(result);
        }
        let executable = result.runtime.executable.as_deref().ok_or_else(|| HubError::unavailable("CLI path is unavailable"))?;
        let Some(profile) = result.connection.profile.as_deref() else {
            result.phase = ConnectionPhase::NotConfigured; return Ok(result);
        };
        let output = self.runtime.run(executable, adapter::status_command(Some(profile), false),
            self.shutdown.child_token(), None).await?;
        match adapter::parse_identity(&output) {
            Ok(identity) => {
                result.phase = identity.phase; result.actor = identity.actor;
                result.permissions = adapter::permissions(&identity.scopes);
                if identity.message.is_some() { result.message = identity.message; }
            }
            Err(error) => {
                result.phase = if error.code=="OFFICE_AUTH_EXPIRED"{ConnectionPhase::Expired}else{ConnectionPhase::Error};
                result.message = Some(error.message);
            }
        }
        Ok(result)
    }

    /// Explicit configuration discovery works before the workbench is enabled.
    /// It reads the existing CLI's profile list, never a user login or token.
    pub async fn profiles(&self)->Result<Vec<OfficeProfile>,HubError>{
        let connection=self.connection().await?;
        let runtime=self.runtime.inspect(connection.executable.as_deref()).await;
        if runtime.phase!=RuntimePhase::Ready{
            return Err(HubError::unavailable(runtime.message.unwrap_or_else(||"Prepare or select an existing compatible CLI before listing profiles".into())))
        }
        let executable=runtime.executable.ok_or_else(||HubError::unavailable("CLI path is unavailable"))?;
        let output=self.runtime.run(&executable,adapter::profiles_command(),self.shutdown.child_token(),None).await?;
        adapter::parse_profiles(&output)
    }

    pub async fn select_runtime(&self, selection: RuntimeSelection) -> Result<RuntimeInfo, HubError> {
        let _admission=self.admission.lock().await;
        if lock(&self.prepare_view).as_ref().is_some_and(|v| v.phase == RuntimePhase::Preparing)
            || self.is_executing() || self.is_authorizing() {
            return Err(HubError::conflict("Office preparation or execution is still active", None));
        }
        let view = match selection.executable.as_deref(){
            Some(executable)=>self.runtime.select(executable).await?,
            None=>self.runtime.inspect(None).await,
        };
        let mut connection = self.connection().await?;
        connection.executable = selection.executable.and(view.executable.clone()); connection.updated_at = now();
        self.store.save_connection(&self.owner, &connection).await?;
        *lock(&self.prepare_view)=None;
        Ok(view)
    }

    pub async fn check_update(&self)->Result<RuntimeUpdateInfo,HubError>{
        let connection=self.connection().await?;
        let runtime=self.runtime.inspect(connection.executable.as_deref()).await;
        if runtime.phase!=RuntimePhase::Ready{return Err(HubError::unavailable("The Feishu CLI must be ready before checking its update source; this check never installs another runtime"))}
        let executable=runtime.executable.as_deref().ok_or_else(||HubError::unavailable("CLI path unavailable"))?;
        // This exact upstream mode skips installation, stale-file cleanup and skills sync.
        let command=crate::runtime::CliCommand::new(["update","--check","--json"],false);
        let output=self.runtime.run(executable,command,self.shutdown.child_token(),None).await?;
        if !output.successful(){return Err(adapter::output_error(&output,false))}
        let value:serde_json::Value=serde_json::from_str(output.stdout.trim())
            .map_err(|_|HubError::unavailable("The CLI update check returned an unsupported response"))?;
        if value.get("ok")!=Some(&serde_json::Value::Bool(true)){return Err(adapter::output_error(&output,false))}
        let latest=value.get("latest_version").and_then(serde_json::Value::as_str)
            .ok_or_else(||HubError::unavailable("The update source did not report a latest version"))?;
        let latest_version=semver::Version::parse(latest.trim_start_matches('v'))
            .map_err(|_|HubError::unavailable("The update source returned an invalid version"))?;
        let installed=runtime.version.as_deref().ok_or_else(||HubError::unavailable("Installed version unavailable"))?;
        let installed_version=semver::Version::parse(installed.trim_start_matches('v'))
            .map_err(|_|HubError::unavailable("Installed version is invalid"))?;
        Ok(RuntimeUpdateInfo{installed_version:Some(installed.into()),latest_version:Some(latest_version.to_string()),
            compatible_version:FEISHU_CLI_VERSION.into(),update_available:latest_version>installed_version,
            message:Some("This check did not install or modify the CLI. The verified version is the build NomiFun's managed installation pins; any official release remains usable, and externally managed installations retain their original update method.".into())})
    }

    pub async fn prepare_runtime(self: &Arc<Self>, repair: bool) -> Result<RuntimeInfo, HubError> {
        let _admission=self.admission.lock().await;
        if self.is_executing()||self.is_authorizing(){return Err(HubError::conflict("Wait for existing office work before changing CLI preparation",None))}
        let connection = self.connection().await?;
        let observed = self.runtime.inspect(connection.executable.as_deref()).await;
        if observed.phase == RuntimePhase::Ready { return Ok(observed); }
        let repairable = observed.phase == RuntimePhase::Broken;
        if observed.phase != RuntimePhase::Missing && !(repair && repairable) {
            return Err(HubError::unavailable(
                observed.message.unwrap_or_else(|| "An existing CLI must be repaired rather than replaced".into())));
        }
        {
            let mut view = lock(&self.prepare_view);
            if let Some(current) = view.as_ref().filter(|v| v.phase == RuntimePhase::Preparing) {
                return Ok(current.clone());
            }
            *view = Some(RuntimeInfo { phase: RuntimePhase::Preparing, message: Some("Preparing the official CLI npm package".into()), ..RuntimeInfo::default() });
        }
        let service = self.clone();
        let selected = connection.executable.clone();
        if !(self.spawn)(Box::pin(async move {
            let result = service.runtime.prepare(selected.as_deref(), repair, service.shutdown.child_token()).await;
            let view = match result {
                Ok(view) => {
                    if let Ok(mut connection) = service.connection().await {
                        connection.executable = view.executable.clone(); connection.updated_at = now();
                        if let Err(error) = service.store.save_connection(&service.owner, &connection).await {
                            tracing::error!(code=%error.code, "office CLI selection could not be saved");
                        }
                    }
                    view
                }
                Err(error) => RuntimeInfo { phase: RuntimePhase::Error, message: Some(error.message), ..RuntimeInfo::default() },
            };
            *lock(&service.prepare_view) = Some(view);
        })) {
            *lock(&self.prepare_view) = None;
            return Err(HubError::unavailable("The host is shutting down; CLI preparation did not start"));
        }
        Ok(lock(&self.prepare_view).clone().unwrap_or_default())
    }

    pub(crate) async fn ready(&self) -> Result<(OfficeConnection, String, adapter::CliIdentity), HubError> {
        let connection = self.connection().await?;
        if !connection.enabled { return Err(HubError::conflict("Enable the office connection before accessing Feishu", None)); }
        let profile = connection.profile.as_deref().ok_or_else(|| HubError::invalid("Select or configure a CLI profile"))?;
        let runtime = self.runtime.inspect(connection.executable.as_deref()).await;
        if runtime.phase != RuntimePhase::Ready { return Err(HubError::unavailable(runtime.message.unwrap_or_else(|| "The Feishu CLI is not ready".into()))); }
        let executable = runtime.executable.ok_or_else(|| HubError::unavailable("CLI path is unavailable"))?;
        let output = self.runtime.run(&executable, adapter::status_command(Some(profile), false), self.shutdown.child_token(), None).await?;
        let identity = adapter::parse_identity(&output)?;
        if identity.actor.is_none() || !matches!(identity.phase, ConnectionPhase::Ready) {
            return Err(HubError::unavailable("Sign in with a Feishu user identity before this operation"));
        }
        Ok((connection, executable, identity))
    }

    pub async fn query(&self, query: OfficeQuery) -> Result<OfficeQueryResult, HubError> {
        let (connection, executable, identity) = self.ready().await?;
        require_permissions(&adapter::query_permissions(&query, &identity.scopes))?;
        let commands = adapter::query_commands(&query, connection.profile.as_deref())?;
        let mut outputs = Vec::with_capacity(commands.len());
        for command in commands {
            outputs.push(self.runtime.run(&executable, command, self.shutdown.child_token(), None).await?);
        }
        let mut result = adapter::normalize_query(&query, &outputs)?;
        if let OfficeQuery::BaseRecords { base_token, table_id, .. } = &query {
            let actor=identity.actor.as_ref().ok_or_else(||HubError::unavailable("User identity unavailable"))?;
            let targets=result.items.iter().map(|item|{
                let mutation=OfficeMutation::BaseUpdate{client_item_id:item.id.clone(),base_token:base_token.clone(),
                    table_id:table_id.clone(),record_id:item.id.clone(),fields:BTreeMap::new(),before:BTreeMap::new()};
                crate::store::target_key(actor,connection.profile.as_deref().unwrap_or_default(),&mutation).map(|target|(item.id.clone(),target))
            }).collect::<Result<BTreeMap<_,_>,HubError>>()?;
            let pending=self.store.pending_targets(&self.owner,&targets.values().cloned().collect::<Vec<_>>()).await?;
            for item in &mut result.items{
                if let Some((operation,status))=targets.get(&item.id).and_then(|target|pending.get(target)){
                    item.pending_operation_id=Some(operation.clone());item.pending_status=Some(status.clone());
                }
            }
        } else if matches!(query,OfficeQuery::Documents{..}) {
            let actor=identity.actor.as_ref().ok_or_else(||HubError::unavailable("User identity unavailable"))?;
            let targets=result.items.iter().filter_map(|item|adapter::document_target(item).map(|(kind,id)|(item,kind,id)))
                .map(|(item,kind,id)|{
                    crate::store::target_key(actor,connection.profile.as_deref().unwrap_or_default(),&OfficeMutation::DocumentDelete{
                        client_item_id:item.id.clone(),document_id:id,document_type:kind,title:item.title.clone()})
                        .map(|target|(item.id.clone(),target))
                }).collect::<Result<BTreeMap<_,_>,HubError>>()?;
            let keys=targets.values().cloned().collect::<Vec<_>>();
            let mut pending=BTreeMap::new();
            for keys in keys.chunks(200) { pending.extend(self.store.pending_targets(&self.owner,keys).await?); }
            for item in &mut result.items {
                if let Some((operation,status))=targets.get(&item.id).and_then(|target|pending.get(target)) {
                    item.pending_operation_id=Some(operation.clone());item.pending_status=Some(status.clone());
                }
            }
        }
        Ok(result)
    }

    pub async fn operation(&self, id: &str) -> Result<OfficeOperation, HubError> {
        self.store.get(&self.owner, id).await?.ok_or_else(|| operation_not_found())
    }
    pub async fn operation_by_request(&self, request_id:&str)->Result<OfficeOperation,HubError>{
        self.store.get_by_request(&self.owner,request_id).await?.ok_or_else(||operation_not_found())
    }

    async fn validate_original(&self, operation: &OfficeOperation) -> Result<(String, adapter::CliIdentity), HubError> {
        let (connection, executable, identity) = self.ready().await?;
        if connection.id != operation.connection_id || connection.profile.as_deref() != Some(operation.profile.as_str())
            || !identity.actor.as_ref().is_some_and(|actor|same_actor(actor,&operation.actor)) {
            return Err(HubError::conflict("The selected Feishu application, user or profile changed; inspect the original operation", Some(operation.id.clone())));
        }
        Ok((executable, identity))
    }

    /// The host owns validation, admission and execution before the HTTP waiter
    /// can disappear. A dropped response is recovered by the same request ID.
    pub async fn execute(self:&Arc<Self>,request:ExecuteRequest)->Result<OfficeOperation,HubError>{
        let service=self.clone();
        let (reply,receiver)=tokio::sync::oneshot::channel();
        if !(self.spawn)(Box::pin(async move{
            let result=service.execute_owned(request).await;
            let _=reply.send(result);
        })){return Err(HubError::unavailable("The host is shutting down; no office work was admitted"))}
        receiver.await.map_err(|_|HubError::unknown("The execution owner did not return; recover the original result by request identity before another save"))?
    }

    async fn execute_owned(self:&Arc<Self>,request:ExecuteRequest)->Result<OfficeOperation,HubError>{
        let _admission=self.admission.lock().await;
        if let Some(existing)=self.store.get_by_request(&self.owner,&request.request_id).await?{
            if !request_matches(&existing,&request)?{
                return Err(HubError::conflict("Request identity already belongs to different submitted input",Some(existing.id)))
            }
            return Ok(existing)
        }
        if self.is_authorizing(){return Err(HubError::conflict("Finish or cancel authorization, then explicitly save the retained input again",None))}
        let Some(first)=request.items.first()else{return Err(HubError::invalid("Choose an office operation"))};
        if request.items.len()>200||request.items.iter().any(|item|item.kind()!=first.kind()){
            return Err(HubError::invalid("A request must contain one operation kind and at most 200 records"))
        }
        if let OfficeMutation::BaseUpdate{base_token,table_id,..}=first{
            if request.items.iter().any(|item|!matches!(item,OfficeMutation::BaseUpdate{base_token:b,table_id:t,..} if b==base_token&&t==table_id)){
                return Err(HubError::invalid("A Base save must target one data table"))
            }
        }else if request.items.len()!=1{return Err(HubError::invalid("Save one document change at a time"))}
        let (connection,executable,identity)=self.ready().await?;
        let actor=identity.actor.as_ref().ok_or_else(||HubError::unavailable("The user identity is unavailable"))?;
        let profile=connection.profile.as_deref().unwrap_or_default();
        if request.profile!=profile||!same_actor(actor,&request.expected_actor){
            return Err(HubError::conflict("The selected Feishu user or profile changed; refresh the connection before saving",None))
        }
        for item in &request.items{
            adapter::mutation_command(item,&uuid::Uuid::now_v7().to_string(),Some(profile))?;
            crate::store::target_key(actor,profile,item)?;
            require_permissions(&adapter::mutation_permissions(item,&identity.scopes))?;
        }
        self.preflight(&request.items,profile,actor,&executable,&identity).await?;
        let operation=loop{
            match self.store.create_submitted(&self.owner,&connection.id,actor,profile,&request).await{
                Ok(operation)=>break operation,
                Err(error)if error.code=="OFFICE_STORAGE"&&!self.shutdown.is_cancelled()=>{
                    // This owner has not dispatched. Retry only durable local
                    // admission with the same identity; never a remote command.
                    tokio::select!{_ = self.shutdown.cancelled()=>return Err(error),
                        _ = tokio::time::sleep(std::time::Duration::from_millis(250))=>{}}
                }
                Err(error)=>return Err(error),
            }
        };
        self.begin_execution(operation,executable).await
    }

    async fn begin_execution(self: &Arc<Self>, operation: OfficeOperation, executable: String) -> Result<OfficeOperation, HubError> {
        let id = operation.id.clone();
        let already_active = {
            let mut active = lock(&self.active);
            !active.insert(id.clone())
        };
        if already_active { return self.operation(&id).await; }
        let service = self.clone();
        let returned = operation.clone();
        if !(self.spawn)(Box::pin(async move {
            service.execute_items(operation, executable).await;
            lock(&service.active).remove(&id);
        })) {
            lock(&self.active).remove(&returned.id);
            self.deny_queued(&returned, "The host is shutting down; no CLI invocation started").await?;
            return self.operation(&returned.id).await;
        }
        Ok(returned)
    }

    async fn deny_queued(&self, operation: &OfficeOperation, message: &str) -> Result<(), HubError> {
        for item in operation.items.iter().filter(|i| i.status == ItemStatus::Queued) {
            let attempt = self.start_owned_attempt(operation,item).await?;
            self.store.finish_attempt(&self.owner, &operation.id, &item.id, &attempt.id, ItemStatus::NotExecuted,
                Some(message), None, None, None).await?;
        }
        Ok(())
    }

    async fn start_owned_attempt(&self,operation:&OfficeOperation,item:&OperationItem)->Result<OperationAttempt,HubError>{
        let number=item.attempts.len() as u32+1;
        loop{
            match self.store.start_attempt(&self.owner,&operation.id,&item.id).await{
                Ok(attempt)=>return Ok(attempt),
                Err(error) if error.code=="OFFICE_STORAGE"=>{
                    // This same live owner has not dispatched the CLI yet. If
                    // COMMIT succeeded but its response was lost, recover that
                    // exact reserved ordinal/input rather than create another.
                    if let Ok(Some(current))=self.store.get(&self.owner,&operation.id).await
                        && let Some(current)=current.items.iter().find(|current|current.id==item.id)
                        && let Some(attempt)=current.attempts.last()
                        && current.status==ItemStatus::Running && attempt.status==ItemStatus::Running
                        && attempt.number==number && attempt.mutation==item.mutation{
                        return Ok(attempt.clone())
                    }
                    if self.shutdown.is_cancelled(){return Err(error)}
                    tokio::select!{
                        _=self.shutdown.cancelled()=>return Err(error),
                        _=tokio::time::sleep(std::time::Duration::from_millis(250))=>{}
                    }
                }
                Err(error)=>return Err(error),
            }
        }
    }

    async fn execute_items(&self, operation: OfficeOperation, executable: String) {
        let _execution=self.execution.lock().await;
        if operation.items.iter().filter(|i|i.status==ItemStatus::Queued).all(|i|matches!(current_mutation(&i.mutation),Ok(OfficeMutation::BaseUpdate{..}))){
            self.execute_base_batch(&operation,&executable).await;
            return;
        }
        for item in operation.items.iter().filter(|i| i.status == ItemStatus::Queued) {
            let attempt = match self.start_owned_attempt(&operation,item).await {
                Ok(value) => value,
                Err(error) => {
                    let withdrawn=self.store.get(&self.owner,&operation.id).await.ok().flatten()
                        .is_some_and(|current|current.items.iter().any(|current|current.id==item.id&&current.status==ItemStatus::NotExecuted));
                    if withdrawn{continue}
                    tracing::error!(code=%error.code, "office attempt could not start"); return;
                }
            };
            let result = if self.shutdown.is_cancelled() {
                Err(HubError::unavailable("Host shutdown occurred before this CLI invocation"))
            } else {
                match current_mutation(&attempt.mutation).and_then(|mutation|adapter::mutation_command(&mutation, &attempt.client_token, Some(&operation.profile)).map(|command|(mutation,command))) {
                    Ok((mutation,command)) => match self.runtime.run(&executable, command, self.shutdown.child_token(), None).await {
                        Ok(output) => adapter::normalize_mutation(&mutation, &output),
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(error),
                }
            };
            let receipt = result.unwrap_or_else(|error| adapter::MutationReceipt {
                status: if error.not_executed { ItemStatus::NotExecuted } else { ItemStatus::Unknown },
                remote_id: None, remote_url: None, message: Some(error.message),
                result: serde_json::json!({"error_code":error.code,"not_executed":error.not_executed}),
            });
            if !self.settle_receipt(&operation.id,&item.id,&attempt.id,&receipt).await{return}
        }
    }

    async fn settle_receipt(&self,operation:&str,item:&str,attempt:&str,receipt:&adapter::MutationReceipt)->bool{
        // Retain this exact original result until durable; never rerun the CLI.
        loop{
            match self.store.finish_attempt(&self.owner,operation,item,attempt,receipt.status.clone(),
                receipt.message.as_deref(),Some(&receipt.result),receipt.remote_id.as_deref(),receipt.remote_url.as_deref()).await{
                Ok(_)=>return true,
                Err(error)if error.code=="OFFICE_STORAGE"=>{
                    tracing::warn!(code=%error.code,"retaining original office result for storage retry");
                    // Cancellation stops new remote work, not persistence of a
                    // receipt this owner already observed. Host drain owns its
                    // overall deadline and may report this owner unsettled.
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(error)=>{tracing::error!(code=%error.code,"office result remains unresolved");return false}
            }
        }
    }

    async fn execute_base_batch(&self,operation:&OfficeOperation,executable:&str){
        let mut started=Vec::new();
        for item in operation.items.iter().filter(|item|item.status==ItemStatus::Queued){
            match self.start_owned_attempt(operation,item).await{
                Ok(attempt)=>started.push((item,attempt)),
                Err(error)=>{
                    // No CLI request has been sent for this group.
                    let receipt=adapter::MutationReceipt{status:ItemStatus::NotExecuted,remote_id:None,remote_url:None,
                        message:Some("Batch admission failed before the CLI request was sent".into()),result:serde_json::json!({"error_code":error.code})};
                    for (item,attempt) in &started{self.settle_receipt(&operation.id,&item.id,&attempt.id,&receipt).await;}
                    return;
                }
            }
        }
        if started.is_empty(){return}
        let mutations=started.iter().map(|(_,attempt)|current_mutation(&attempt.mutation)).collect::<Result<Vec<_>,_>>();
        let output=if self.shutdown.is_cancelled(){Err(HubError::unavailable("Host stopped before the batch request"))}
            else{match mutations.and_then(|mutations|adapter::base_batch_command(&mutations,Some(&operation.profile))){
                Ok(command)=>self.runtime.run(executable,command,self.shutdown.child_token(),None).await,
                Err(error)=>Err(error),
            }};
        for (item,attempt) in &started{
            let receipt=match &output{
                Ok(output)=>current_mutation(&attempt.mutation).and_then(|mutation|adapter::normalize_mutation(&mutation,output)),
                Err(error)=>Err(error.clone()),
            }.unwrap_or_else(|error|adapter::MutationReceipt{status:if error.not_executed{ItemStatus::NotExecuted}else{ItemStatus::Unknown},
                remote_id:None,remote_url:None,message:Some(error.message),result:serde_json::json!({"error_code":error.code})});
            if !self.settle_receipt(&operation.id,&item.id,&attempt.id,&receipt).await{return}
        }
    }

    async fn preflight(&self,items:&[OfficeMutation],profile:&str,_actor:&OfficeActor,executable:&str,identity:&adapter::CliIdentity)->Result<(),HubError>{
        let mut checked_tables=HashMap::new();
        for mutation in items{
            match mutation{
                OfficeMutation::BaseUpdate{base_token,table_id,record_id,fields,before,..}=>{
                    let key=(base_token.clone(),table_id.clone());
                    if !checked_tables.contains_key(&key){
                        let query=OfficeQuery::BaseRecords{base_token:base_token.clone(),table_id:table_id.clone(),offset:Some(0),filter:None};
                        require_permissions(&adapter::query_permissions(&query,&identity.scopes))?;
                        let mut outputs=Vec::new();
                        for command in adapter::query_commands(&query,Some(profile))?{
                            outputs.push(self.runtime.run(executable,command,self.shutdown.child_token(),None).await?);
                        }
                        checked_tables.insert(key.clone(),adapter::normalize_query(&query,&outputs)?.fields);
                    }
                    let schema=&checked_tables[&key];
                    if fields.keys().any(|id|!schema.iter().any(|field|field.id==*id&&field.writable)){
                        return Err(HubError::invalid("A selected field is unavailable or is not supported for writing"))
                    }
                    let output=self.runtime.run(executable,adapter::base_readback_command(base_token,table_id,&[record_id.clone()],Some(profile))?,self.shutdown.child_token(),None).await?;
                    let rows=adapter::normalize_base_readback(&output)?;
                    let row=rows.items.iter().find(|row|row.id==*record_id).ok_or_else(||HubError::invalid("The exact target record could not be read before writing"))?;
                    if before.iter().any(|(key,value)|row.fields.get(key)!=Some(value)){
                        return Err(HubError::conflict("The table values changed since reading; refresh the source before saving",None))
                    }
                }
                _=>{}
            }
        }
        Ok(())
    }

    pub async fn reconcile(&self, id: &str) -> Result<OfficeOperation, HubError> {
        let operation = self.operation(id).await?;
        let (executable, identity) = self.validate_original(&operation).await?;
        for item in operation.items.iter().filter(|i| matches!(i.status, ItemStatus::Unknown | ItemStatus::AcceptedPendingConfirmation)) {
            let parsed=current_mutation(&item.mutation).ok();
            let observation = match parsed.as_ref() {
                Some(OfficeMutation::BaseUpdate { base_token, table_id, record_id, fields, .. }) => {
                    let query = OfficeQuery::BaseRecords { base_token: base_token.clone(), table_id: table_id.clone(), offset: Some(0), filter: None };
                    require_permissions(&adapter::query_permissions(&query, &identity.scopes))?;
                    let command = adapter::base_readback_command(base_token, table_id, &[record_id.clone()], Some(&operation.profile))?;
                    let output = self.runtime.run(&executable, command, self.shutdown.child_token(), None).await?;
                    let result = adapter::normalize_base_readback(&output)?;
                    let observed = result.items.iter().find(|row| row.id == *record_id);
                    let confirmed = observed.is_some_and(|row| fields.iter().all(|(key,value)| row.fields.get(key) == Some(value)));
                    (ObservationKind::TargetReadback, confirmed,
                        if confirmed { "Observed the requested field values at the original record identity" } else { "The requested values are not yet visible; this is not evidence of non-execution" },
                        serde_json::to_value(observed).map_err(HubError::storage)?, Some(record_id.as_str()), None::<String>)
                }
                _ => (ObservationKind::Inconclusive, false,
                    "No exact original terminal receipt is available; historical or unknown mutations are not replayed or inferred from text or same-name search",
                    serde_json::Value::Null, None, None),
            };
            self.store.append_observation(&self.owner, id, &item.id, observation.0, observation.2,
                Some(&observation.3), observation.1, observation.4, observation.5.as_deref()).await?;
        }
        self.operation(id).await
    }
}

fn same_actor(left:&OfficeActor,right:&OfficeActor)->bool{left.app_id==right.app_id&&left.user_id==right.user_id}
fn operation_not_found()->HubError{
    let mut error=HubError::invalid("Office operation not found");error.code="OFFICE_NOT_FOUND".into();error
}
fn current_mutation(value:&serde_json::Value)->Result<OfficeMutation,HubError>{
    serde_json::from_value(value.clone()).map_err(|_|HubError::invalid("This historical receipt is not a current executable office command"))
}
fn request_matches(operation:&OfficeOperation,request:&ExecuteRequest)->Result<bool,HubError>{
    if operation.profile!=request.profile||!same_actor(&operation.actor,&request.expected_actor)||operation.items.len()!=request.items.len(){return Ok(false)}
    for (item,input) in operation.items.iter().zip(&request.items){
        if item.mutation!=serde_json::to_value(input).map_err(HubError::storage)?{return Ok(false)}
    }
    Ok(true)
}

pub(crate) fn validate_profile(profile: &str) -> Result<(), HubError> {
    if profile.is_empty() || profile.len() > 128 || !profile.bytes().all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c)) {
        return Err(HubError::invalid("CLI profile names must contain only letters, digits, dots, underscores and hyphens"));
    }
    Ok(())
}
pub(crate) fn require_permissions(permissions: &[OfficePermission]) -> Result<(), HubError> {
    let missing: Vec<String> = permissions.iter().filter(|p| !p.granted)
        .flat_map(|p| p.required_scopes.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    if missing.is_empty() { return Ok(()); }
    let mut error = HubError::invalid("The selected operation needs additional Feishu authorization");
    error.code = "OFFICE_PERMISSION".into(); error.missing_scopes = missing; Err(error)
}
fn host_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "NomiFun host".into())
}

#[cfg(test)]
#[path="service_tests.rs"]
mod tests;
