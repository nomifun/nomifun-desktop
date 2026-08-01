use std::sync::Arc;
use std::time::Duration;

use nomifun_common::{AgentKillReason, AgentType, AppError, ConversationStatus, ErrorChain, TimestampMs};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, broadcast, watch};
use tracing::{debug, error, info};

use crate::runtime_state::AgentRuntimeState;
use crate::capability::cli_process::CliAgentProcess;
use crate::factory::construction_guard::ConstructionGuard;
use crate::manager::process_registry::{
    register_session_process, unregister_agent_process,
};
use crate::protocol::events::AgentStreamEvent;
use crate::protocol::send_error::AgentSendError;
use crate::types::SendMessageData;
use nomifun_api_types::OpenClawBuildExtra;

use super::config::load_openclaw_config;
use super::connection::{AuthConfig, OpenClawConnection};
use super::device_identity::load_or_create_identity;
use super::event_mapper::TextFallbackState;
use super::gateway_driver::{
    self, GatewayCore, GatewayState, abandon_gateway_turn, admit_gateway_turn, teardown_target_from_state,
};
use super::protocol::{ChatAbortParams, normalize_ws_url};
use super::teardown::{
    GatewayRunTurn, TeardownAttempt, TeardownCoordinator,
    request_abort_bounded, wait_for_terminal_proof,
};

mod confirmations;
mod spawn_helpers;

use spawn_helpers::{build_spawn_config, is_port_listening, wait_for_gateway_ready};

pub const DEFAULT_GATEWAY_PORT: u16 = 18789;

const OPENCLAW_KILL_GRACE_MS: u64 = 1000;
#[cfg(not(test))]
pub(super) const GATEWAY_READY_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
pub(super) const GATEWAY_READY_TIMEOUT: Duration = Duration::from_millis(200);
pub(super) const GATEWAY_READY_POLL_INTERVAL: Duration = Duration::from_millis(200);
#[cfg(not(test))]
const OPENCLAW_TEARDOWN_RPC_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const OPENCLAW_TEARDOWN_RPC_TIMEOUT: Duration = Duration::from_millis(200);
#[cfg(not(test))]
const OPENCLAW_TEARDOWN_TERMINAL_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const OPENCLAW_TEARDOWN_TERMINAL_TIMEOUT: Duration = Duration::from_millis(200);

/// Log/error label distinguishing this manager from the remote variant.
const OPENCLAW_LABEL: &str = "OpenClaw";

async fn kill_owned_gateway_process(
    connection: &OpenClawConnection,
    process: Arc<CliAgentProcess>,
) -> Result<(), AppError> {
    connection.close().await;
    process
        .kill(Duration::from_millis(OPENCLAW_KILL_GRACE_MS))
        .await
}

async fn teardown_owned_gateway_before_error(
    process_guard: &mut Option<ConstructionGuard<CliAgentProcess>>,
    data_dir: &std::path::Path,
    error: AppError,
) -> AppError {
    let Some(process_guard) = process_guard.as_mut() else {
        return error;
    };
    let data_dir = data_dir.to_path_buf();
    process_guard
        .teardown_before_error(error, move |process| {
            let data_dir = data_dir.clone();
            async move {
                let result = process
                    .kill(Duration::from_millis(OPENCLAW_KILL_GRACE_MS))
                    .await;
                if result.is_ok() {
                    let _ = unregister_agent_process(&data_dir, process.pid());
                }
                result
            }
        })
        .await
}

