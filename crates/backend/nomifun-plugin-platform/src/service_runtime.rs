use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    PluginArtifact, PluginId, PluginMigrationManifest, PluginServiceMode,
};
use nomifun_js_runtime::{CommittedRuntimeProvider, JavaScriptWorkKind, RuntimeUseLease};
use serde_json::Value;
use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock};
use uuid::Uuid;

use crate::{
    DataGeneration, NodePluginServiceProcess, NodePluginServiceProcessFactory,
    PluginArtifactStore, PluginMigrationRecord, PluginRuntimeContext, PluginRuntimePort,
    PluginServiceCancellation, PluginServiceError, PluginServiceFence, PluginServiceGrants,
    PluginServiceInvocation, PluginServiceLaunch, PluginServicePorts, PluginServiceProcess,
};

struct RunningService {
    process: Arc<NodePluginServiceProcess>,
    _runtime_lease: RuntimeUseLease,
}

struct ServiceSlot {
    context: PluginRuntimeContext,
    running: Option<RunningService>,
    process_generation: u64,
    preview: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginServiceObservation {
    Stopped,
    Running { generation: u64, process_id: u32 },
    Failed { generation: u64, code: String },
}

/// Process-wide Unified Plugin Service owner. One slot exists per Plugin and a
/// slot owns at most one Node process. UI-only Plugins never enter this map.
pub struct PluginServiceRuntime {
    runtime: Arc<dyn CommittedRuntimeProvider>,
    artifacts: Arc<PluginArtifactStore>,
    ports: PluginServicePorts,
    slots: Mutex<HashMap<PluginId, ServiceSlot>>,
    next_generation: AtomicU64,
    closed: AtomicBool,
    admission: Arc<RwLock<()>>,
}

impl std::fmt::Debug for PluginServiceRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginServiceRuntime")
            .finish_non_exhaustive()
    }
}

impl PluginServiceRuntime {
    pub fn new(
        runtime: Arc<dyn CommittedRuntimeProvider>,
        artifacts: Arc<PluginArtifactStore>,
        ports: PluginServicePorts,
    ) -> Self {
        Self {
            runtime,
            artifacts,
            ports,
            slots: Mutex::new(HashMap::new()),
            next_generation: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            admission: Arc::new(RwLock::new(())),
        }
    }

    pub async fn invoke(
        &self,
        plugin_id: &PluginId,
        action: &str,
        input: Value,
        call_chain: Vec<String>,
        cancellation: PluginServiceCancellation,
    ) -> Result<Value, PluginServiceError> {
        let admission = self.enter().await?;
        let mut slots = self.slots.lock().await;
        let slot = slots.get_mut(plugin_id).ok_or_else(|| {
            PluginServiceError::InvalidConfiguration("Plugin Service is not active".into())
        })?;
        if let Some(running) = &slot.running
            && running.process.terminal_result().is_some()
        {
            slot.running = None;
        }
        if slot.running.is_none() {
            slot.process_generation = self.next_process_generation()?;
            slot.running = Some(
                self.spawn(&slot.context, slot.process_generation, slot.preview)
                    .await?,
            );
        }
        let running = slot.running.as_ref().expect("running Service was created");
        let invocation = PluginServiceInvocation {
            fence: running.process.fence().clone(),
            action: action.to_owned(),
            input,
            call_chain,
        };
        let process = Arc::clone(&running.process);
        drop(slots);
        drop(admission);
        process.invoke(invocation, cancellation).await
    }

    pub async fn observation(&self, plugin_id: &PluginId) -> PluginServiceObservation {
        let mut slots = self.slots.lock().await;
        let Some(slot) = slots.get_mut(plugin_id) else {
            return PluginServiceObservation::Stopped;
        };
        let Some(running) = &slot.running else {
            return PluginServiceObservation::Stopped;
        };
        match running.process.terminal_result() {
            None => PluginServiceObservation::Running {
                generation: running.process.fence().process_generation,
                process_id: running.process.process_id(),
            },
            Some(Ok(())) => {
                slot.running = None;
                PluginServiceObservation::Stopped
            }
            Some(Err(error)) => {
                let generation = running.process.fence().process_generation;
                let code = error_code(&error).to_owned();
                slot.running = None;
                PluginServiceObservation::Failed { generation, code }
            }
        }
    }

