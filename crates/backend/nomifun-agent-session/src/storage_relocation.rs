//! Offline current-dataset relocation through the canonical Session writer.
use super::*;
use crate::WorkspaceRelocationOutcome;
use nomifun_agent_contracts::{AgentPatchRecoveryState, ManagedStorageRelocationV1, ManagedSessionWorkspaceNamespaceV1, ManagedAppWorkspaceNamespaceV1, NativeProcessWitness,
    is_default_managed_workspace, is_app_project_location_resource, managed_workspace_parameters, relocate_storage_path,
    resource_definition_id, storage_paths_equal, validate_storage_root};

impl AgentSessionStore {
    /// Resolve only paths witnessed at this Session's managed location before
    /// a formal relocation. This is transient read-model data, never a rewrite
    /// of historical text, payloads, resource definitions or a binding ledger.
    pub async fn managed_history_file_paths(&self, session_id: &AgentSessionId,
        managed_workspace_root: &Path, candidates: &BTreeMap<u64, BTreeSet<String>>)
        -> Result<BTreeMap<u64, BTreeMap<String, String>>, SessionStoreError> {
        if candidates.len() > MAX_EVENT_PAGE_SIZE as usize {
            return Err(conflict("too many historical workspace path sources"));
        }
        let mut tx = self.pool.begin().await?;
        let session = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if candidates.is_empty() { tx.rollback().await?; return Ok(BTreeMap::new()); }
        let Some(history) = history_workspace_locations_tx(&mut tx, &session, managed_workspace_root).await? else {
            tx.rollback().await?;
            return Ok(BTreeMap::new());
        };
        let HistoryWorkspaceLocations { current_session_root, locations, first_agent_transition,
            current_external, last_seq, app, .. } = history;
        let mut resolved = BTreeMap::new();
        for (seq, paths) in candidates {
            if *seq == 0 || *seq > last_seq { return Err(conflict("historical workspace path source is outside committed Session history")); }
            let source: Option<StoredEventRow> = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND seq=?")
                .bind(session_id.as_ref()).bind(as_i64(*seq, "historical path source")?).fetch_optional(&mut *tx).await?;
            let source = source.ok_or_else(|| conflict("historical workspace path has no committed source event"))?;
            let source = event_from_row(source)?;
            let actual_files = if source.kind.0 == "message/user-accepted" {
                payload_value_for_event_tx(&mut tx, &source).await?.get("files").and_then(Value::as_array)
                    .map(|files| files.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default()
            } else { Vec::new() };
            let mut aliases = BTreeMap::new();
            // Existing workspace selection happens at Session creation. The
            // current-generation legacy absolute-default Agent switch is the
            // only transition that can turn an opening default into external
            // authority. Its first ordinary binding event ends that managed
            // historical interval; explicit external paths stay untouched.
            let managed_interval = !current_external || first_agent_transition.is_none_or(|transition| *seq < transition);
            for path in paths {
                if path.len() > 4096 || path.chars().any(char::is_control) || !Path::new(path).is_absolute() { continue; }
                // The private display marker removes a filename timestamp. Use
                // the exact admitted file receipt, rather than searching disks
                // or guessing which similarly named attachment the user meant.
                let exact = actual_files.iter().find(|file| storage_paths_equal(file, path));
                let matching = actual_files.iter().filter(|file| storage_paths_equal(&attachment_display_path(file), path)).collect::<Vec<_>>();
                let actual = if let Some(exact) = exact { exact.as_str() }
                    else if matching.len() == 1 { matching[0].as_str() }
                    else if matching.len() > 1 { continue; }
                    else { path.as_str() };
                let mut proven_locations = Vec::new();
                if managed_interval {
                    proven_locations.extend(locations.iter().map(|(boundary, source)|
                        (*boundary, source.as_str(), current_session_root.as_path())));
                }
                if let Some(app) = &app && *seq > app.alias_after_seq {
                    proven_locations.extend(app.locations.iter().map(|(boundary, source)|
                        (*boundary, source.as_str(), app.current_root.as_path())));
                }
                for (relocation_seq, source, target) in proven_locations {
                    if *seq >= relocation_seq { continue; }
                    if let Some(moved) = relocate_storage_path(actual, source, &target.to_string_lossy()) {
                        let Ok(relative) = Path::new(&moved).strip_prefix(target) else { continue; };
                        if relative.as_os_str().is_empty() || relative.components().any(|component| match component {
                            std::path::Component::Normal(name) => name.to_str().is_none_or(|name|
                                name.trim() != name || name.contains(':') || name.contains('\\') || name.chars().any(char::is_control)),
                            _ => true,
                        }) { continue; }
                        aliases.insert(path.clone(), moved);
                        break;
                    }
                }
            }
            resolved.insert(*seq, aliases);
        }
        tx.rollback().await?;
        Ok(resolved)
    }

    /// Resolve root-only file-owner observations without probing old disks or
    /// borrowing a later selected workspace. The source is a settled file
    /// effect or native observation; managed locations use the namespace/proof reader,
    /// and external roots use that effect's immutable resource definition.
    pub async fn history_workspace_roots(&self, session_id: &AgentSessionId,
        managed_workspace_root: &Path, candidates: &BTreeMap<u64, BTreeSet<String>>)
        -> Result<BTreeMap<u64, BTreeMap<String, String>>, SessionStoreError> {
        if candidates.len() > MAX_EVENT_PAGE_SIZE as usize {
            return Err(conflict("too many historical workspace root sources"));
        }
        let mut tx = self.pool.begin().await?;
        let session = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if candidates.is_empty() { tx.rollback().await?; return Ok(BTreeMap::new()); }
        let history = history_workspace_locations_tx(&mut tx, &session, managed_workspace_root).await?;
        let last_seq = if let Some(history) = &history { history.last_seq }
            else { head_by_id_tx(&mut tx, session_id.as_ref()).await?.last_seq };
        let mut resolved = BTreeMap::new();
        for (seq, hashes) in candidates {
            if *seq == 0 || *seq > last_seq { return Err(conflict("historical workspace root source is outside committed Session history")); }
            let row: StoredEventRow = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND seq=?")
                .bind(session_id.as_ref()).bind(as_i64(*seq, "historical root source")?).fetch_optional(&mut *tx).await?
                .ok_or_else(|| conflict("historical workspace root has no committed source event"))?;
            let event = event_from_row(row)?;
            let value = payload_value_for_event_tx(&mut tx, &event).await?;
            let witnessed = file_owner_root_hashes(&event, &value)?;
            let external = if event.kind.0 == "effect/succeeded" {
                if let Some(binding_id) = value.get("resource_binding_id").and_then(Value::as_str) {
                    let row: Option<StoredResourceRow> = sqlx::query_as("SELECT binding_id,resource_kind,resource_id,owner_id,operations_json,connection_config_ref,typed_parameters_json,binding_digest FROM agent_session_resources WHERE session_id=? AND binding_id=?")
                        .bind(session_id.as_ref()).bind(binding_id).fetch_optional(&mut *tx).await?;
                    let resource = row.map(resource_from_row).transpose()?
                        .ok_or_else(|| conflict("historical file effect has no immutable resource definition"))?;
                    if resource.owner_id != session.owner_ref.principal_id {
                        return Err(conflict("historical file resource belongs to another owner"));
                    }
                    if resource.resource_kind.as_ref() == "workspace" && !is_default_managed_workspace(&resource) {
                        resource.typed_parameters.get("workspace_root").cloned()
                    } else { None }
                } else { None }
            } else { None };
            let mut roots = BTreeMap::new();
            for hash in hashes {
                if witnessed.as_ref().is_some_and(|roots| !roots.contains(hash)) { continue; }
                if let Some(history) = &history {
                    let managed_interval = !history.current_external
                        || history.first_agent_transition.is_none_or(|boundary| *seq < boundary);
                    if managed_interval && history.has_default_namespace {
                        // Pick the exact stage containing the source cursor.
                        // A future relocation's root cannot supply old authority.
                        let stage = history.locations.iter().find(|(boundary, _)| seq < boundary)
                            .map(|(_, root)| root.clone())
                            .unwrap_or_else(|| history.current_session_root.to_string_lossy().into_owned());
                        if history_root_digest_matches(&stage, hash) {
                            roots.insert(hash.clone(), history.current_session_root.to_string_lossy().into_owned());
                            continue;
                        }
                    }
                    if let Some(app) = &history.app {
                        let stage = app.locations.iter().find(|(boundary, _)| seq < boundary)
                            .map(|(_, root)| root.clone())
                            .unwrap_or_else(|| app.current_root.to_string_lossy().into_owned());
                        if history_root_digest_matches(&stage, hash) {
                            roots.insert(hash.clone(), app.current_root.to_string_lossy().into_owned());
                            continue;
                        }
                    }
                }
                if let Some(root) = &external {
                    root_string(Path::new(root))?;
                    if history_root_digest_matches(root, hash) { roots.insert(hash.clone(), root.clone()); }
                }
            }
            resolved.insert(*seq, roots);
        }
        tx.rollback().await?;
        Ok(resolved)
    }

    /// Read-only preflight. The storage coordinator still owns process teardown,
    /// root locks, the SQLite snapshot and the filesystem commit point.
    pub async fn validate_workspace_relocation(&self, source_data_root: &Path, source_work_root: &Path)
        -> Result<(), SessionStoreError> {
        self.validate_workspace_relocation_with_app_workspaces(source_data_root, source_work_root, &BTreeSet::new()).await
    }

    /// Application identities must come from the hosting owner's read-only
    /// registry validation for this exact source dataset, never a path scan.
    pub async fn validate_workspace_relocation_with_app_workspaces(&self, source_data_root: &Path,
        source_work_root: &Path, app_ids: &BTreeSet<String>) -> Result<(), SessionStoreError> {
        let data = root_string(source_data_root)?;
        let work = root_string(source_work_root)?;
        let mut tx = self.pool.begin().await?;
        let apps = managed_app_namespaces_tx(&mut tx, app_ids).await?;
        require_relocation_idle_tx(&mut tx).await?;
        require_native_storage_quiescent_tx(&mut tx, None).await?;
        for id in live_session_ids_tx(&mut tx).await? {
            require_settled_patch_history_tx(&mut tx, &id).await?;
            let session = live_session_by_id_tx(&mut tx, &id).await?;
            let namespace = opening_workspace_namespace_tx(&mut tx, &session).await?;
            validate_namespace_location_tx(&mut tx, &session, namespace.as_ref(), &data, &work).await?;
            prepare_resources_tx(&mut tx, &session, namespace.as_ref(), &apps, &data, &work, &data).await?;
        }
        tx.rollback().await?;
        Ok(())
    }

    /// Mutate only an offline staging copy of the exact current schema. Every
    /// Session transition and immutable resource definition commits together.
    /// Replays prove their identity from the existing canonical event itself.
    pub async fn relocate_managed_workspaces(&self, source_data_root: &Path, source_work_root: &Path,
        target_data_root: &Path, relocation_id: &OperationId) -> Result<WorkspaceRelocationOutcome, SessionStoreError> {
        self.relocate_managed_workspaces_with_app_workspaces(source_data_root, source_work_root,
            target_data_root, relocation_id, &BTreeSet::new()).await
    }

    /// Extend the existing location boundary to exact registered application
    /// projects. All Session resources still commit in the canonical owner.
    pub async fn relocate_managed_workspaces_with_app_workspaces(&self, source_data_root: &Path, source_work_root: &Path,
        target_data_root: &Path, relocation_id: &OperationId, app_ids: &BTreeSet<String>)
        -> Result<WorkspaceRelocationOutcome, SessionStoreError> {
        let data = root_string(source_data_root)?;
        let work = root_string(source_work_root)?;
        let target = root_string(target_data_root)?;
        validate_uuidv7(relocation_id.as_ref(), "relocation_id")?;
        let mut tx = self.begin_write_transaction().await?;
        let apps = managed_app_namespaces_tx(&mut tx, app_ids).await?;
        require_relocation_idle_tx(&mut tx).await?;
        require_native_storage_quiescent_tx(&mut tx, None).await?;
        let mut changed = 0;
        let mut managed_namespaces = Vec::new();
        let mut app_workspaces = BTreeMap::new();
        for id in live_session_ids_tx(&mut tx).await? {
            require_settled_patch_history_tx(&mut tx, &id).await?;
            let session_id = AgentSessionId::from(id.clone());
            let session = live_session_by_id_tx(&mut tx, &id).await?;
            let event_id = EventId::from(format!("storage-relocation:{}:{id}", relocation_id.as_ref()));
            if let Some(row) = event_by_event_id_tx(&mut tx, event_id.as_ref()).await? {
                let event = event_from_row(row)?;
                let payload: AgentBindingChangedPayloadV1 = serde_json::from_value(payload_value_for_event_tx(&mut tx, &event).await?)?;
                payload.validate_storage_relocation().map_err(SessionStoreError::InvalidPayload)?;
                let proof = payload.workspace_relocation.as_ref().expect("validated proof");
                validate_namespace_proof_tx(&mut tx, &session, proof).await?;
                if event.kind.0 != "session/agent-binding-changed" || event.agent_session_id != session_id
                    || event.correlation_id.as_ref() != relocation_id.as_ref()
                    || !storage_paths_equal(&proof.source_data_root, &data)
                    || !storage_paths_equal(&proof.source_work_root, &work)
                    || !storage_paths_equal(&proof.target_data_root, &target)
                    || payload.next_binding_ref != AgentHandoffBindingRefV1::from(&session.agent_binding)
                    || proof.next_resources != session.agent_binding.typed_resource_bindings
                    || proof.managed_app_workspace.as_ref().is_some_and(|app| !apps.contains(app))
                { return Err(conflict("storage relocation replay differs from its exact canonical transition")); }
                validate_namespace_location_tx(&mut tx, &session,
                    proof.managed_session_namespace.as_ref(), &target, &target).await?;
                if let Some(app) = &proof.managed_app_workspace {
                    validate_app_location_tx(&mut tx, &session, app, &target).await?;
                    app_workspaces.insert(session_id.clone(), app.clone());
                }
                if let Some(namespace) = &proof.managed_session_namespace {
                    managed_namespaces.push(namespace.clone());
                }
                continue;
            }
            let namespace = opening_workspace_namespace_tx(&mut tx, &session).await?;
            validate_namespace_location_tx(&mut tx, &session, namespace.as_ref(), &data, &work).await?;
            let (replacement_resources, knowledge_ids, app_workspace) = prepare_resources_tx(&mut tx, &session, namespace.as_ref(), &apps, &data, &work, &target).await?;
            let unchanged_resources = replacement_resources == session.agent_binding.typed_resource_bindings;
            let managed_location_changed = !storage_paths_equal(&data, &target)
                && (namespace.is_some() || (!replacement_resources.iter().any(|resource| resource.resource_kind.as_ref() == "workspace"
                    && !is_default_managed_workspace(resource))
                && replacement_resources.iter().any(|resource| is_default_managed_workspace(resource)
                    && resource.typed_parameters == managed_workspace_parameters())));
            if unchanged_resources && !managed_location_changed { continue; }
            if session.remote_binding_provenance.is_some() { return Err(conflict("Remote Session resources are immutable during relocation")); }
            let expected = session.agent_binding.clone();
            let mut replacement = expected.clone();
            replacement.binding_version = expected.binding_version.checked_add(1)
                .ok_or_else(|| conflict("Session binding version exhausted"))?;
            replacement.typed_resource_bindings = replacement_resources;
            let proof = ManagedStorageRelocationV1 {
                source_data_root: data.clone(), source_work_root: work.clone(), target_data_root: target.clone(),
                managed_knowledge_ids: knowledge_ids, previous_resources: expected.typed_resource_bindings.clone(),
                managed_session_namespace: namespace.clone(),
                managed_app_workspace: app_workspace,
                next_resources: replacement.typed_resource_bindings.clone(),
            };
            if let Some(app) = &proof.managed_app_workspace {
                app_workspaces.insert(session_id.clone(), app.clone());
            }
            let head = head_by_id_tx(&mut tx, &id).await?;
            let payload = AgentBindingChangedPayloadV1 {
                transition_id: relocation_id.clone(), request_digest: digest_payload(&proof)?,
                previous_binding_ref: AgentHandoffBindingRefV1::from(&expected),
                next_binding_ref: AgentHandoffBindingRefV1::from(&replacement),
                previous_agent_label: "Agent".into(), next_agent_label: "Agent".into(),
                handoff_mode: AgentHandoffMode::ContextOnly, handoff_payload_id: None, handoff_payload_digest: None,
                completion_gate_inherited: false, effective_after_seq: head.last_seq, workspace_relocation: Some(proof),
            };
            payload.validate_storage_relocation().map_err(SessionStoreError::InvalidPayload)?;
            let result = sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=? AND state='live' AND agent_binding_json=?")
                .bind(serde_json::to_string(&replacement)?).bind(&id).bind(serde_json::to_string(&expected)?)
                .execute(&mut *tx).await?;
            if result.rows_affected() != 1 { return Err(conflict("storage relocation lost its Session compare-and-swap boundary")); }
            if !unchanged_resources {
                replace_session_resource_definitions_tx(&mut tx, &session_id, &session.owner_ref, &replacement.typed_resource_bindings).await?;
            }
            let value = StrictJsonValue(serde_json::to_value(payload)?);
            let stored = if canonical_json_bytes(&value.0)?.len() > MAX_INLINE_JSON_BYTES {
                Some(build_payload_record(SessionPayloadId::from(format!("storage-relocation:{}:{id}:proof", relocation_id.as_ref())),
                    session_id.clone(), "application/json".into(), SessionPayloadBody::Json(value.clone()))?)
            } else { None };
            let payload_ref = match &stored {
                Some(record) => SessionEventPayloadRef::Stored(record.payload_id.clone()),
                None => SessionEventPayloadRef::InlineJson(value),
            };
            let append = SessionEventAppend {
                agent_session_id: session_id, event_id, producer_id: EventProducerId::from("session_api"),
                // Event producer idempotency is globally unique in this
                // canonical store. One dataset operation commits an exact
                // location boundary in each affected Session stream.
                idempotency_key: IdempotencyKey::from(format!("storage-relocation:{}:{id}", relocation_id.as_ref())),
                semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind: SessionEventKind("session/agent-binding-changed".into()), kind_version: 1,
                    correlation_id: CorrelationId::from(relocation_id.as_ref()), causation_event_id: None,
                    payload: payload_ref,
                },
            };
            required_ack(self.append_event_tx(&mut tx, &append, stored.as_ref()).await?)?;
            if let Some(namespace) = namespace {
                managed_namespaces.push(namespace);
            }
            changed += 1;
        }
        tx.commit().await?;
        Ok(WorkspaceRelocationOutcome { changed_sessions: changed, managed_namespaces, app_workspaces })
    }
}