async fn run_openclaw_teardown(
    connection: Arc<OpenClawConnection>,
    state: Arc<RwLock<GatewayState>>,
    terminal_rx: watch::Receiver<Option<GatewayRunTurn>>,
    gateway_process: Option<Arc<CliAgentProcess>>,
) -> Result<(), AppError> {
    // A previously proven local process-tree exit remains authoritative on a
    // quarantine retry even if the first attempt had to report an abort RPC
    // error. This lets the registry release the slot on the next audit without
    // pretending the original protocol failure did not happen.
    if let Some(process) = gateway_process.as_ref()
        && process.exit_status().is_some()
    {
        connection.close().await;
        return Ok(());
    }

    let target = {
        let state = state.read().await;
        teardown_target_from_state(&state, OPENCLAW_LABEL)
    };
    let target = match target {
        Ok(target) => target,
        Err(state_error) => {
            let Some(process) = gateway_process else {
                return Err(state_error);
            };
            return match kill_owned_gateway_process(&connection, process).await {
                Ok(()) => Err(state_error),
                Err(kill_error) => Err(AppError::Internal(format!(
                    "{state_error}; local OpenClaw process teardown also failed: {kill_error}"
                ))),
            };
        }
    };

    let Some(target) = target else {
        connection.close().await;
        if let Some(process) = gateway_process {
            process
                .kill(Duration::from_millis(OPENCLAW_KILL_GRACE_MS))
                .await?;
        }
        return Ok(());
    };

    let params = serde_json::to_value(ChatAbortParams {
        session_key: target.session_key.clone(),
        run_id: target.run_id.clone(),
    })
    .map_err(|error| AppError::Internal(format!("Failed to serialize OpenClaw chat.abort: {error}")))?;
    let abort_result = request_abort_bounded(
        async {
            connection
                .request::<Value>("chat.abort", params)
                .await
                .map(|_| ())
        },
        OPENCLAW_TEARDOWN_RPC_TIMEOUT,
        "OpenClaw teardown",
    )
    .await;

    if let Err(abort_error) = abort_result {
        let Some(process) = gateway_process else {
            // Keep the transport alive: an externally managed gateway may
            // still publish a real terminal, and a quarantine retry must retain
            // the ability to issue a fresh abort.
            return Err(abort_error);
        };
        return match kill_owned_gateway_process(&connection, process).await {
            Ok(()) => Err(abort_error),
            Err(kill_error) => Err(AppError::Internal(format!(
                "{abort_error}; local OpenClaw process teardown also failed: {kill_error}"
            ))),
        };
    }

    match wait_for_terminal_proof(
        &target,
        terminal_rx,
        OPENCLAW_TEARDOWN_TERMINAL_TIMEOUT,
        "OpenClaw teardown",
    )
    .await
    {
        Ok(()) => {
            connection.close().await;
            if let Some(process) = gateway_process {
                process
                    .kill(Duration::from_millis(OPENCLAW_KILL_GRACE_MS))
                    .await?;
            }
            Ok(())
        }
        Err(terminal_error) => {
            let Some(process) = gateway_process else {
                // Closing a socket does not stop work owned by an external
                // gateway. Preserve the connection and fail closed.
                return Err(terminal_error);
            };
            // For a gateway process spawned and exclusively owned by this
            // manager, exact process-tree exit is an independent proof that no
            // local tools or write-back work can continue.
            kill_owned_gateway_process(&connection, process).await
        }
    }
}

pub struct OpenClawAgentManager {
    runtime: AgentRuntimeState,
    config: OpenClawBuildExtra,
    gateway_process: Option<Arc<CliAgentProcess>>,
    pub(super) connection: Arc<OpenClawConnection>,
    pub(super) state: Arc<RwLock<GatewayState>>,
    text_state: Mutex<TextFallbackState>,
    terminal_proof_tx: watch::Sender<Option<GatewayRunTurn>>,
    teardown: Arc<TeardownCoordinator>,
}

impl GatewayCore for OpenClawAgentManager {
    fn runtime(&self) -> &AgentRuntimeState {
        &self.runtime
    }

    fn connection(&self) -> &OpenClawConnection {
        &self.connection
    }

    fn state(&self) -> &RwLock<GatewayState> {
        &self.state
    }

    fn text_state(&self) -> &Mutex<TextFallbackState> {
        &self.text_state
    }

