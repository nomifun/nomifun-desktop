//! Prepare a fresh private Session, then stop the setup backend. Launch the
//! formal Dev app with the printed data_dir and send the printed prompt in its
//! normal UI. This fixture never invokes a Computer tool itself.
//!
//! Arguments: NEW_DATA_DIR AUDIT_APP_NAME PRIVATE_MARKER_JSON BUNDLE_ID APP_PATH
//! The independently prepared audit App must write exactly the public marker
//! fields {pid,bundle_identifier,bundle_path,stage:"didFinishLaunching"}.
//! Three model responses only: cursor_position, bare launch, ordinary text/stop.
//! GET /report checks canonical tool/effect/Turn facts through a read-only Store.
//! No CompletionReported is expected for this Computer-only, non-ledger Turn.

#[cfg(not(all(target_os = "macos", feature = "computer-use")))]
fn main() {
    eprintln!("This fixture requires macOS and the computer-use feature.");
    std::process::exit(1);
}

#[cfg(all(target_os = "macos", feature = "computer-use"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fixture::run().await
}

#[cfg(all(target_os = "macos", feature = "computer-use"))]
mod fixture {
    use axum::{
        extract::State,
        routing::{get, post},
        Json, Router,
    };
    use nomifun_agent_contracts::{AgentSessionId, SessionEventPayloadRef, StrictJsonValue};
    use nomifun_agent_runtime::{AgentEngineEvent, AgentToolResult};
    use nomifun_agent_session::{
        AgentEffectState, AgentSessionStore, EffectStrategy, TurnReceiptStatus,
    };
    use nomifun_app::{DesktopHostServices, DesktopServer};
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::{
        fs,
        io::Read,
        os::unix::fs::{DirBuilderExt, MetadataExt},
        path::{Component, Path, PathBuf},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };
    use tokio_util::sync::CancellationToken;

    const CURSOR: &str = "macos-canonical-cursor-once";
    const LAUNCH: &str = "macos-canonical-launch-once";
    const MODEL: &str = "macos-computer-fixture";
    const PREFIX: &str = "NomiFun Launch Audit ";

    struct Config {
        data_dir: PathBuf,
        marker: PathBuf,
        app_name: String,
        bundle_id: String,
        app_path: PathBuf,
    }

    #[derive(Clone, Deserialize, serde::Serialize, PartialEq, Eq)]
    #[serde(deny_unknown_fields)]
    struct Marker {
        pid: u32,
        bundle_identifier: String,
        bundle_path: PathBuf,
        stage: String,
    }

    #[derive(Clone, Default)]
    struct Evidence {
        cursor: Option<Value>,
        launch: Option<Value>,
        marker: Option<Marker>,
        observe_name: Option<String>,
        launch_name: Option<String>,
        final_response_emitted: bool,
    }

    struct Fixture {
        config: Config,
        calls: AtomicUsize,
        evidence: Mutex<Evidence>,
        failure: Mutex<Option<String>>,
        session_id: std::sync::OnceLock<AgentSessionId>,
        stop: CancellationToken,
    }