struct HistoryWorkspaceLocations {
    current_session_root: std::path::PathBuf,
    locations: Vec<(u64, String)>,
    first_agent_transition: Option<u64>,
    current_external: bool,
    last_seq: u64,
    has_default_namespace: bool,
    app: Option<AppHistoryWorkspaceLocations>,
}

struct AppHistoryWorkspaceLocations {
    current_root: std::path::PathBuf,
    locations: Vec<(u64, String)>,
    /// Text-only references need a proved opening App stage. For a later App
    /// selection, the first formal location boundary starts the known stage.
    alias_after_seq: u64,
}

fn history_root_digest_matches(root: &str, hash: &str) -> bool {
    if digest_bytes(root.as_bytes()).as_ref() == hash { return true; }
    #[cfg(windows)]
    {
        // File-owner observations historically hash std::fs canonical paths;
        // frozen resource roots use the common simplified Windows form. These
        // are deterministic prefix spellings of one already-witnessed root,
        // not case guesses or filesystem discovery against an old volume.
        let native = root.replace('/', "\\");
        let alternate = if let Some(unc) = native.strip_prefix("\\\\?\\UNC\\") {
            format!("\\\\{unc}")
        } else if let Some(plain) = native.strip_prefix("\\\\?\\") {
            plain.to_owned()
        } else if let Some(unc) = native.strip_prefix("\\\\") {
            format!("\\\\?\\UNC\\{unc}")
        } else if native.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && native.as_bytes().get(1) == Some(&b':') && native.as_bytes().get(2) == Some(&b'\\') {
            format!("\\\\?\\{native}")
        } else { return false; };
        return digest_bytes(alternate.as_bytes()).as_ref() == hash;
    }
    #[cfg(not(windows))]
    { false }
}