    fn terminal_proof_tx(&self) -> &watch::Sender<Option<GatewayRunTurn>> {
        &self.terminal_proof_tx
    }

    fn label(&self) -> &'static str {
        OPENCLAW_LABEL
    }

    fn preset_context(&self) -> Option<&str> {
        self.config.preset_context.as_deref()
    }
}

impl OpenClawAgentManager {
    pub async fn new(
        conversation_id: String,
        workspace: String,
        config: OpenClawBuildExtra,
        resume_session_key: Option<String>,
        data_dir: std::path::PathBuf,
    ) -> Result<Self, AppError> {
        let file_config = load_openclaw_config();

        let host = config.gateway.host.as_deref().unwrap_or("127.0.0.1");
        let port = config
            .gateway
            .port
            .or_else(|| {
                file_config
                    .as_ref()
                    .and_then(|c| c.gateway.as_ref())
                    .and_then(|g| g.port)
            })
            .unwrap_or(DEFAULT_GATEWAY_PORT);

        let (gateway_process, mut process_guard) = if !config.gateway.use_external_gateway {
            let cli_path = config
                .gateway
                .cli_path
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("OpenClaw CLI path is required".into()))?;

            if !is_port_listening(host, port).await {
                let spawn_config = build_spawn_config(cli_path, &workspace, &config.gateway);
                let command_preview = spawn_config.command.display().to_string();
                let process = Arc::new(CliAgentProcess::spawn(spawn_config).await?);
                let mut process_guard = Some(ConstructionGuard::new(
                    Arc::clone(&process),
                    "OpenClaw gateway CLI process",
                    CliAgentProcess::request_exact_tree_cleanup,
                ));
                if let Err(error) = register_session_process(
                    &data_dir,
                    Arc::clone(&process),
                    conversation_id.clone(),
                    AgentType::OpenclawGateway,
                    None,
                    Some(command_preview),
                ) {
                    return Err(
                        teardown_owned_gateway_before_error(
                            &mut process_guard,
                            &data_dir,
                            error,
                        )
                        .await,
                    );
                }

                if let Err(error) = wait_for_gateway_ready(host, port).await {
                    return Err(
                        teardown_owned_gateway_before_error(
                            &mut process_guard,
                            &data_dir,
                            error,
                        )
                        .await,
                    );
                }

                info!(
                    conversation_id = %conversation_id,
                    port = port,
                    "OpenClaw gateway subprocess ready"
                );

                (Some(process), process_guard)
            } else {
                debug!(port = port, "OpenClaw gateway already listening, skipping spawn");
                (None, None)
            }
        } else {
            (None, None)
        };

        let ws_url = normalize_ws_url(host, port);

        let identity = match load_or_create_identity(None) {
            Ok(identity) => identity,
            Err(error) => {
                return Err(
                    teardown_owned_gateway_before_error(
                        &mut process_guard,
                        &data_dir,
                        error,
                    )
                    .await,
                );
            }
        };

        let shared_token = config
            .gateway
            .token
            .clone()
            .or_else(|| super::config::get_gateway_auth_token(file_config.as_ref()));
        let device_token =
            super::device_auth_store::load_device_auth_token(&identity.device_id, "operator").map(|entry| entry.token);
        let password = config
            .gateway
            .password
            .clone()
            .or_else(|| super::config::get_gateway_auth_password(file_config.as_ref()));

        let auth = if shared_token.is_some() || device_token.is_some() || password.is_some() {
            Some(AuthConfig {
                token: shared_token,
                device_token,
                password,
            })
        } else {
            None
        };

        let (connection, hello) =
            match OpenClawConnection::connect(&ws_url, auth, &identity).await {
                Ok(connected) => connected,
                Err(error) => {
                    error!(
                        conversation_id = %conversation_id,
                        url = %ws_url,
                        error = %ErrorChain(&error),
                        "Failed to connect to OpenClaw gateway"
                    );
                    return Err(
                        teardown_owned_gateway_before_error(
                            &mut process_guard,
                            &data_dir,
                            error,
                        )
                        .await,
                    );
                }
            };