    pub async fn shutdown(&self) -> Result<(), PluginServiceError> {
        let _admission = self.admission.write().await;
        self.closed.store(true, Ordering::Release);
        let processes = {
            let mut slots = self.slots.lock().await;
            slots
                .values_mut()
                .filter_map(|slot| slot.running.take())
                .collect::<Vec<_>>()
        };
        let mut first = None;
        for running in processes {
            if let Err(error) = running.process.stop().await
                && first.is_none()
            {
                first = Some(error);
            }
        }
        match first {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub async fn activate_preview(&self, context: PluginRuntimeContext) -> Result<(), String> {
        let _admission = self.enter().await.map_err(|error| error.to_string())?;
        if !context.artifact.manifest.has_service() {
            return Ok(());
        }
        let plugin_id = context.plugin.plugin_id.clone();
        self.quiesce(&plugin_id).await?;
        let mut slot = ServiceSlot {
            context,
            running: None,
            process_generation: 0,
            preview: true,
        };
        if slot.context.artifact.manifest.service_mode() == PluginServiceMode::Continuous {
            slot.process_generation = self.next_process_generation().map_err(|error| error.to_string())?;
            slot.running = Some(
                self.spawn(&slot.context, slot.process_generation, true)
                    .await
                    .map_err(|error| error.to_string())?,
            );
        }
        self.slots.lock().await.insert(plugin_id, slot);
        Ok(())
    }

    async fn spawn(
        &self,
        context: &PluginRuntimeContext,
        process_generation: u64,
        preview: bool,
    ) -> Result<RunningService, PluginServiceError> {
        let lease = self
            .runtime
            .acquire_use(JavaScriptWorkKind::PluginServiceHost)
            .await
            .map_err(|error| PluginServiceError::Spawn(error.to_string()))?;
        let stored = self
            .artifacts
            .load(&context.artifact.artifact_digest)
            .map_err(|error| PluginServiceError::InvalidConfiguration(error.to_string()))?;
        let module_path = stored.package_root.join("service/main.mjs");
        let factory = NodePluginServiceProcessFactory::new(lease.executable_path().to_path_buf())?
            .with_ports(self.ports.clone());
        let process = factory
            .spawn(PluginServiceLaunch {
                fence: PluginServiceFence {
                    plugin_id: context.plugin.plugin_id.clone(),
                    artifact_digest: context.artifact.artifact_digest.clone(),
                    data_generation: DataGeneration::new(context.plugin.data_generation.clone())
                        .map_err(|error| PluginServiceError::InvalidConfiguration(error.to_string()))?,
                    process_generation,
                },
                module_path,
                data_root: context.data_root.clone(),
                config: context.plugin.config.clone(),
                credential_bindings: context.credential_bindings.clone(),
                grants: grants(context),
                preview,
            })
            .await?;
        Ok(RunningService {
            process,
            _runtime_lease: lease,
        })
    }

    fn next_process_generation(&self) -> Result<u64, PluginServiceError> {
        self.next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| value.checked_add(1))
            .map(|value| value + 1)
            .map_err(|_| PluginServiceError::InvalidConfiguration("process generation overflow".into()))
    }

    async fn enter(&self) -> Result<OwnedRwLockReadGuard<()>, PluginServiceError> {
        let admission = Arc::clone(&self.admission).read_owned().await;
        if self.closed.load(Ordering::Acquire) {
            Err(PluginServiceError::InvalidConfiguration(
                "Plugin Service runtime is closed".into(),
            ))
        } else {
            Ok(admission)
        }
    }

    async fn validate_service(&self, context: &PluginRuntimeContext) -> Result<(), String> {
        if !context.artifact.manifest.has_service() {
            return Ok(());
        }
        let generation = self.next_process_generation().map_err(|error| error.to_string())?;
        let running = self
            .spawn(context, generation, true)
            .await
            .map_err(|error| error.to_string())?;
        running.process.stop().await.map_err(|error| error.to_string())
    }

    fn validate_ui(&self, artifact: &PluginArtifact) -> Result<(), String> {
        let Some(entrypoint) = artifact.manifest.entrypoints.ui.as_deref() else {
            return Ok(());
        };
        let stored = self
            .artifacts
            .load(&artifact.artifact_digest)
            .map_err(|error| error.to_string())?;
        let bytes = std::fs::read(stored.package_root.join(entrypoint))
            .map_err(|error| format!("UI entrypoint could not be read: {error}"))?;
        let html = std::str::from_utf8(&bytes)
            .map_err(|_| "UI entrypoint must be UTF-8 HTML".to_owned())?;
        if html.contains('\0') {
            return Err("UI entrypoint contains a NUL character".into());
        }
        Ok(())
    }

    async fn run_migration(
        &self,
        artifact: &PluginArtifact,
        data_root: &crate::PluginDataRootHandle,
        migration: &PluginMigrationManifest,
    ) -> Result<(), String> {
        let file = artifact
            .files
            .iter()
            .find(|file| file.normalized_relative_path == migration.path)
            .ok_or_else(|| format!("missing migration {}", migration.path))?;
        let existing = data_root.storage().migrations().map_err(|error| error.to_string())?;
        if let Some(record) = existing.iter().find(|record| record.migration_id == migration.id) {
            if record.migration_digest == file.digest
                && record.from_version == migration.from
                && record.to_version == migration.to
            {
                return Ok(());
            }
            return Err(format!("migration {} was rewritten", migration.id));
        }
        let stored = self
            .artifacts
            .load(&artifact.artifact_digest)
            .map_err(|error| error.to_string())?;
        let migration_path = stored.package_root.join(&migration.path);
        let canonical_migration = std::fs::canonicalize(&migration_path)
            .map_err(|error| error.to_string())?;
        let canonical_migration = canonical_migration
            .to_str()
            .ok_or_else(|| "migration module path must be UTF-8".to_owned())?;
        let migration_path_json =
            serde_json::to_string(canonical_migration).map_err(|error| error.to_string())?;
        let wrapper = data_root
            .path()
            .join(format!(".migration-{}.mjs", Uuid::now_v7()));
        let source = format!(
            "import {{ pathToFileURL }} from 'node:url';\n\
             const {{ migrate }} = await import(pathToFileURL({migration_path_json}).href);\n\
             export async function activate(ctx) {{\n\
               if (typeof migrate !== 'function') throw new Error('migration must export migrate(ctx)');\n\
               await migrate(ctx);\n\
               return {{ async invoke() {{ return null; }} }};\n\
             }}\n"
        );
        write_private_wrapper(&wrapper, source.as_bytes()).map_err(|error| error.to_string())?;
        let execution: Result<(), String> = async {
            let lease = self
                .runtime
                .acquire_use(JavaScriptWorkKind::PluginServiceHost)
                .await
                .map_err(|error| error.to_string())?;
            let factory =
                NodePluginServiceProcessFactory::new(lease.executable_path().to_path_buf())
                    .map_err(|error| error.to_string())?
                    .with_ports(self.ports.clone());
            let generation = self
                .next_process_generation()
                .map_err(|error| error.to_string())?;
            let process = factory
                .spawn_migration(PluginServiceLaunch {
                    fence: PluginServiceFence {
                        plugin_id: data_root.plugin_id().clone(),
                        artifact_digest: artifact.artifact_digest.clone(),
                        data_generation: data_root.generation().clone(),
                        process_generation: generation,
                    },
                    module_path: wrapper.clone(),
                    data_root: data_root.clone(),
                    config: Value::Object(Default::default()),
                    credential_bindings: Default::default(),
                    grants: PluginServiceGrants::default(),
                    preview: true,
                })
                .await
                .map_err(|error| error.to_string())?;
            process.stop().await.map_err(|error| error.to_string())
        }
        .await;
        let cleanup = std::fs::remove_file(&wrapper).map_err(|error| error.to_string());
        match (execution, cleanup) {
            (Ok(()), Ok(())) => {}
            (Err(error), Ok(())) => return Err(error),
            (Ok(()), Err(cleanup)) => {
                return Err(format!("migration wrapper cleanup failed: {cleanup}"));
            }
            (Err(error), Err(cleanup)) => {
                return Err(format!(
                    "{error}; migration wrapper cleanup also failed: {cleanup}"
                ));
            }
        }
        data_root
            .storage()
            .record_migration(&PluginMigrationRecord {
                migration_id: migration.id.clone(),
                migration_digest: file.digest.clone(),
                from_version: migration.from,
                to_version: migration.to,
                applied_at_ms: nomifun_common::now_ms().max(1),
            })
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl PluginRuntimePort for PluginServiceRuntime {
    async fn migrate(
        &self,
        artifact: &PluginArtifact,
        data_root: &crate::PluginDataRootHandle,
        migrations: &[PluginMigrationManifest],
    ) -> Result<(), String> {
        let _admission = self.enter().await.map_err(|error| error.to_string())?;
        for migration in migrations {
            self.run_migration(artifact, data_root, migration).await?;
        }
        Ok(())
    }

    async fn validate(&self, context: &PluginRuntimeContext) -> Result<(), String> {
        let _admission = self.enter().await.map_err(|error| error.to_string())?;
        self.validate_ui(&context.artifact)?;
        self.validate_service(context).await
    }

    async fn quiesce(&self, plugin_id: &PluginId) -> Result<(), String> {
        let running = self
            .slots
            .lock()
            .await
            .remove(plugin_id)
            .and_then(|slot| slot.running);
        if let Some(running) = running {
            running.process.stop().await.map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    async fn activate(&self, context: PluginRuntimeContext) -> Result<(), String> {
        let _admission = self.enter().await.map_err(|error| error.to_string())?;
        if !context.artifact.manifest.has_service() {
            self.quiesce(&context.plugin.plugin_id).await?;
            return Ok(());
        }
        let plugin_id = context.plugin.plugin_id.clone();
        let continuous = context.artifact.manifest.service_mode() == PluginServiceMode::Continuous;
        let mut slot = ServiceSlot {
            context,
            running: None,
            process_generation: 0,
            preview: false,
        };
        if continuous {
            slot.process_generation = self.next_process_generation().map_err(|error| error.to_string())?;
            slot.running = Some(
                self.spawn(&slot.context, slot.process_generation, false)
                    .await
                    .map_err(|error| error.to_string())?,
            );
        }
        self.slots.lock().await.insert(plugin_id, slot);
        Ok(())
    }

    async fn remove(&self, plugin_id: &PluginId) -> Result<(), String> {
        self.quiesce(plugin_id).await
    }
}

fn grants(context: &PluginRuntimeContext) -> PluginServiceGrants {
    PluginServiceGrants {
        secret_slots: context
            .credential_bindings
            .keys()
            .filter(|slot| context.artifact.manifest.secrets.contains(slot))
            .cloned()
            .collect(),
        host_capabilities: context.granted_permissions.clone(),
        allow_action_invoke: context.granted_permissions.contains("actions.invoke"),
    }
}

fn write_private_wrapper(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn error_code(error: &PluginServiceError) -> &'static str {
    match error {
        PluginServiceError::InvalidConfiguration(_) => "invalid_configuration",
        PluginServiceError::Spawn(_) => "spawn_failed",
        PluginServiceError::Protocol(_) => "protocol_failed",
        PluginServiceError::QueueFull => "queue_full",
        PluginServiceError::Canceled => "canceled",
        PluginServiceError::TimedOut => "timed_out",
        PluginServiceError::Rejected { .. } => "rejected",
        PluginServiceError::Crashed(_) => "crashed",
        PluginServiceError::StaleFence => "stale_fence",
    }
}