fn file_owner_root_hashes(event: &SessionEventRecord, value: &Value) -> Result<Option<BTreeSet<String>>, SessionStoreError> {
    if event.kind_version != 1 { return Err(conflict("historical file root source version is unsupported")); }
    let mut roots = BTreeSet::new();
    if event.kind.0 == nomifun_agent_contracts::TURN_WORKSPACE_FILES_OBSERVED {
        let observation: nomifun_agent_contracts::TurnWorkspaceFilesObservedV1 = serde_json::from_value(value.clone())?;
        observation.validate().map_err(SessionStoreError::InvalidPayload)?;
        roots.insert(observation.workspace_root_sha256);
    } else if event.kind.0 == "effect/succeeded" {
        if !matches!(value.get("action_id").and_then(Value::as_str),
            Some("workspace.files/write" | "workspace.files/patch" | "workspace.files/delete" | "workspace.artifacts/publish")) {
            return Ok(Some(roots));
        }
        if let Some(result) = value.get("result") {
            roots.extend(result.pointer("/workspace_path/root_sha256").and_then(Value::as_str).map(str::to_owned));
            roots.extend(result.get("workspace_root_sha256").and_then(Value::as_str).map(str::to_owned));
            if let Some(files) = result.get("files").and_then(Value::as_array) {
                roots.extend(files.iter().filter_map(|file| file.pointer("/workspace_path/root_sha256")
                    .and_then(Value::as_str)).map(str::to_owned));
            }
        } else if value.get("observation_truncated").and_then(Value::as_bool) == Some(true) {
            let digest = value.get("observation_digest").and_then(Value::as_str)
                .ok_or_else(|| conflict("truncated file receipt has no observation digest"))?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
                return Err(conflict("truncated file receipt digest is invalid"));
            }
            // The app has already verified expanded native output against this
            // digest. This port resolves only a matching canonical workspace
            // stage/resource root; it supplies neither paths nor byte evidence.
            return Ok(None);
        }
    }
    Ok(Some(roots))
}