        if let Some(ref device_token) = hello.auth.device_token
        {
            super::device_auth_store::store_device_auth_token(
                &identity.device_id,
                &hello.auth.role,
                device_token,
                &hello.auth.scopes,
            );
        }

        info!(
            conversation_id = %conversation_id,
            url = %ws_url,
            "Connected to OpenClaw gateway via WebSocket"
        );

        let has_resume_key = resume_session_key.is_some();
        if has_resume_key {
            info!(
                conversation_id = %conversation_id,
                "Resuming OpenClaw session with stored session key"
            );
        }

        let runtime = AgentRuntimeState::new(conversation_id, workspace, 256);

        let (terminal_proof_tx, _) = watch::channel(None);
        let manager = Self {
            runtime,
            config,
            gateway_process,
            connection: Arc::clone(&connection),
            state: Arc::new(RwLock::new(GatewayState::new(resume_session_key))),
            text_state: Mutex::new(TextFallbackState::new()),
            terminal_proof_tx,
            teardown: Arc::new(TeardownCoordinator::default()),
        };

        if let Some(process_guard) = process_guard.as_mut() {
            process_guard.disarm();
        }
        Ok(manager)
    }

    pub fn start_event_relay(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.run_event_relay().await;
        });
    }

    async fn run_event_relay(self: Arc<Self>) {
        gateway_driver::relay_events(self.as_ref()).await;
        gateway_driver::mark_relay_closed(self.as_ref());
    }

    /// Clear the conversation context ("release model context"): forget the
    /// gateway session key and pending confirmations so the next send is
    /// treated as a first message — `resolve_session` then falls straight to
    /// `sessions.reset`, allocating a brand-new gateway session with no
    /// history. Robust even when the gateway is momentarily disconnected: the
    /// reset happens lazily on the next send.
    pub async fn clear_context(&self) -> Result<(), AppError> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            "Clearing OpenClaw context"
        );
        let mut state = self.state.write().await;
        state.session_key = None;
        state.has_messages = false;
        state.active_run_id = None;
        state.runtime_turn = None;
        state.pending_run_events.clear();
        state.turn_generation = state.turn_generation.wrapping_add(1);
        state.confirmations.clear();
        Ok(())
    }

    pub async fn get_diagnostics(&self) -> Value {
        let state = self.state.read().await;
        let host = self.config.gateway.host.as_deref().unwrap_or("127.0.0.1");
        let port = self.config.gateway.port.unwrap_or(DEFAULT_GATEWAY_PORT);

        json!({
            "workspace": self.runtime.workspace(),
            "backend": serde_json::to_value(&self.config.backend).unwrap_or_default(),
            "agentName": self.config.agent_name,
            "cliPath": self.config.gateway.cli_path,
            "gatewayHost": host,
            "gatewayPort": port,
            "conversationId": self.runtime.conversation_id(),
            "isConnected": self.connection.is_connected(),
            "hasActiveSession": state.session_key.is_some(),
            "sessionKey": state.session_key,
        })
    }

    fn start_teardown_attempt(
        &self,
        reason: Option<AgentKillReason>,
    ) -> Result<TeardownAttempt, AppError> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            ?reason,
            "Starting ordered OpenClaw teardown"
        );
        let connection = Arc::clone(&self.connection);
        let state = Arc::clone(&self.state);
        let terminal_rx = self.terminal_proof_tx.subscribe();
        let gateway_process = self.gateway_process.clone();
        self.teardown.start_or_join(async move {
            run_openclaw_teardown(connection, state, terminal_rx, gateway_process).await
        })
    }
}

#[cfg(test)]
mod turn_lifecycle_tests {
    use std::collections::HashMap;