    fn absent(path: &Path) -> anyhow::Result<()> {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
            Ok(_) => anyhow::bail!("fixture path already exists: {}", path.display()),
        }
    }

    fn absolute_normal(path: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            path.is_absolute()
                && path
                    .components()
                    .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
            "explicit absolute path required"
        );
        Ok(())
    }

    fn config() -> anyhow::Result<Config> {
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        anyhow::ensure!(
            args.len() == 5,
            "expected NEW_DATA_DIR AUDIT_APP_NAME PRIVATE_MARKER_JSON BUNDLE_ID APP_PATH"
        );
        let app_name = args[1]
            .clone()
            .into_string()
            .map_err(|_| anyhow::anyhow!("audit App name must be UTF-8"))?;
        let suffix = app_name
            .strip_prefix(PREFIX)
            .ok_or_else(|| anyhow::anyhow!("only owned launch audit App names are allowed"))?;
        anyhow::ensure!(
            !suffix.is_empty()
                && suffix.len() <= 64
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
            "audit App suffix must be 1–64 ASCII letters, digits or hyphens"
        );
        let bundle_id = args[3]
            .clone()
            .into_string()
            .map_err(|_| anyhow::anyhow!("bundle ID must be UTF-8"))?;
        anyhow::ensure!(
            bundle_id == format!("com.nomifun.desktop.audit.launch.{suffix}"),
            "audit bundle ID must match the unique App suffix"
        );
        let data_dir = PathBuf::from(&args[0]);
        let marker = PathBuf::from(&args[2]);
        let app_path = PathBuf::from(&args[4]);
        for path in [&data_dir, &marker, &app_path] {
            absolute_normal(path)?;
        }
        let parent = fs::canonicalize(
            data_dir
                .parent()
                .ok_or_else(|| anyhow::anyhow!("private fixture parent required"))?,
        )?;
        let parent_meta = fs::metadata(&parent)?;
        anyhow::ensure!(
            parent_meta.is_dir() && parent_meta.mode() & 0o077 == 0,
            "fixture parent must already be a private directory"
        );
        anyhow::ensure!(fs::canonicalize(marker.parent().ok_or_else(|| anyhow::anyhow!("marker parent required"))?)? == parent,
            "marker must be a direct sibling of the fresh data directory in the private fixture parent");
        let data_dir = parent.join(
            data_dir
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("data directory leaf required"))?,
        );
        let marker = parent.join(
            marker
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("marker filename required"))?,
        );
        absent(&data_dir)?;
        absent(&marker)?;
        let app_meta = fs::symlink_metadata(&app_path)?;
        anyhow::ensure!(
            app_meta.is_dir()
                && !app_meta.file_type().is_symlink()
                && app_meta.uid() == parent_meta.uid()
                && app_path.file_name().and_then(|name| name.to_str())
                    == Some(format!("{app_name}.app").as_str()),
            "only the explicitly prepared owned audit .app is accepted"
        );
        let app_path = fs::canonicalize(app_path)?;
        fs::DirBuilder::new().mode(0o700).create(&data_dir)?;
        anyhow::ensure!(
            fs::metadata(&data_dir)?.uid() == parent_meta.uid(),
            "private fixture parent must belong to this process user"
        );
        Ok(Config {
            data_dir,
            marker,
            app_name,
            bundle_id,
            app_path,
        })
    }

    fn fail(f: &Fixture, error: impl std::fmt::Display) -> String {
        let message = error.to_string();
        f.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_or_insert_with(|| message.clone());
        message
    }

    fn text(value: &Value) -> anyhow::Result<String> {
        if let Some(text) = value.as_str() {
            return Ok(text.to_owned());
        }
        let parts = value
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("tool result has no text content"))?;
        anyhow::ensure!(
            parts.len() == 1 && parts[0]["type"] == "text",
            "text-only Computer result required"
        );
        Ok(parts[0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("tool text is missing"))?
            .to_owned())
    }

    fn result(body: &Value, call_id: &str) -> anyhow::Result<Value> {
        let messages = body["messages"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("model request has no messages"))?;
        let matches = messages
            .iter()
            .filter(|message| message["role"] == "tool" && message["tool_call_id"] == call_id)
            .collect::<Vec<_>>();
        let [message] = matches.as_slice() else {
            anyhow::bail!("one exact tool result is required for {call_id}");
        };
        anyhow::ensure!(
            message.get("is_error") != Some(&Value::Bool(true)),
            "tool returned an error: {call_id}"
        );
        serde_json::from_str(&text(&message["content"])?).map_err(Into::into)
    }

    fn envelope(value: &Value) -> anyhow::Result<&str> {
        let root = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Computer result must be an object"))?;
        let result = value["result"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Computer result body missing"))?;
        anyhow::ensure!(
            root.len() == 2
                && value["generation"].as_u64() == Some(0)
                && result.len() == 1
                && result.contains_key("text"),
            "Computer result must have generation 0 and text only; no input authority or media"
        );
        result["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Computer text missing"))
    }

    fn cursor(value: &Value) -> anyhow::Result<(i32, i32)> {
        let text = envelope(value)?;
        let coordinates = text
            .strip_prefix("Cursor position: (")
            .and_then(|text| {
                text.strip_suffix(") in screen coordinates (no screenshot taken yet).")
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "expected real screen-coordinate cursor observation, without screenshot cache"
                )
            })?;
        let (x, y) = coordinates
            .split_once(", ")
            .ok_or_else(|| anyhow::anyhow!("cursor coordinates missing"))?;
        let point = (x.parse::<i32>()?, y.parse::<i32>()?);
        anyhow::ensure!(
            coordinates == format!("{}, {}", point.0, point.1),
            "invalid cursor coordinate carrier"
        );
        Ok(point)
    }

    fn launch(value: &Value, name: &str) -> anyhow::Result<()> {
        anyhow::ensure!(envelope(value)? == format!("Opened {name:?} via macOS LaunchServices. Take a screenshot or run `observe` to see the result."),
            "expected exact successful LaunchServices handoff for the owned audit App");
        Ok(())
    }

    fn advertised(body: &Value, action: &str, arguments: &Value) -> anyhow::Result<String> {
        let tools = body["tools"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("model request has no advertised tools"))?;
        let marker = format!("Action: {action}.");
        let matches = tools
            .iter()
            .filter(|tool| {
                tool["function"]["description"]
                    .as_str()
                    .is_some_and(|text| text.contains(&marker))
            })
            .collect::<Vec<_>>();
        let [tool] = matches.as_slice() else {
            anyhow::bail!("one advertised {action} is required");
        };
        let function = &tool["function"];
        anyhow::ensure!(
            function["parameters"].is_object()
                && jsonschema::validator_for(&function["parameters"])?.is_valid(arguments),
            "operation does not satisfy the current advertised {action} schema"
        );
        function["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("advertised tool name missing"))
    }

    fn marker(config: &Config) -> anyhow::Result<Option<Marker>> {
        let before = match fs::symlink_metadata(&config.marker) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(metadata) => metadata,
        };
        anyhow::ensure!(
            before.is_file()
                && !before.file_type().is_symlink()
                && before.nlink() == 1
                && before.uid() == fs::metadata(&config.data_dir)?.uid()
                && before.len() <= 4096,
            "marker must be the owned, bounded, regular fixture file"
        );
        let file = fs::File::open(&config.marker)?;
        let opened = file.metadata()?;
        anyhow::ensure!(
            opened.dev() == before.dev() && opened.ino() == before.ino(),
            "marker identity changed before read"
        );
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 4096, "marker exceeded its bound");
        let marker: Marker = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            marker.pid > 1
                && marker.stage == "didFinishLaunching"
                && marker.bundle_identifier == config.bundle_id
                && marker.bundle_path == config.app_path,
            "marker does not identify the exact independently prepared audit App"
        );
        Ok(Some(marker))
    }

    fn frame(delta: Value, finish: Option<&str>) -> String {
        format!(
            "data: {}\n\n",
            json!({"id":"macos-computer-gui","object":"chat.completion.chunk","created":1,
            "model":MODEL,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
        )
    }

    fn stream(delta: Value, finish: &str) -> axum::response::Response {
        axum::response::Response::builder()
            .header("content-type", "text/event-stream")
            .body(axum::body::Body::from(format!(
                "{}{}data: [DONE]\n\n",
                frame(delta, None),
                frame(json!({}), Some(finish))
            )))
            .unwrap()
    }

    async fn model(
        State(f): State<Arc<Fixture>>,
        Json(body): Json<Value>,
    ) -> axum::response::Response {
        let index = f.calls.fetch_add(1, Ordering::SeqCst);
        let reply = model_reply(&f, &body, index).await;
        match reply {
            Ok((delta, reason)) => stream(delta, reason),
            Err(error) => axum::response::Response::builder()
                .status(400)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    json!({"error":{"message":fail(&f,error),"type":"fixture_error"}}).to_string(),
                ))
                .unwrap(),
        }
    }

    async fn model_reply(
        f: &Fixture,
        body: &Value,
        index: usize,
    ) -> anyhow::Result<(Value, &'static str)> {
        anyhow::ensure!(
            f.failure.lock().unwrap().is_none() && index < 3,
            "fixed three-response fixture refuses retries or extra model requests"
        );
        anyhow::ensure!(
            body["stream"] == true && body["tools"].is_array(),
            "expected native streaming tool request, not an extra context/summarization request"
        );
        let messages = body["messages"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("messages missing"))?;
        let tool_ids = messages
            .iter()
            .filter(|message| message["role"] == "tool")
            .map(|message| {
                message["tool_call_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("tool result ID missing"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let expected_ids: &[&str] = match index {
            0 => &[],
            1 => &[CURSOR],
            2 => &[CURSOR, LAUNCH],
            _ => unreachable!(),
        };
        anyhow::ensure!(
            tool_ids == expected_ids,
            "unexpected, missing or repeated tool result in fixed journey"
        );
        if index == 0 {
            anyhow::ensure!(
                marker(&f.config)?.is_none(),
                "audit App marker predates this Turn"
            );
        }
        if index >= 1 {
            let value = result(body, CURSOR)?;
            cursor(&value)?;
            let mut evidence = f.evidence.lock().unwrap();
            if let Some(previous) = &evidence.cursor {
                anyhow::ensure!(
                    *previous == value,
                    "cursor result changed between model steps"
                );
            }
            evidence.cursor = Some(value);
        }
        if index == 2 {
            let value = result(body, LAUNCH)?;
            launch(&value, &f.config.app_name)?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            let native_marker = loop {
                if let Some(marker) = marker(&f.config)? {
                    break marker;
                }
                anyhow::ensure!(
                    tokio::time::Instant::now() < deadline,
                    "LaunchServices handoff returned but exact audit App marker was not observed"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            let mut evidence = f.evidence.lock().unwrap();
            let point = cursor(evidence.cursor.as_ref().unwrap())?;
            evidence.launch = Some(value);
            evidence.marker = Some(native_marker.clone());
            evidence.final_response_emitted = true;
            return Ok((
                json!({"role":"assistant","content":format!(
                "正式 Computer 工具返回光标位置 ({}, {})，观察版本为 0。LaunchServices 已接收自有应用 {} 的启动请求；独立验收服务读取到其 didFinishLaunching 标记（PID {}）。此旅程使用本机固定模型，仅执行光标查询和指定应用启动，没有调用完成报告工具。辅助功能、屏幕捕获和真实键鼠输入仍未由本次旅程验收。",
                point.0, point.1, f.config.app_name, native_marker.pid)}),
                "stop",
            ));
        }
        let (call_id, action, arguments) = if index == 0 {
            (
                CURSOR,
                "computer/observe",
                json!({"action":"cursor_position"}),
            )
        } else {
            (
                LAUNCH,
                "computer/launch",
                json!({"action":"launch","target":f.config.app_name}),
            )
        };
        let name = advertised(body, action, &arguments)?;
        let mut evidence = f.evidence.lock().unwrap();
        if index == 0 {
            evidence.observe_name = Some(name.clone());
        } else {
            evidence.launch_name = Some(name.clone());
        }
        Ok((
            json!({"role":"assistant","tool_calls":[{"index":0,"id":call_id,"type":"function",
            "function":{"name":name,"arguments":arguments.to_string()}}]}),
            "tool_calls",
        ))
    }

    fn boundaries() -> Value {
        json!({"provider":"localhost fixed ChatCompletion SSE fixture, not a live model",
            "device_scope":["cursor_position","owned audit App bare launch"],
            "a11y_pixel_input_tcc_acceptance":false,"completion_reported":false,"task_ledger":false,
            "formal_dev_ui_identity_requires_operator_evidence":true,
            "marker_source":"independent read-only fixture verifier, not a Runtime Computer observation"})
    }

    async fn report_inner(f: &Fixture) -> anyhow::Result<Value> {
        let failure = f.failure.lock().unwrap().clone();
        anyhow::ensure!(
            failure.is_none(),
            "fixture recorded first failure: {}",
            failure.unwrap_or_default()
        );
        let session_id = f
            .session_id
            .get()
            .ok_or_else(|| anyhow::anyhow!("setup not ready"))?;
        // This is a read-only connection to the explicitly isolated dataset.
        // Store::from_pool validates its existing schema; no startup/rebuild,
        // event append, projection-to-context fallback or second ledger exists.
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(
                        nomifun_app::AppConfig {
                            data_dir: f.config.data_dir.clone(),
                            ..Default::default()
                        }
                        .database_path(),
                    )
                    .create_if_missing(false)
                    .read_only(true)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await?;
        let store = AgentSessionStore::from_pool(pool.clone()).await?;
        let checked = canonical_report(f, &store, session_id).await;
        pool.close().await;
        checked
    }

    async fn canonical_report(
        f: &Fixture,
        store: &AgentSessionStore,
        session_id: &AgentSessionId,
    ) -> anyhow::Result<Value> {
        let page = store
            .read_events(session_id, None, nomifun_agent_session::MAX_EVENT_PAGE_SIZE)
            .await?;
        let head = store.head(session_id).await?;
        anyhow::ensure!(
            page.events.len() < nomifun_agent_session::MAX_EVENT_PAGE_SIZE as usize
                || page.next_cursor.seq == head.last_seq,
            "canonical facts exceeded the fixed fixture bound"
        );
        let starts = page
            .events
            .iter()
            .filter(|event| event.kind.0 == "turn/started")
            .collect::<Vec<_>>();
        if starts.is_empty()
            || head.active_turn_id.is_some()
            || page.next_cursor.seq != head.last_seq
        {
            return Ok(
                json!({"passed":false,"pending":true,"calls":f.calls.load(Ordering::SeqCst),"reason":"exact canonical Turn is not terminal","boundaries":boundaries()}),
            );
        }
        let [started] = starts.as_slice() else {
            anyhow::bail!("expected exactly one audit Turn");
        };
        let turn_id = nomifun_agent_contracts::OperationId::from(started.correlation_id.as_ref());
        let receipt = store.read_turn_receipt(session_id, &turn_id).await?;
        anyhow::ensure!(
            receipt.status == TurnReceiptStatus::Completed
                && receipt
                    .terminal_event
                    .as_ref()
                    .is_some_and(|event| event.kind.0 == "turn/completed")
                && head.status == "ready",
            "audit Turn did not complete successfully"
        );
        let terminal = receipt.terminal_event.as_ref().unwrap();
        let SessionEventPayloadRef::InlineJson(payload) = &terminal.payload else {
            anyhow::bail!("bounded canonical Turn terminal must be inline");
        };
        anyhow::ensure!(
            terminal.kind_version == 1
                && payload.0["model_steps"].as_u64() == Some(3)
                && payload.0["finish_reason"] == "completed",
            "canonical terminal did not close the exact three successful model responses"
        );
        anyhow::ensure!(
            f.calls.load(Ordering::SeqCst) == 3,
            "unexpected model request count"
        );
        let live = store.get_live_session(session_id).await?;
        let [resource] = live.agent_binding.typed_resource_bindings.as_slice() else {
            anyhow::bail!("expected the sole explicit Computer binding");
        };
        anyhow::ensure!(
            resource.resource_kind.as_ref() == "computer"
                && resource.resource_id.as_ref() == "local-desktop"
                && resource.operations
                    == std::collections::BTreeSet::from([
                        "observe".to_owned(),
                        "launch".to_owned()
                    ]),
            "audit resources widened beyond observe and launch"
        );
        let evidence = f.evidence.lock().unwrap().clone();
        anyhow::ensure!(
            evidence.final_response_emitted && evidence.marker.is_some(),
            "final fixed response or native marker missing"
        );
        let expected = [
            (
                CURSOR,
                "computer/observe",
                evidence.observe_name.as_deref().unwrap(),
                json!({"action":"cursor_position"}),
                evidence.cursor.as_ref().unwrap(),
            ),
            (
                LAUNCH,
                "computer/launch",
                evidence.launch_name.as_deref().unwrap(),
                json!({"action":"launch","target":f.config.app_name}),
                evidence.launch.as_ref().unwrap(),
            ),
        ];
        let mut started_ids = Vec::new();
        let mut operations = Vec::new();
        let tool_starts = page
            .events
            .iter()
            .filter(|event| event.kind.0 == "tool/call-started")
            .collect::<Vec<_>>();
        let tool_results = page
            .events
            .iter()
            .filter(|event| event.kind.0 == "tool/result-recorded")
            .collect::<Vec<_>>();
        anyhow::ensure!(
            tool_starts.len() == 2 && tool_results.len() == 2,
            "expected exactly two canonical physical tool calls/results"
        );
        for (id, action, name, arguments, observed) in &expected {
            let matches = tool_starts
                .iter()
                .filter(|event| {
                    matches!(&event.payload,
                SessionEventPayloadRef::InlineJson(value) if value.0["call_id"] == *id)
                })
                .collect::<Vec<_>>();
            let [event] = matches.as_slice() else {
                anyhow::bail!("exact canonical tool start missing/ambiguous: {id}");
            };
            let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
                unreachable!()
            };
            anyhow::ensure!(
                event.kind_version == 1
                    && event.producer_id.as_ref() == "runtime_supervisor"
                    && event.causation_event_id.as_ref() == Some(&started.event_id)
                    && payload.0["capability_id"] == "computer"
                    && payload.0["action_id"] == *action
                    && payload.0["name"] == *name,
                "canonical tool identity differs from the fixed advertised action"
            );
            let operation = payload.0["operation_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("canonical tool operation missing"))?;
            anyhow::ensure!(
                operation == format!("{}:tool:{id}", turn_id.as_ref()),
                "tool operation is not the exact per-call identity"
            );
            let matches = tool_results
                .iter()
                .filter(|result| result.causation_event_id.as_ref() == Some(&event.event_id))
                .collect::<Vec<_>>();
            let [result] = matches.as_slice() else {
                anyhow::bail!("exact canonical tool result missing/ambiguous: {id}");
            };
            let SessionEventPayloadRef::InlineJson(payload) = &result.payload else {
                anyhow::bail!("bounded canonical tool result must be inline");
            };
            anyhow::ensure!(
                payload.0["operation_id"] == operation
                    && payload.0["call_id"] == *id
                    && payload.0["error"].is_null(),
                "canonical tool result identity/error mismatch"
            );
            let result: AgentToolResult = serde_json::from_value(payload.0["output"].clone())?;
            anyhow::ensure!(
                result.call_id.as_ref() == *id
                    && !result.is_error
                    && serde_json::from_str::<Value>(&result.output_text())? == **observed,
                "canonical output differs from the actual provider transcript"
            );
            let proposal_count = page
                .events
                .iter()
                .filter(|event| event.kind.0 == "runtime/progress-recorded")
                .filter_map(|event| {
                    let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
                        return None;
                    };
                    (payload.0.pointer("/event/event").and_then(Value::as_str)
                        == Some("tool_call_completed"))
                    .then(|| serde_json::from_value::<AgentEngineEvent>(payload.0["event"].clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let matching = proposal_count.iter().filter(|event| matches!(event, AgentEngineEvent::ToolCallCompleted {call,..}
                if call.call_id.as_ref() == *id && call.name == *name && call.arguments == StrictJsonValue(arguments.clone()))).count();
            anyhow::ensure!(
                proposal_count.len() == 2 && matching == 1,
                "canonical proposal arguments widened or repeated the fixed journey"
            );
            started_ids.push(event.event_id.clone());
            operations.push(operation.to_owned());
        }
        for event in &page.events {
            if event.kind.0 != "runtime/progress-recorded" {
                continue;
            }
            let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
                anyhow::bail!("unexpected stored native progress in the bounded fixture");
            };
            if matches!(
                payload.0.pointer("/event/event").and_then(Value::as_str),
                Some(
                    "runtime_modules_activated"
                        | "completion_reported"
                        | "completion_candidate_recorded"
                        | "completion_delivered"
                )
            ) {
                let native: AgentEngineEvent = serde_json::from_value(payload.0["event"].clone())?;
                match native {
                    AgentEngineEvent::RuntimeModulesActivated { modules, .. } => anyhow::ensure!(
                        !modules.contains(&nomifun_agent_runtime::AgentRuntimeModule::TaskLedger),
                        "Computer fixture unexpectedly activated task ledger"
                    ),
                    _ => anyhow::bail!(
                        "Computer-only fixed journey unexpectedly produced a completion report"
                    ),
                }
            }
        }
        let effects = store.list_effects(session_id).await?;
        let [effect] = effects.as_slice() else {
            anyhow::bail!("expected exactly one canonical launch effect");
        };
        anyhow::ensure!(
            effect.turn_id == turn_id
                && effect.operation_id.as_ref() == operations[1]
                && effect.capability_module.as_ref() == "computer"
                && effect.action_id.as_ref() == "computer/launch"
                && effect.resource_binding_id.as_ref() == Some(&resource.binding_id)
                && effect.strategy == EffectStrategy::ManagedEffect
                && effect.state == AgentEffectState::Returned,
            "launch effect did not settle through the exact managed Computer owner"
        );
        anyhow::ensure!(
            effect.bounded_observation.as_ref().is_some_and(
                |observation| observation["result"] == *evidence.launch.as_ref().unwrap()
            ),
            "managed owner observation differs from the retained launch result"
        );
        let terminal_id = effect
            .terminal_event_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("launch terminal fact missing"))?;
        anyhow::ensure!(
            page.events
                .iter()
                .any(|event| event.event_id == *terminal_id && event.kind.0 == "effect/succeeded")
                && page
                    .events
                    .iter()
                    .any(|event| event.event_id == effect.started_event_id
                        && event.kind.0 == "effect/started"
                        && event.causation_event_id.as_ref() == Some(&started_ids[1]))
                && !store.has_unsettled_effects(session_id).await?,
            "launch effect causation/terminal/settlement mismatch"
        );
        let current_marker =
            marker(&f.config)?.ok_or_else(|| anyhow::anyhow!("native audit marker disappeared"))?;
        anyhow::ensure!(
            evidence.marker.as_ref() == Some(&current_marker),
            "the accepted native marker changed after the final model response"
        );
        Ok(
            json!({"passed":true,"pending":false,"calls":3,"session_id":session_id,"turn_id":turn_id,
            "tool_call_ids":[CURSOR,LAUNCH],"tool_started_event_ids":started_ids,
            "effect_id":effect.effect_id,"effect_terminal_event_id":terminal_id,
            "turn_terminal_event_id":receipt.terminal_event.unwrap().event_id,
            "native_marker":current_marker,"boundaries":boundaries()}),
        )
    }

    async fn report(State(f): State<Arc<Fixture>>) -> Json<Value> {
        match report_inner(&f).await {
            Ok(value) => Json(value),
            Err(error) => Json(
                json!({"passed":false,"pending":false,"calls":f.calls.load(Ordering::SeqCst),
                "failure":error.to_string(),"first_model_failure":f.failure.lock().unwrap().clone(),"boundaries":boundaries()}),
            ),
        }
    }

    fn setup_http_diagnostic(
        stage: &str,
        method: &reqwest::Method,
        path: &str,
        status: Option<u16>,
        content_type: &str,
        bytes: usize,
        classification: &str,
    ) -> Value {
        json!({"stage":stage,"method":method.as_str(),"path":path,"status":status,
            "content_type":content_type.chars().filter(char::is_ascii).take(96).collect::<String>(),
            "received_bytes":bytes,"classification":classification,"body_logged":false})
    }

    fn setup_http_failure(diagnostic: Value) -> anyhow::Error {
        eprintln!("MACOS_COMPUTER_GUI_SETUP_HTTP {diagnostic}");
        anyhow::anyhow!("fixture setup HTTP stage failed: {diagnostic}")
    }

    async fn api(
        app: &DesktopServer,
        stage: &str,
        method: reqwest::Method,
        path: &str,
        body: Value,
    ) -> anyhow::Result<Value> {
        const MAX_SETUP_RESPONSE_BYTES: usize = 256 * 1024;
        let request = reqwest::Client::builder()
            .no_proxy()
            .build()?
            .request(
                method.clone(),
                format!("http://127.0.0.1:{}{path}", app.loopback_port()),
            )
            .header("x-nomi-local-trust", app.local_trust_secret())
            .json(&body)
            .send()
            .await;
        let mut response = request.map_err(|error| {
            let class = if error.is_timeout() {
                "transport_timeout"
            } else if error.is_connect() {
                "transport_connect"
            } else {
                "transport_request"
            };
            setup_http_failure(setup_http_diagnostic(
                stage, &method, path, None, "", 0, class,
            ))
        })?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("absent")
            .to_owned();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            setup_http_failure(setup_http_diagnostic(
                stage,
                &method,
                path,
                Some(status.as_u16()),
                &content_type,
                bytes.len(),
                "response_body_transport",
            ))
        })? {
            if chunk.len() > MAX_SETUP_RESPONSE_BYTES.saturating_sub(bytes.len()) {
                return Err(setup_http_failure(setup_http_diagnostic(
                    stage,
                    &method,
                    path,
                    Some(status.as_u16()),
                    &content_type,
                    bytes.len().saturating_add(chunk.len()),
                    "response_body_exceeded_256kib_bound",
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        // Status and media type precede decoding: JSON-extractor rejections,
        // empty method responses and HTML cannot become anonymous decode errors.
        let diagnostic = |class| {
            setup_http_diagnostic(
                stage,
                &method,
                path,
                Some(status.as_u16()),
                &content_type,
                bytes.len(),
                class,
            )
        };
        if !status.is_success() {
            return Err(setup_http_failure(diagnostic(if bytes.is_empty() {
                "http_non_success_empty_body"
            } else {
                "http_non_success_body_omitted"
            })));
        }
        if bytes.is_empty() {
            return Err(setup_http_failure(diagnostic(
                "successful_response_empty_body",
            )));
        }
        let media = content_type.split(';').next().unwrap_or("").trim();
        if media != "application/json" && !media.ends_with("+json") {
            return Err(setup_http_failure(diagnostic(
                "successful_response_not_json_media_type",
            )));
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
            let mut detail = diagnostic("successful_response_invalid_json");
            detail["json_error_category"] = json!(format!("{:?}", error.classify()));
            detail["json_line"] = json!(error.line());
            detail["json_column"] = json!(error.column());
            setup_http_failure(detail)
        })?;
        let data = value
            .get("data")
            .filter(|data| !data.is_null())
            .cloned()
            .ok_or_else(|| {
                setup_http_failure(diagnostic("successful_response_missing_api_data"))
            })?;
        eprintln!(
            "MACOS_COMPUTER_GUI_SETUP_HTTP {}",
            diagnostic("accepted_json_api_data")
        );
        Ok(data)
    }

    pub(super) async fn run() -> anyhow::Result<()> {
        let f = Arc::new(Fixture {
            config: config()?,
            calls: AtomicUsize::new(0),
            evidence: Mutex::new(Evidence::default()),
            failure: Mutex::new(None),
            session_id: std::sync::OnceLock::new(),
            stop: CancellationToken::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let router = Router::new().route("/v1/chat/completions",post(model)).route("/report",get(report))
            .route("/status",get(|State(f):State<Arc<Fixture>>|async move {Json(json!({"calls":f.calls.load(Ordering::SeqCst),
                "failure":f.failure.lock().unwrap().clone(),"final_response_emitted":f.evidence.lock().unwrap().final_response_emitted,"boundaries":boundaries()}))}))
            .route("/shutdown",post(|State(f):State<Arc<Fixture>>|async move {f.stop.cancel(); "stopped"})).with_state(f.clone());
        let stop = f.stop.clone();
        let service = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
        });
        let cli = nomifun_app::cli::Cli {
            host: "127.0.0.1".into(),
            port: 0,
            data_dir: f.config.data_dir.clone(),
            work_dir: None,
            app_version: env!("CARGO_PKG_VERSION").into(),
            local: true,
            log_dir: Some(f.config.data_dir.join("logs")),
            log_level: Some("off".into()),
            command: None,
        };
        let (app, keep_alive) = DesktopServer::start_with_outcome(
            &cli,
            "",
            None,
            None,
            None,
            DesktopHostServices::default(),
        )
        .await?;
        let prepared = async {
            let provider = api(&app,"create_provider",reqwest::Method::POST,"/api/providers",json!({"platform":"custom","name":"macOS 最小 Computer 验收固定模型",
                "base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,
                "initial_model":{"model":MODEL,"enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
            let provider = provider["provider_id"].as_str().ok_or_else(||anyhow::anyhow!("provider missing"))?;
            let editor = api(&app,"create_preset_from_template",reqwest::Method::POST,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"macOS 最小 Computer 正式入口验收",
                "model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":MODEL}})).await?;
            let preset = editor["preset"]["preset_id"].as_str().ok_or_else(||anyhow::anyhow!("preset missing"))?;
            let mut draft = editor["draft"].clone();
            let capabilities = json!([{"capability":{"id":"computer"},"action_allowlist":["computer/observe","computer/launch"]}]);
            draft["document"]["enabled_capabilities"] = capabilities.clone();
            draft["document"]["skill_bindings"] = json!([]);
            let typed_draft: nomifun_api_types::AgentPresetDraftDto = serde_json::from_value(draft.clone()).map_err(|error|
                anyhow::anyhow!("fixture setup draft schema preflight failed: category={:?}, line={}, column={}; body omitted",error.classify(),error.line(),error.column()))?;
            anyhow::ensure!(typed_draft.document.skill_bindings.is_empty(), "fixture draft must not retain Package Skills");
            let saved = api(&app,"save_preset_revision",reqwest::Method::POST,&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":draft["current_revision"],
                "draft":draft,"reason":"isolated canonical Computer cursor/owned bare launch acceptance"})).await?;
            let saved_capabilities: Vec<nomifun_api_types::CapabilitySelectionDto> = serde_json::from_value(saved["revision"]["document"]["enabled_capabilities"].clone())?;
            let expected_capabilities: Vec<nomifun_api_types::CapabilitySelectionDto> = serde_json::from_value(capabilities)?;
            anyhow::ensure!(saved_capabilities == expected_capabilities, "frozen Computer actions changed during setup");
            let session = api(&app,"create_agent_session",reqwest::Method::POST,"/api/agent-sessions",json!({"preset_id":preset,"title":"macOS 光标与自有 audit App 启动验收",
                "resource_selections":[{"resource_kind":"computer","resource_id":"local-desktop"}],"model":{"provider_id":provider,"model":MODEL}})).await?;
            Ok::<_,anyhow::Error>(AgentSessionId::from(session["agent_session_id"].as_str().ok_or_else(||anyhow::anyhow!("session missing"))?))
        }.await;
        app.shutdown_all().await?;
        drop(app);
        drop(keep_alive);
        let session = prepared?;
        f.session_id
            .set(session.clone())
            .map_err(|_| anyhow::anyhow!("Session already prepared"))?;
        println!(
            "MACOS_COMPUTER_GUI_FIXTURE_READY {}",
            json!({"data_dir":f.config.data_dir,"control":format!("http://{address}"),
            "session_id":session,"app_name":f.config.app_name,"marker":f.config.marker,"bundle_identifier":f.config.bundle_id,"app_path":f.config.app_path,
            "prompt":format!("请仅查询一次光标位置，然后仅启动自有应用 {} 一次，并说明返回结果。失败立即停止，不重试、不截图、不列举窗口、不读取辅助功能树、不键入或点击任何应用。",f.config.app_name),
            "setup_backend_stopped":true,"expected_model_responses":3,"boundaries":boundaries()})
        );
        f.stop.cancelled().await;
        service.await??;
        Ok(())
    }
}