/// Both absolute attachment aliases and hashed file-owner observations use the
/// same validated managed namespace and ordered relocation proof chain.
async fn history_workspace_locations_tx(tx: &mut Transaction<'_, Sqlite>,
    session: &AgentSessionLiveRecord, managed_workspace_root: &Path)
    -> Result<Option<HistoryWorkspaceLocations>, SessionStoreError> {
    let session_id = &session.agent_session_id;
    let app = history_app_workspace_locations_tx(tx, session, managed_workspace_root).await?;
    let namespace = opening_workspace_namespace_tx(tx, session).await?;
    let Some(namespace) = namespace else {
        if app.is_none() { return Ok(None); }
        let head = head_by_id_tx(tx, session_id.as_ref()).await?;
        return Ok(Some(HistoryWorkspaceLocations { current_session_root: managed_workspace_root.join(session_id.as_ref()),
            locations: Vec::new(), first_agent_transition: None, current_external: true, last_seq: head.last_seq,
            has_default_namespace: false, app }));
    };
    validate_uuidv7(session_id.as_ref(), "managed Session")?;
    let current_root = root_string(managed_workspace_root)?;
    let current_session_root = managed_workspace_root.join(session_id.as_ref());
    let rows: Vec<StoredEventRow> = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed' ORDER BY seq")
        .bind(session_id.as_ref()).fetch_all(&mut **tx).await?;
    let mut locations: Vec<(u64, String)> = Vec::new();
    let mut previous_target: Option<String> = None;
    let mut first_agent_transition = None;
    for row in rows {
        let event = event_from_row(row)?;
        let value = payload_value_for_event_tx(tx, &event).await?;
        if value.get("workspace_relocation").is_none_or(Value::is_null) {
            first_agent_transition.get_or_insert(event.seq);
            continue;
        }
        let payload: AgentBindingChangedPayloadV1 = serde_json::from_value(value)?;
        payload.validate_storage_relocation().map_err(SessionStoreError::InvalidPayload)?;
        let proof = payload.workspace_relocation.as_ref().expect("validated relocation proof");
        if event.kind_version != 1 || event.correlation_id.as_ref() != payload.transition_id.as_ref()
            || payload.effective_after_seq.checked_add(1) != Some(event.seq)
            || event.event_id.as_ref() != format!("storage-relocation:{}:{}", payload.transition_id.as_ref(), session_id.as_ref())
            || proof.previous_resources.iter().chain(&proof.next_resources)
                .any(|resource| resource.owner_id != session.owner_ref.principal_id)
            || proof.managed_session_namespace.as_ref() != Some(&namespace)
        { return Err(conflict("historical workspace location has invalid canonical provenance")); }
        if let Some(previous) = &previous_target {
            if !storage_paths_equal(previous, &proof.source_data_root)
                || !storage_paths_equal(previous, &proof.source_work_root)
            { return Err(conflict("historical workspace relocation locations do not form one committed chain")); }
        } else if namespace.opening_workspace.typed_parameters.get("workspace_root")
            .is_some_and(|root| !storage_paths_equal(root, &proof.source_work_root)) {
            return Err(conflict("historical workspace relocation differs from its opening work root"));
        }
        locations.push((event.seq, Path::new(&proof.source_work_root).join("conversations")
            .join(session_id.as_ref()).to_string_lossy().into_owned()));
        previous_target = Some(proof.target_data_root.clone());
    }
    if let Some(target) = previous_target {
        let expected = Path::new(&target).join("conversations");
        if !storage_paths_equal(&current_root, &expected.to_string_lossy()) {
            return Err(conflict("historical workspace location differs from the active managed root"));
        }
    }
    let head = head_by_id_tx(tx, session_id.as_ref()).await?;
    let current_external = session.agent_binding.typed_resource_bindings.iter().any(|resource|
        resource.resource_kind.as_ref() == "workspace" && !is_default_managed_workspace(resource)
        && !resource.typed_parameters.get("workspace_root").is_some_and(|root|
            storage_paths_equal(root, &current_session_root.to_string_lossy())));

    Ok(Some(HistoryWorkspaceLocations { current_session_root, locations, first_agent_transition,
        current_external, last_seq:head.last_seq, has_default_namespace: true, app }))
}