    use super::super::gateway_driver::map_event_for_gateway_turn;
    use super::super::protocol::EventFrame;
    use super::*;
    use crate::runtime_state::AgentRuntimeTurn;

    fn state_for_turn(turn: AgentRuntimeTurn, run_id: Option<&str>) -> GatewayState {
        GatewayState {
            session_key: Some("session-1".into()),
            confirmations: Vec::new(),
            has_messages: run_id.is_some(),
            active_run_id: run_id.map(str::to_owned),
            turn_generation: 1,
            runtime_turn: Some(turn),
            pending_run_events: Vec::new(),
            approval_memory: HashMap::new(),
        }
    }

    #[test]
    fn first_send_failure_does_not_poison_next_send_admission() {
        let runtime = AgentRuntimeState::new("openclaw-first-send", "/workspace", 8);
        let first_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
        let mut state = state_for_turn(first_turn, None);
        state.has_messages = false;

        assert!(admit_gateway_turn(&mut state, first_turn));
        assert!(!state.has_messages, "admission alone must not claim a successful message");
        abandon_gateway_turn(&mut state, first_turn);

        let second_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
        assert!(
            admit_gateway_turn(&mut state, second_turn),
            "a failed first chat.send must retry session resolution on the next turn"
        );
    }

    #[tokio::test]
    async fn old_frame_mapping_is_linearized_before_new_turn_text_reset() {
        let runtime = AgentRuntimeState::new("openclaw-map-order", "/workspace", 8);
        let old_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
        let state = Arc::new(RwLock::new(state_for_turn(old_turn, Some("run-old"))));
        let text_state = Arc::new(Mutex::new(TextFallbackState::new()));
        let held_text = text_state.lock().await;
        let old_binding = GatewayRunTurn {
            run_id: "run-old".into(),
            turn_generation: 1,
            runtime_turn: old_turn,
        };
        let old_frame = EventFrame {
            event: "chat".into(),
            payload: Some(json!({
                "runId": "run-old",
                "state": "delta",
                "deltaText": "stale"
            })),
            seq: None,
        };

        let state_for_old = Arc::clone(&state);
        let text_for_old = Arc::clone(&text_state);
        let old_mapper = tokio::spawn(async move {
            map_event_for_gateway_turn(&state_for_old, &text_for_old, &old_frame, &old_binding).await
        });

        // Wait until the mapper has acquired the state read guard and is
        // blocked on the text mutex we hold.
        let mut mapper_holds_state = false;
        for _ in 0..100 {
            if state.try_write().is_err() {
                mapper_holds_state = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(mapper_holds_state, "old mapper never reached its linearization guard");

        let new_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
        let state_for_new = Arc::clone(&state);
        let text_for_new = Arc::clone(&text_state);
        let new_admission = tokio::spawn(async move {
            let mut state = state_for_new.write().await;
            admit_gateway_turn(&mut state, new_turn);
            let mut text = text_for_new.lock().await;
            text.reset_for_new_turn();
            text.current_run_id = Some("run-new".into());
        });

        drop(held_text);
        assert!(old_mapper.await.unwrap().is_some());
        new_admission.await.unwrap();

        let text = text_state.lock().await;
        assert_eq!(text.current_run_id.as_deref(), Some("run-new"));
        assert!(text.accumulated_text.is_empty(), "old run text leaked past the new-turn reset");
    }
}

#[cfg(all(test, unix))]
mod construction_tests {
    use std::path::PathBuf;

    use nomifun_api_types::OpenClawGatewayConfig;

    use super::*;
    use crate::manager::process_registry::agent_process_registry_path;

    fn endless_test_cli() -> PathBuf {
        ["/usr/bin/yes", "/bin/yes"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .expect("the Unix test host should provide yes(1)")
    }

    async fn unused_loopback_port() -> u16 {
        for candidate in 65_500..65_510 {
            if !is_port_listening("127.0.0.1", candidate).await {
                return candidate;
            }
        }
        panic!("test host should have an unused high loopback port");
    }

    fn test_config(port: u16) -> OpenClawBuildExtra {
        OpenClawBuildExtra {
            backend: None,
            agent_name: None,
            preset_context: None,
            gateway: OpenClawGatewayConfig {
                host: Some("127.0.0.1".into()),
                port: Some(port),
                token: None,
                password: None,
                use_external_gateway: false,
                cli_path: Some(endless_test_cli().to_string_lossy().into_owned()),
            },
            skills: Vec::new(),
            preset_id: None,
            cron_job_id: None,
            session_key: None,
        }
    }

    fn process_exists(pid: libc::pid_t) -> bool {
        if unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    fn process_group_exists(process_group_id: libc::pid_t) -> bool {
        if unsafe { libc::kill(-process_group_id, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    fn pid_from_registry_error(error: &AppError) -> libc::pid_t {
        error
            .to_string()
            .split_once("agent process ")
            .and_then(|(_, remainder)| remainder.split_once(" in runtime registry"))
            .and_then(|(pid, _)| pid.parse().ok())
            .expect("registration error should retain the exact spawned pid")
    }

    async fn wait_for_process_tree_exit(pid: libc::pid_t) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while process_exists(pid) || process_group_exists(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("construction failure returned before the spawned gateway tree exited");
    }

    async fn read_registered_pid(registry_path: &std::path::Path) -> libc::pid_t {
        tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                if let Ok(contents) = std::fs::read_to_string(registry_path)
                    && let Ok(value) =
                        serde_json::from_str::<serde_json::Value>(&contents)
                    && let Some(pid) = value["processes"][0]["pid"].as_u64()
                {
                    return libc::pid_t::try_from(pid).unwrap();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("OpenClaw process should be durably registered before readiness")
    }

    #[tokio::test]
    async fn registry_failure_keeps_openclaw_construction_pending_until_tree_exit() {
        let data_dir = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(agent_process_registry_path(data_dir.path()))
            .unwrap();

        let build = tokio::spawn(OpenClawAgentManager::new(
            "openclaw-construction-registry-failure".into(),
            workspace.path().to_string_lossy().into_owned(),
            test_config(unused_loopback_port().await),
            None,
            data_dir.path().to_path_buf(),
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !build.is_finished(),
            "OpenClaw exposed a construction error while its old gateway tree was alive"
        );

        let result = tokio::time::timeout(Duration::from_secs(10), build)
            .await
            .expect("OpenClaw exact construction cleanup timed out")
            .expect("OpenClaw construction task panicked");
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("registry destination directory must reject registration"),
        };
        assert!(error.to_string().contains("runtime registry"));
        let pid = pid_from_registry_error(&error);
        wait_for_process_tree_exit(pid).await;
    }

    #[tokio::test]
    async fn readiness_failure_waits_for_tree_exit_and_unregisters_before_error() {
        let data_dir = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let registry_path = agent_process_registry_path(data_dir.path());
        let build = tokio::spawn(OpenClawAgentManager::new(
            "openclaw-construction-readiness-failure".into(),
            workspace.path().to_string_lossy().into_owned(),
            test_config(unused_loopback_port().await),
            None,
            data_dir.path().to_path_buf(),
        ));
        let pid = read_registered_pid(&registry_path).await;

        assert!(process_exists(pid));
        assert!(
            !build.is_finished(),
            "OpenClaw exposed readiness failure before its gateway tree was cleaned"
        );

        let result = tokio::time::timeout(Duration::from_secs(5), build)
            .await
            .expect("OpenClaw readiness cleanup timed out")
            .expect("OpenClaw readiness task panicked");
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("yes(1) cannot open the configured gateway port"),
        };
        assert!(error.to_string().contains("did not become ready"));
        wait_for_process_tree_exit(pid).await;

        let registry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&registry_path).unwrap())
                .unwrap();
        assert_eq!(
            registry["processes"].as_array().map(Vec::len),
            Some(0),
            "readiness cleanup returned before unregistering the exited process"
        );
    }
}

#[cfg(test)]
mod teardown_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nomifun_api_types::OpenClawGatewayConfig;

    use super::super::device_identity::generate_identity;
    use super::super::teardown::{
        TestAbortBehavior as AbortBehavior, spawn_test_gateway,
    };

    async fn connected_test_manager(
        behavior: AbortBehavior,
        active: bool,
    ) -> (
        Arc<OpenClawAgentManager>,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        let (url, abort_count, server) = spawn_test_gateway(behavior).await;
        let (connection, _) =
            OpenClawConnection::connect(&url, None, &generate_identity())
                .await
                .unwrap();
        let runtime = AgentRuntimeState::new("openclaw-teardown-test", "/workspace", 16);
        let runtime_turn =
            active.then(|| runtime.reset_for_new_turn(ConversationStatus::Running));
        let (terminal_proof_tx, _) = watch::channel(None);
        let manager = Arc::new(OpenClawAgentManager {
            runtime,
            config: OpenClawBuildExtra {
                backend: None,
                agent_name: None,
                preset_context: None,
                gateway: OpenClawGatewayConfig::default(),
                skills: Vec::new(),
                preset_id: None,
                cron_job_id: None,
                session_key: None,
            },
            gateway_process: None,
            connection,
            state: Arc::new(RwLock::new(GatewayState {
                session_key: Some("session-1".into()),
                confirmations: Vec::new(),
                has_messages: active,
                active_run_id: active.then(|| "run-1".into()),
                turn_generation: u64::from(active),
                runtime_turn,
                pending_run_events: Vec::new(),
                approval_memory: HashMap::new(),
            })),
            text_state: Mutex::new(TextFallbackState::new()),
            terminal_proof_tx,
            teardown: Arc::new(TeardownCoordinator::default()),
        });
        if active {
            let mut text_state = manager.text_state.lock().await;
            text_state.reset_for_new_turn();
            text_state.current_run_id = Some("run-1".into());
        }
        manager.start_event_relay();
        tokio::task::yield_now().await;
        (manager, abort_count, server)
    }

    async fn finish_server(
        manager: &OpenClawAgentManager,
        server: tokio::task::JoinHandle<()>,
    ) {
        manager.connection.close().await;
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("mock gateway did not observe connection close")
            .unwrap();
    }

    #[tokio::test]
    async fn openclaw_abort_rpc_failure_is_a_teardown_error() {
        let (manager, abort_count, server) =
            connected_test_manager(AbortBehavior::Reject, true).await;

        let result = manager
            .kill_and_wait(Some(AgentKillReason::UserCancelled))
            .await;

        assert!(result.is_err());
        assert_eq!(abort_count.load(Ordering::SeqCst), 1);
        assert!(manager.connection.is_connected());
        finish_server(&manager, server).await;
    }

    #[tokio::test]
    async fn openclaw_exact_terminal_allows_external_gateway_close() {
        let (manager, abort_count, server) =
            connected_test_manager(AbortBehavior::AcknowledgeAndTerminate, true).await;

        manager
            .kill_and_wait(Some(AgentKillReason::UserCancelled))
            .await
            .unwrap();

        assert_eq!(abort_count.load(Ordering::SeqCst), 1);
        assert!(manager.state.read().await.active_run_id.is_none());
        assert!(!manager.connection.is_connected());
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("mock gateway did not observe successful close")
            .unwrap();
    }

    #[tokio::test]
    async fn idle_openclaw_teardown_closes_without_abort() {
        let (manager, abort_count, server) =
            connected_test_manager(AbortBehavior::AcknowledgeOnly, false).await;

        manager.kill_and_wait(None).await.unwrap();

        assert_eq!(abort_count.load(Ordering::SeqCst), 0);
        assert!(!manager.connection.is_connected());
        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("mock gateway did not observe idle close")
            .unwrap();
    }
}

#[async_trait::async_trait]
impl crate::runtime_handle::AgentRuntimeControl for OpenClawAgentManager {
    fn agent_type(&self) -> AgentType {
        AgentType::OpenclawGateway
    }

    fn conversation_id(&self) -> &str {
        self.runtime.conversation_id()
    }

    fn workspace(&self) -> &str {
        self.runtime.workspace()
    }

    fn status(&self) -> Option<ConversationStatus> {
        self.runtime.status()
    }

    fn is_transport_healthy(&self) -> bool {
        self.runtime.is_transport_healthy()
    }

    fn last_activity_at(&self) -> TimestampMs {
        self.runtime.last_activity_at()
    }

    fn touch_activity(&self) {
        self.runtime.bump_activity();
    }

    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.runtime.subscribe()
    }

    async fn send_message(&self, data: SendMessageData) -> Result<(), AgentSendError> {
        self.runtime.bump_activity();
        if !self.runtime.is_transport_healthy() {
            return Err(AgentSendError::stream_broken(
                "OpenClaw's permanent connection relay is no longer running",
            ));
        }

        let runtime_turn = self.runtime.reset_for_new_turn(ConversationStatus::Running);
        let is_first = {
            let mut state = self.state.write().await;
            admit_gateway_turn(&mut state, runtime_turn)
        };
        if !self.runtime.is_transport_healthy() {
            let error = AgentSendError::stream_broken(
                "OpenClaw's connection relay stopped during turn admission",
            );
            let mut state = self.state.write().await;
            abandon_gateway_turn(&mut state, runtime_turn);
            drop(state);
            self.runtime
                .emit_error_data_for_turn(runtime_turn, error.stream_error().clone());
            return Err(error);
        }

        {
            let mut text_state = self.text_state.lock().await;
            text_state.reset_for_new_turn();
        }

        match gateway_driver::send_chat_message(self, is_first, runtime_turn, data).await {
            Ok(()) => Ok(()),
            Err(err) => {
                let mut state = self.state.write().await;
                abandon_gateway_turn(&mut state, runtime_turn);
                drop(state);
                error!(
                    conversation_id = %self.runtime.conversation_id(),
                    error = %ErrorChain(&err),
                    "OpenClaw send_message failed, emitting terminal Error"
                );
                let send_error = AgentSendError::from_app_error(err);
                self.runtime
                    .emit_error_data_for_turn(runtime_turn, send_error.stream_error().clone());
                Err(send_error)
            }
        }
    }

    async fn cancel(&self) -> Result<(), AppError> {
        let target = {
            let state = self.state.read().await;
            teardown_target_from_state(&state, OPENCLAW_LABEL)
        };
        let abort_result = if let Some(target) = target? {
            let params = ChatAbortParams {
                session_key: target.session_key,
                run_id: target.run_id,
            };
            self
                .connection
                .request::<Value>("chat.abort", serde_json::to_value(params).unwrap_or_default())
                .await
                .map(|_| ())
        } else {
            Ok(())
        };

        {
            let mut state = self.state.write().await;
            state.confirmations.clear();
        }

        // The real gateway terminal event owns state clearing and Finish/Error
        // emission. A timer-generated Finish would erase the only run identity
        // teardown can use while the gateway may still be executing tools.
        abort_result
    }

    fn kill(&self, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.start_teardown_attempt(reason)?;
        Ok(())
    }
}

impl OpenClawAgentManager {
    pub fn kill_and_wait(
        &self,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send>> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            ?reason,
            "Killing OpenClaw agent and waiting for shutdown"
        );
        let attempt = self.start_teardown_attempt(reason);
        let teardown = Arc::clone(&self.teardown);
        Box::pin(async move {
            teardown
                .wait(attempt?, "OpenClaw ordered teardown failed")
                .await
        })
    }
}