async fn history_app_workspace_locations_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionLiveRecord,
    managed_workspace_root: &Path) -> Result<Option<AppHistoryWorkspaceLocations>, SessionStoreError> {
    let rows: Vec<StoredEventRow> = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed' ORDER BY seq DESC")
        .bind(session.agent_session_id.as_ref()).fetch_all(&mut **tx).await?;
    let mut latest = None;
    for row in rows {
        let event = event_from_row(row)?;
        let value = payload_value_for_event_tx(tx, &event).await?;
        if value.get("workspace_relocation").is_none_or(Value::is_null) { continue; }
        let payload: AgentBindingChangedPayloadV1 = serde_json::from_value(value)?;
        payload.validate_storage_relocation().map_err(SessionStoreError::InvalidPayload)?;
        if let Some(app) = payload.workspace_relocation.as_ref().and_then(|proof| proof.managed_app_workspace.clone()) {
            latest = Some((app, payload));
            break;
        }
    }
    let Some((app, latest_transition)) = latest else { return Ok(None); };
    let history = app_relocation_history_tx(tx, session, &app).await?;
    let (_, last) = history.last().ok_or_else(|| conflict("application location has no canonical transition"))?;
    validate_current_app_location_resources_tx(tx, session, &latest_transition).await?;
    let current_root = app.workspace_root(&last.target_data_root);
    if !session.agent_binding.typed_resource_bindings.iter().any(|resource| resource.resource_kind.as_ref() == "workspace"
        && resource.owner_id == app.owner_id && resource.typed_parameters.get("workspace_root")
            .is_some_and(|root| storage_paths_equal(root, &current_root))) {
        return Ok(None);
    }
    if session.agent_binding.typed_resource_bindings.iter().any(|resource| is_app_project_location_resource(resource)
        && resource.typed_parameters.get("workspace_root").is_some_and(|root| !storage_paths_equal(root, &current_root)))
    { return Err(conflict("historical application has inconsistent current project roots")); }
    let expected_managed = Path::new(&last.target_data_root).join("conversations");
    if !storage_paths_equal(&root_string(managed_workspace_root)?, &expected_managed.to_string_lossy()) {
        // App deletion preserves its ordinary Session and exact frozen root.
        // A later dataset relocation has no registered App authority to move
        // that root. It is now external to the active dataset, so retain the
        // original history without supplying aliases into a new App directory.
        return Ok(None);
    }
    let opening: StoredEventRow = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind='session/opening' ORDER BY seq LIMIT 1")
        .bind(session.agent_session_id.as_ref()).fetch_one(&mut **tx).await?;
    let opening = event_from_row(opening)?;
    let payload = payload_value_for_event_tx(tx, &opening).await?;
    let binding: AgentBindingValue = serde_json::from_value(payload.get("agent_binding").cloned()
        .ok_or_else(|| conflict("application history has no canonical opening binding"))?)?;
    let (first_seq, first) = history.first().expect("nonempty application history");
    let opening_is_app = binding.typed_resource_bindings.iter().any(|resource| resource.resource_kind.as_ref() == "workspace"
        && resource.owner_id == app.owner_id && resource.typed_parameters.get("workspace_root")
            .is_some_and(|root| storage_paths_equal(root, &app.workspace_root(&first.source_data_root))));
    let alias_after_seq = if opening_is_app { 0 } else { *first_seq };
    let locations = history.into_iter().map(|(seq, proof)| (seq, app.workspace_root(&proof.source_data_root))).collect();
    Ok(Some(AppHistoryWorkspaceLocations { current_root: Path::new(&current_root).to_path_buf(), locations, alias_after_seq }))
}

async fn validate_current_app_location_resources_tx(tx: &mut Transaction<'_, Sqlite>,
    session: &AgentSessionLiveRecord, latest: &AgentBindingChangedPayloadV1) -> Result<(), SessionStoreError> {
    let proof = latest.workspace_relocation.as_ref().expect("validated application proof");
    if latest.next_binding_ref.binding_version > session.agent_binding.binding_version
        || (latest.next_binding_ref.binding_version == session.agent_binding.binding_version
            && (latest.next_binding_ref != AgentHandoffBindingRefV1::from(&session.agent_binding)
                || proof.next_resources != session.agent_binding.typed_resource_bindings))
    { return Err(conflict("historical application proof differs from its current frozen binding")); }
    // A later ordinary Agent/model/resource choice may legitimately advance
    // the binding. Its current definitions must still be exact canonical
    // rows; neither an old proof nor an external classification forgives a
    // modified or missing current resource definition.
    for resource in &session.agent_binding.typed_resource_bindings {
        if !is_app_project_location_resource(resource) { continue; }
        let row: Option<StoredResourceRow> = sqlx::query_as("SELECT binding_id,resource_kind,resource_id,owner_id,operations_json,connection_config_ref,typed_parameters_json,binding_digest FROM agent_session_resources WHERE session_id=? AND binding_id=?")
            .bind(session.agent_session_id.as_ref()).bind(resource.binding_id.as_ref()).fetch_optional(&mut **tx).await?;
        let stored = row.map(resource_from_row).transpose()?
            .ok_or_else(|| conflict("historical application has no current immutable project resource definition"))?;
        if resource.owner_id != session.owner_ref.principal_id
            || resource_definition_id(resource)? != resource.binding_id || stored != *resource {
            return Err(conflict("historical application differs from its current immutable project resource definition"));
        }
    }
    Ok(())
}

fn attachment_display_path(raw: &str) -> String {
    let Some(index) = raw.rfind("_nomifun_") else { return raw.to_owned(); };
    let tail = &raw[index + "_nomifun_".len()..];
    let Some(timestamp) = tail.get(..13) else { return raw.to_owned(); };
    let extension = &tail[13..];
    if !timestamp.bytes().all(|byte| byte.is_ascii_digit())
        || (!extension.is_empty() && (!extension.starts_with('.') || extension.len() == 1
            || !extension[1..].bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')))
    { return raw.to_owned(); }
    format!("{}{}", &raw[..index], extension)
}

async fn opening_workspace_namespace_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionLiveRecord)
    -> Result<Option<ManagedSessionWorkspaceNamespaceV1>, SessionStoreError> {
    let row: StoredEventRow = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind='session/opening' ORDER BY seq LIMIT 1")
        .bind(session.agent_session_id.as_ref()).fetch_one(&mut **tx).await?;
    let event = event_from_row(row)?;
    let payload = payload_value_for_event_tx(tx, &event).await?;
    let binding: AgentBindingValue = serde_json::from_value(payload.get("agent_binding").cloned()
        .ok_or_else(|| conflict("managed Session namespace has no canonical opening binding"))?)?;
    if binding.typed_resource_bindings.iter().any(|resource| resource.resource_kind.as_ref() == "workspace"
        && !is_default_managed_workspace(resource)) { return Ok(None); }
    let Some(workspace) = binding.typed_resource_bindings.into_iter().find(|resource|
        resource.resource_kind.as_ref() == "workspace" && is_default_managed_workspace(resource)) else { return Ok(None); };
    if workspace.owner_id != session.owner_ref.principal_id { return Err(conflict("opening workspace belongs to another owner")); }
    let namespace = ManagedSessionWorkspaceNamespaceV1 { agent_session_id: session.agent_session_id.clone(), opening_workspace: workspace };
    namespace.validate().map_err(SessionStoreError::InvalidPayload)?;
    Ok(Some(namespace))
}

pub(super) async fn validate_namespace_proof_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionLiveRecord,
    proof: &ManagedStorageRelocationV1) -> Result<(), SessionStoreError> {
    if let Some(app) = &proof.managed_app_workspace {
        if app.owner_id != session.owner_ref.principal_id || session.owner_ref.principal_kind != "user"
            || session.remote_binding_provenance.is_some()
        { return Err(conflict("managed application proof differs from its local Session owner")); }
    }
    if let Some(namespace) = &proof.managed_session_namespace {
        if namespace.agent_session_id != session.agent_session_id
            || opening_workspace_namespace_tx(tx, session).await?.as_ref() != Some(namespace)
        { return Err(conflict("managed Session namespace differs from its immutable opening event")); }
    }
    Ok(())
}

async fn validate_namespace_location_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionLiveRecord,
    namespace: Option<&ManagedSessionWorkspaceNamespaceV1>, data: &str, work: &str) -> Result<(), SessionStoreError> {
    let Some(namespace) = namespace else { return Ok(()); };
    let rows: Vec<StoredEventRow> = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed' ORDER BY seq")
        .bind(session.agent_session_id.as_ref()).fetch_all(&mut **tx).await?;
    let mut previous_target: Option<String> = None;
    for row in rows {
        let event = event_from_row(row)?;
        let value = payload_value_for_event_tx(tx, &event).await?;
        if value.get("workspace_relocation").is_none_or(Value::is_null) { continue; }
        let payload: AgentBindingChangedPayloadV1 = serde_json::from_value(value)?;
        payload.validate_storage_relocation().map_err(SessionStoreError::InvalidPayload)?;
        let proof = payload.workspace_relocation.as_ref().expect("validated relocation proof");
        if proof.managed_session_namespace.as_ref() != Some(namespace)
            || event.correlation_id.as_ref() != payload.transition_id.as_ref()
            || payload.effective_after_seq.checked_add(1) != Some(event.seq)
            || event.event_id.as_ref() != format!("storage-relocation:{}:{}", payload.transition_id.as_ref(), session.agent_session_id.as_ref())
        { return Err(conflict("managed Session namespace differs from its canonical opening or operation")); }
        if let Some(previous) = &previous_target {
            if !storage_paths_equal(previous, &proof.source_data_root) || !storage_paths_equal(previous, &proof.source_work_root) {
                return Err(conflict("managed Session namespace positions do not form a committed relocation chain"));
            }
        } else if namespace.opening_workspace.typed_parameters.get("workspace_root")
            .is_some_and(|root| !storage_paths_equal(root, &proof.source_work_root)) {
            return Err(conflict("managed Session namespace relocation differs from its opening work root"));
        }
        previous_target = Some(proof.target_data_root.clone());
    }
    if let Some(previous) = previous_target {
        if !storage_paths_equal(&previous, data) || !storage_paths_equal(&previous, work) {
            return Err(conflict("managed Session namespace source differs from its last committed location"));
        }
        return Ok(());
    }
    if namespace.opening_workspace.typed_parameters.get("workspace_root").is_some_and(|root| !storage_paths_equal(root, work)) {
        return Err(conflict("managed Session namespace source differs from its opening work root"));
    }
    Ok(())
}

async fn app_relocation_history_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionLiveRecord,
    app: &ManagedAppWorkspaceNamespaceV1) -> Result<Vec<(u64, ManagedStorageRelocationV1)>, SessionStoreError> {
    let rows: Vec<StoredEventRow> = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed' ORDER BY seq")
        .bind(session.agent_session_id.as_ref()).fetch_all(&mut **tx).await?;
    let mut history = Vec::new();
    let mut previous_target: Option<String> = None;
    for row in rows {
        let event = event_from_row(row)?;
        let value = payload_value_for_event_tx(tx, &event).await?;
        if value.get("workspace_relocation").is_none_or(Value::is_null) { continue; }
        let payload: AgentBindingChangedPayloadV1 = serde_json::from_value(value)?;
        payload.validate_storage_relocation().map_err(SessionStoreError::InvalidPayload)?;
        let proof = payload.workspace_relocation.as_ref().expect("validated relocation proof");
        validate_namespace_proof_tx(tx, session, proof).await?;
        if event.kind_version != 1 || event.correlation_id.as_ref() != payload.transition_id.as_ref()
            || payload.effective_after_seq.checked_add(1) != Some(event.seq)
            || event.event_id.as_ref() != format!("storage-relocation:{}:{}", payload.transition_id.as_ref(), session.agent_session_id.as_ref())
        { return Err(conflict("application workspace location has invalid canonical operation provenance")); }
        if proof.managed_app_workspace.as_ref() != Some(app) { continue; }
        if previous_target.as_ref().is_some_and(|previous| !storage_paths_equal(previous, &proof.source_data_root)) {
            return Err(conflict("application workspace locations do not form a committed relocation chain"));
        }
        previous_target = Some(proof.target_data_root.clone());
        history.push((event.seq, proof.clone()));
    }
    Ok(history)
}

async fn validate_app_location_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionLiveRecord,
    app: &ManagedAppWorkspaceNamespaceV1, data: &str) -> Result<(), SessionStoreError> {
    if let Some((_, previous)) = app_relocation_history_tx(tx, session, app).await?.last()
        && !storage_paths_equal(&previous.target_data_root, data)
    { return Err(conflict("registered application source differs from its last canonical location")); }
    Ok(())
}

fn conflict(message: &str) -> SessionStoreError { SessionStoreError::Conflict(message.into()) }
fn root_string(root: &Path) -> Result<String, SessionStoreError> {
    let raw = root.to_str().ok_or_else(|| conflict("storage relocation root is not UTF-8"))?;
    validate_storage_root(raw).map_err(SessionStoreError::InvalidSession)?;
    Ok(raw.to_owned())
}

async fn live_session_ids_tx(tx: &mut Transaction<'_, Sqlite>) -> Result<Vec<String>, SessionStoreError> {
    Ok(sqlx::query_scalar("SELECT agent_session_id FROM agent_sessions WHERE state='live' ORDER BY agent_session_id")
        .fetch_all(&mut **tx).await?)
}

async fn managed_app_namespaces_tx(tx: &mut Transaction<'_, Sqlite>, app_ids: &BTreeSet<String>)
    -> Result<Vec<ManagedAppWorkspaceNamespaceV1>, SessionStoreError> {
    if app_ids.is_empty() { return Ok(Vec::new()); }
    let owner: String = sqlx::query_scalar("SELECT owner_user_id FROM installation_identity WHERE singleton_key='installation'")
        .fetch_optional(&mut **tx).await?.ok_or_else(|| conflict("managed applications have no current installation owner"))?;
    app_ids.iter().map(|app_id| {
        let namespace = ManagedAppWorkspaceNamespaceV1 { app_id: app_id.clone(), owner_id: owner.clone() };
        namespace.validate().map_err(SessionStoreError::InvalidPayload)?;
        Ok(namespace)
    }).collect()
}

async fn require_relocation_idle_tx(tx: &mut Transaction<'_, Sqlite>) -> Result<(), SessionStoreError> {
    for (query, reason) in [
        ("SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE state='deleting')", "Session deletion is unfinished"),
        ("SELECT EXISTS(SELECT 1 FROM agent_session_heads WHERE active_turn_id IS NOT NULL OR status IN ('running','paused'))", "an active or paused Turn still owns storage"),
        ("SELECT EXISTS(SELECT 1 FROM agent_turns WHERE state IN ('accepted','running') OR (native_checkpoint_json IS NOT NULL AND state='interrupted'))", "a Turn or recoverable native checkpoint still owns storage"),
        ("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE state IN ('pending','unknown'))", "pending or unknown effects must be settled before relocation"),
        ("SELECT EXISTS(SELECT 1 FROM agent_execution_attempts WHERE status IN ('queued','running','waiting_input','interrupted'))", "an AgentExecution Attempt still owns storage"),
    ] {
        if sqlx::query_scalar::<_, i64>(query).fetch_one(&mut **tx).await? != 0 { return Err(conflict(reason)); }
    }
    Ok(())
}

/// A terminal Turn or a released server lock cannot prove descendant exit.
/// Read the existing canonical owner receipts, including those of terminal
/// Turns. Missing proofs fail closed; this never probes/kills guessed PIDs or
/// obtains authority from the retired agent-process registry.
pub(super) async fn require_native_storage_quiescent_tx(tx: &mut Transaction<'_, Sqlite>, session: Option<&AgentSessionId>) -> Result<(), SessionStoreError> {
    #[derive(Default)]
    struct Scope {
        pending_operations: BTreeSet<(String, String)>,
        latest_process: Option<(String, u16)>,
    }
    let turns: BTreeSet<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT session_id,operation_id FROM agent_turns WHERE (? IS NULL OR session_id=?)")
        .bind(session.map(|id| id.as_ref())).bind(session.map(|id| id.as_ref()))
        .fetch_all(&mut **tx).await?.into_iter().collect();
    let rows: Vec<StoredEventRow> = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE kind='runtime/progress-recorded' AND (? IS NULL OR session_id=?) ORDER BY session_id,seq")
        .bind(session.map(|id| id.as_ref())).bind(session.map(|id| id.as_ref()))
        .fetch_all(&mut **tx).await?;
    let mut scopes = BTreeMap::<(String, String), Scope>::new();
    for row in rows {
        let event = event_from_row(row)?;
        let payload = payload_value_for_event_tx(tx, &event).await?;
        let native = payload.get("event").ok_or_else(|| conflict("native progress has no typed event"))?;
        let Some(kind) = native.get("event").and_then(Value::as_str) else { continue; };
        if !matches!(kind, "host_tool_dispatch" | "host_resource_dispatch" | "host_tool_settled"
            | "host_resource_settled" | "host_process_dispatch" | "host_process_quiescent" | "host_cleanup_proven") { continue; }
        let key = (event.agent_session_id.as_ref().to_owned(), event.correlation_id.as_ref().to_owned());
        if !turns.contains(&key) { return Err(conflict("native storage receipt has no exact canonical Turn")); }
        let scope = scopes.entry(key).or_default();
        match kind {
            "host_tool_dispatch" | "host_resource_dispatch" => {
                let dispatch = native.get("dispatch").unwrap_or(native);
                let operation = dispatch.get("operation_id").and_then(Value::as_str)
                    .ok_or_else(|| conflict("native storage dispatch has no owner operation identity"))?;
                let owner_kind = if kind == "host_tool_dispatch" { "tool" } else { "resource" };
                if operation.is_empty() || operation.len() > 1024 || operation.chars().any(char::is_control)
                    || !scope.pending_operations.insert((owner_kind.into(), operation.to_owned())) {
                    return Err(conflict("invalid or duplicate native storage dispatch"));
                }
            }
            "host_tool_settled" | "host_resource_settled" => {
                let operation = native.get("operation_id").and_then(Value::as_str)
                    .ok_or_else(|| conflict("native storage settlement has no owner operation identity"))?;
                let owner_kind = if kind == "host_tool_settled" { "tool" } else { "resource" };
                if !scope.pending_operations.remove(&(owner_kind.into(), operation.to_owned())) {
                    return Err(conflict("native storage settlement has no dispatch"));
                }
            }
            "host_process_dispatch" | "host_process_quiescent" => {
                let witness: NativeProcessWitness = serde_json::from_value(native.clone())?;
                witness.validate().map_err(|error| SessionStoreError::InvalidPayload(error.into()))?;
                match witness {
                    NativeProcessWitness::Dispatch { operation_id, ordinal } => {
                        scope.latest_process = Some((operation_id, ordinal));
                    }
                    NativeProcessWitness::Quiescent { operation_id, ordinal, .. } => {
                        if scope.latest_process.as_ref() != Some(&(operation_id, ordinal)) {
                            return Err(conflict("native process cleanup does not cover its exact last dispatch"));
                        }
                        scope.latest_process = None;
                    }
                }
            }
            "host_cleanup_proven" => { scope.pending_operations.clear(); scope.latest_process = None; }
            _ => unreachable!(),
        }
    }
    if scopes.values().any(|scope| !scope.pending_operations.is_empty() || scope.latest_process.is_some()) {
        return Err(conflict("native owner cleanup is unproven; complete storage/process reconciliation at the original location"));
    }
    Ok(())
}

async fn prepare_resources_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionLiveRecord,
    namespace: Option<&ManagedSessionWorkspaceNamespaceV1>,
    apps: &[ManagedAppWorkspaceNamespaceV1], data: &str, work: &str, target: &str)
    -> Result<(Vec<TypedResourceBinding>, BTreeSet<String>, Option<ManagedAppWorkspaceNamespaceV1>), SessionStoreError> {
    validate_uuidv7(session.agent_session_id.as_ref(), "managed Session")?;
    let mut resources = session.agent_binding.typed_resource_bindings.clone();
    let selected = resources.iter().any(|resource| resource.resource_kind.as_ref() == "workspace"
        && resource.resource_id.as_ref() != nomifun_agent_contracts::DEFAULT_WORKSPACE_RESOURCE_ID);
    let mut knowledge_ids = BTreeSet::new();
    let knowledge_source = Path::new(data).join("knowledge").to_string_lossy().into_owned();
    let knowledge_target = Path::new(target).join("knowledge").to_string_lossy().into_owned();
    let app_workspace = apps.iter().find(|app| resources.iter().any(|resource|
        resource.resource_kind.as_ref() == "workspace" && resource.typed_parameters.get("workspace_root")
            .is_some_and(|root| storage_paths_equal(root, &app.workspace_root(data))))).cloned();
    if let Some(app) = &app_workspace {
        if app.owner_id != session.owner_ref.principal_id || session.owner_ref.principal_kind != "user"
            || session.remote_binding_provenance.is_some()
        { return Err(conflict("registered application project belongs to another or remote Session owner")); }
        if resources.iter().any(|resource| is_app_project_location_resource(resource)
            && resource.typed_parameters.get("workspace_root").is_some_and(|root|
                !storage_paths_equal(root, &app.workspace_root(data))))
        { return Err(conflict("registered application Session has inconsistent current project roots")); }
        validate_app_location_tx(tx, session, app, data).await?;
    }
    for resource in &mut resources {
        if resource.owner_id != session.owner_ref.principal_id { return Err(conflict("Session storage resource belongs to another owner")); }
        let owned_source = namespace.map(|namespace| Path::new(work).join("conversations").join(namespace.agent_session_id.as_ref()));
        if is_app_project_location_resource(resource)
            && app_workspace.as_ref().is_some_and(|app| resource.typed_parameters.get("workspace_root")
                .is_some_and(|root| storage_paths_equal(root, &app.workspace_root(data)))) {
            resource.typed_parameters.insert("workspace_root".into(), app_workspace.as_ref().expect("matched application").workspace_root(target));
            resource.binding_id = resource_definition_id(resource)?;
        } else if matches!(resource.resource_kind.as_ref(), "workspace" | "process_session")
            && owned_source.as_ref().is_some_and(|source| resource.typed_parameters.get("workspace_root")
                .is_some_and(|root| storage_paths_equal(root, &source.to_string_lossy()))) {
            resource.typed_parameters.insert("workspace_root".into(), Path::new(target).join("conversations")
                .join(session.agent_session_id.as_ref()).to_string_lossy().into_owned());
            resource.binding_id = resource_definition_id(resource)?;
        } else if !selected && is_default_managed_workspace(resource) {
            if resource.typed_parameters == managed_workspace_parameters() { continue; }
            if resource.typed_parameters.len() != 1 || !resource.typed_parameters.get("workspace_root")
                .is_some_and(|root| storage_paths_equal(root, work))
            { return Err(conflict("managed Session Workspace does not have the exact source work root")); }
            // The default resource and exact Session UUID select this isolated
            // child. No prefix-based guess can adopt a user-selected project.
            let _concrete = Path::new(work).join("conversations").join(session.agent_session_id.as_ref());
            resource.typed_parameters = managed_workspace_parameters();
            resource.binding_id = resource_definition_id(resource)?;
        } else if resource.resource_kind.as_ref() == "knowledge_base" {
            let info: Option<(String, i64)> = sqlx::query_as("SELECT root_path,managed FROM knowledge_bases WHERE knowledge_base_id=?")
                .bind(resource.resource_id.as_ref()).fetch_optional(&mut **tx).await?;
            let Some((registered_root, managed)) = info else { return Err(conflict("frozen Knowledge resource has no current registered base")); };
            if managed == 0 { continue; }
            let root = resource.typed_parameters.get("knowledge_root").ok_or_else(|| conflict("Knowledge resource has no frozen root"))?;
            if !storage_paths_equal(root, &registered_root) { return Err(conflict("managed Knowledge resource differs from its registered root")); }
            let exact_source = Path::new(&knowledge_source).join(resource.resource_id.as_ref()).to_string_lossy().into_owned();
            if !storage_paths_equal(root, &exact_source) { return Err(conflict("managed Knowledge resource has no exact product-owned base root")); }
            let moved = relocate_storage_path(root, &knowledge_source, &knowledge_target)
                .ok_or_else(|| conflict("managed Knowledge resource is outside the owned knowledge domain"))?;
            knowledge_ids.insert(resource.resource_id.as_ref().to_owned());
            resource.typed_parameters.insert("knowledge_root".into(), moved);
            resource.binding_id = resource_definition_id(resource)?;
        }
    }
    Ok((resources, knowledge_ids, app_workspace))
}

async fn require_settled_patch_history_tx(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<(), SessionStoreError> {
    let rows: Vec<StoredEventRow> = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind IN ('runtime/progress-recorded','session/agent-binding-changed') ORDER BY seq")
        .bind(id).fetch_all(&mut **tx).await?;
    let mut recovery: Option<(u64, AgentPatchRecoveryState)> = None;
    let mut dispatched = 0;
    for row in rows {
        let event = event_from_row(row)?;
        if event.kind.0 == "session/agent-binding-changed" {
            if recovery.as_ref().is_some_and(|(_, state)| state.has_pending())
                || dispatched > recovery.as_ref().map_or(0, |(seq, _)| *seq)
            { return Err(conflict("a binding transition crosses pending Patch recovery")); }
            recovery = None; dispatched = 0; continue;
        }
        let payload = payload_value_for_event_tx(tx, &event).await?;
        let native = payload.get("event").ok_or_else(|| conflict("native progress has no typed event"))?;
        match native.get("event").and_then(Value::as_str) {
            Some("host_tool_dispatch") if native.pointer("/dispatch/action_id").and_then(Value::as_str) == Some("workspace.files/patch") => dispatched = event.seq,
            Some("patch_recovery_updated") => {
                let state: AgentPatchRecoveryState = serde_json::from_value(native.get("state").cloned()
                    .ok_or_else(|| conflict("Patch recovery has no typed state"))?)?;
                state.validate().map_err(SessionStoreError::InvalidPayload)?;
                recovery = Some((event.seq, state));
            }
            _ => {}
        }
    }
    if recovery.as_ref().is_some_and(|(_, state)| state.has_pending())
        || dispatched > recovery.as_ref().map_or(0, |(seq, _)| *seq)
    { return Err(conflict("resolve pending or uncovered Patch recovery before relocation")); }
    Ok(())
}

#[cfg(all(test, windows))]
mod root_digest_tests {
    use super::*;

    #[test]
    fn history_file_root_hash_tolerates_only_known_windows_prefix_spellings() {
        for (plain, native) in [(r"C:\Work\Session", r"\\?\C:\Work\Session"),
            (r"\\server\share\Session", r"\\?\UNC\server\share\Session")] {
            assert!(history_root_digest_matches(plain, digest_bytes(native.as_bytes()).as_ref()));
            assert!(history_root_digest_matches(native, digest_bytes(plain.as_bytes()).as_ref()));
            assert!(!history_root_digest_matches(plain, digest_bytes(plain.to_lowercase().as_bytes()).as_ref()));
        }
    }
}
