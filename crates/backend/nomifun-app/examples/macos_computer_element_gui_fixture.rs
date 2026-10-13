//! Fixed owned-window native element acceptance through the formal Dev Agent.
//! Args: NEW_DATA_DIR OWNED_APP_NAME PRIVATE_WITNESS_JSON BUNDLE_ID APP_PATH.
//! After Agent launch, root must confirm the actual owned window in CUA and POST
//! {nonce,pid,window_id,window_title} to /confirm-owned-frontmost before any AX call.
//! Observe retains frontmost semantics; this is not PID hard isolation.
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

    const MODEL: &str = "macos-computer-elements-fixture";
    const PREFIX: &str = "NomiFun Element Audit ";
    const REPLACEMENT: &str = "完整替换 中文Ω😀";
    const UNSUPPORTED: &str = "COMPUTER_INPUT_UNSUPPORTED_NO_EFFECT: set_element_value was rejected before native mutation. No pointer or keyboard fallback was admitted.";
    fn public_unsupported(model_context: bool) -> String {
        let code = "ROLE_HOST_UNSUPPORTED_INPUT";
        let message = nomifun_agent_kernel::KernelError::capability_execution_failed(
            code, UNSUPPORTED,
        ).to_string();
        if model_context {
            nomifun_agent_runtime::AgentEngineError::CapabilityKernel {
                code: code.into(), message,
            }.to_string()
        } else {
            nomifun_engine_core::EngineToolError::CapabilityKernel {
                code: code.into(), message,
            }.to_string()
        }
    }
    const IDS: [&str; 11] = [
        "elements-launch",
        "elements-observe-left",
        "elements-left",
        "elements-observe-right",
        "elements-right",
        "elements-observe-double",
        "elements-double",
        "elements-observe-value",
        "elements-value",
        "elements-observe-readonly",
        "elements-readonly",
    ];
    const CONTROLS: [&str; 5] = ["left", "right", "double", "value", "readonly"];
    const ACTIONS: [&str; 5] = [
        "click_element",
        "right_click_element",
        "double_click_element",
        "set_element_value",
        "set_element_value",
    ];
    #[derive(Clone)]
    struct Emitted {
        id: String,
        name: String,
        action: String,
        arguments: Value,
    }
    struct Fixture {
        config: Config,
        calls: AtomicUsize,
        failure: Mutex<Option<String>>,
        session_id: std::sync::OnceLock<AgentSessionId>,
        stop: CancellationToken,
        emitted: Mutex<Vec<Emitted>>,
        observations: Mutex<Vec<(u64, u32)>>,
        results: Mutex<Vec<Value>>,
        witnesses: Mutex<Vec<Value>>,
        value_input_baseline: Mutex<Option<Value>>,
        ready_gate: tokio::sync::Semaphore,
        gate_waiting: std::sync::atomic::AtomicBool,
        confirmed: std::sync::atomic::AtomicBool,
        final_emitted: std::sync::atomic::AtomicBool,
    }
    struct Config {
        data_dir: PathBuf,
        marker: PathBuf,
        app_name: String,
        bundle_id: String,
        app_path: PathBuf,
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
            bundle_id == format!("com.nomifun.desktop.audit.elements.{suffix}"),
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

    fn frame(delta: Value, finish: Option<&str>) -> String {
        format!(
            "data: {}\n\n",
            json!({"id":"macos-computer-elements-gui","object":"chat.completion.chunk","created":1,
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

    fn nonce(config: &Config) -> &str {
        config.app_name.strip_prefix(PREFIX).unwrap()
    }
    fn label(config: &Config, control: &str) -> String {
        format!("NomiFun audit {control} {}", nonce(config))
    }
    fn window_title(config: &Config) -> String {
        format!("NomiFun macOS audit elements {}", nonce(config))
    }

    fn witness(config: &Config) -> anyhow::Result<Option<Value>> {
        let before = match fs::symlink_metadata(&config.marker) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(value) => value,
        };
        anyhow::ensure!(
            before.is_file()
                && !before.file_type().is_symlink()
                && before.nlink() == 1
                && before.uid() == fs::metadata(&config.data_dir)?.uid()
                && before.len() <= 128 * 1024,
            "owned bounded regular witness required"
        );
        let file = fs::File::open(&config.marker)?;
        let opened = file.metadata()?;
        anyhow::ensure!(
            opened.ino() == before.ino() && opened.dev() == before.dev(),
            "witness identity changed before read"
        );
        let mut bytes = Vec::new();
        file.take(128 * 1024 + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 128 * 1024, "witness exceeded bound");
        let value: Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            value["schema_version"] == 1
                && value["nonce"] == nonce(config)
                && value["pid"]
                    .as_u64()
                    .is_some_and(|pid| pid > 1 && pid <= i32::MAX as u64)
                && value["bundle_identifier"] == config.bundle_id
                && value["bundle_path"] == config.app_path.to_string_lossy().as_ref()
                && value["window_title"] == window_title(config)
                && value["window_id"].as_u64().is_some_and(|id| id > 0)
                && value["evidence_source"] == "owned_NSWindow_dispatch_and_control_callbacks"
                && value["native_recorder_evidence"] == false
                && value["dropped_event_details"] == 0,
            "witness does not identify the exact owned fixture window"
        );
        for control in CONTROLS {
            anyhow::ensure!(
                value["controls"][control]["label"] == label(config, control)
                    && value["controls"][control]["nonce"] == nonce(config),
                "owned control witness identity mismatch"
            );
        }
        Ok(Some(value))
    }

    fn raw_result(body: &Value, id: &str) -> anyhow::Result<String> {
        let matches = body["messages"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("messages missing"))?
            .iter()
            .filter(|value| value["role"] == "tool" && value["tool_call_id"] == id)
            .collect::<Vec<_>>();
        let [value] = matches.as_slice() else {
            anyhow::bail!("exact unique result missing");
        };
        anyhow::ensure!(value["is_error"] == true, "expected a model-visible typed tool error");
        text(&value["content"])
    }

    fn envelope_text(value: &Value, positive_generation: bool) -> anyhow::Result<&str> {
        let root = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Computer result object missing"))?;
        let result = value["result"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Computer result body missing"))?;
        let generation = value["generation"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("canonical generation missing"))?;
        anyhow::ensure!(
            root.len() == 2
                && result.len() == 1
                && result.contains_key("text")
                && if positive_generation {
                    generation > 0
                } else {
                    generation == 0
                },
            "expected pure text canonical Computer envelope"
        );
        result["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("text missing"))
    }

    fn observe_target(config: &Config, value: &Value, control: &str) -> anyhow::Result<(u64, u32)> {
        let text = envelope_text(value, true)?;
        anyhow::ensure!(
            text.starts_with("Accessibility snapshot (gen ")
                && text.contains(&format!(" in {}.", config.app_name))
                && !text.contains("truncated")
                && text.contains(&format!("{:?}", window_title(config))),
            "AX observation did not identify the confirmed owned nonce window; raw AX omitted"
        );
        let role = if matches!(control, "value" | "readonly") {
            "textfield"
        } else {
            "group"
        };
        let suffix = format!("] {role} {:?}", label(config, control));
        let matches = text
            .lines()
            .filter(|line| line.contains(&suffix))
            .collect::<Vec<_>>();
        let [line] = matches.as_slice() else {
            anyhow::bail!("owned control reference missing or ambiguous; raw AX omitted");
        };
        let reference = line
            .strip_prefix('[')
            .and_then(|line| line.split_once(']'))
            .ok_or_else(|| anyhow::anyhow!("AX ref carrier invalid"))?
            .0
            .parse::<u32>()?;
        anyhow::ensure!(reference > 0, "AX ref must be positive");
        Ok((value["generation"].as_u64().unwrap(), reference))
    }

    fn check_control(config: &Config, value: &Value, slot: usize) -> anyhow::Result<()> {
        let control = CONTROLS[slot];
        let c = &value["controls"][control];
        let count = |key: &str| c[key].as_u64().unwrap_or(u64::MAX);
        match slot {
            0 => anyhow::ensure!(
                count("mouse_downs") == 1
                    && count("right_downs") == 0
                    && c["click_counts"] == json!([1])
                    && c["received_buttons"] == json!([0])
                    && c["press_action_declared"] == false,
                "ordinary click did not deliver exactly one owned native left event"
            ),
            1 => anyhow::ensure!(
                count("right_downs") == 1
                    && count("mouse_downs") == 0
                    && c["click_counts"] == json!([1])
                    && c["received_buttons"] == json!([1]),
                "right click witness differs"
            ),
            2 => anyhow::ensure!(
                count("mouse_downs") == 2
                    && count("right_downs") == 0
                    && c["click_counts"] == json!([1, 2])
                    && c["received_buttons"] == json!([0, 0]),
                "double click did not deliver two events with native clickCount 2"
            ),
            3 => anyhow::ensure!(
                c["value"] == REPLACEMENT
                    && c["ax_setter_instrumentation"] == false
                    && c["value_snapshot_seen"] == true
                    && c["value_before"] == "seed 非空 α😀"
                    && c["value_after"] == REPLACEMENT
                    && c["editing_active"] == false
                    && c["control_value"] == REPLACEMENT
                    && c["cell_value"] == REPLACEMENT
                    && c["field_editor_value"].is_null()
                    && c["control_class"] == "NSTextField"
                    && c["cell_class"].as_str().is_some_and(|class| !class.is_empty())
                    && count("observed_change_count") == 1
                    && c["observed_changes"].as_array().is_some_and(|changes| {
                        changes.len() == 1
                            && changes[0]["before"]["editing_active"] == false
                            && changes[0]["before"]["control_value"] == "seed 非空 α😀"
                            && changes[0]["before"]["cell_value"] == "seed 非空 α😀"
                            && changes[0]["before"]["field_editor_value"].is_null()
                            && changes[0]["after"]["editing_active"] == false
                            && changes[0]["after"]["control_value"] == REPLACEMENT
                            && changes[0]["after"]["cell_value"] == REPLACEMENT
                            && changes[0]["after"]["field_editor_value"].is_null()
                    })
                    && c["keys"].as_object().is_some(),
                "standard native text field/control/cell did not replace the complete nonempty value without pointer or keyboard input"
            ),
            4 => anyhow::ensure!(
                c["value"] == "readonly 非空 β😀"
                    && count("setter_requests") == 0
                    && count("super_calls") == 0
                    && count("pointer_total") == 0
                    && c["keys"].as_object().is_some_and(|keys| keys.is_empty()),
                "readonly setter admitted pointer/key/setter work"
            ),
            _ => unreachable!(),
        }
        if slot < 3 {
            let event = &c["last_receiver_event"];
            anyhow::ensure!(
                event["pid"] == value["pid"]
                    && event["bundle_identifier"] == config.bundle_id
                    && event["window_id"] == value["window_id"]
                    && event["label"] == label(config, control)
                    && event["nonce"] == nonce(config),
                "native receiver event belongs to a different target"
            );
        }
        Ok(())
    }

    async fn settled_witness(f: &Fixture, slot: usize) -> anyhow::Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(value) = witness(&f.config)? {
                if check_control(&f.config, &value, slot).is_ok() {
                    return Ok(value);
                }
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "owned native witness did not match the single admitted action"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn confirm_frontmost(
        State(f): State<Arc<Fixture>>,
        Json(body): Json<Value>,
    ) -> (axum::http::StatusCode, Json<Value>) {
        let checked = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                f.gate_waiting.load(Ordering::SeqCst) && !f.confirmed.load(Ordering::SeqCst),
                "fixture is not waiting for one owned-frontmost attestation"
            );
            let value =
                witness(&f.config)?.ok_or_else(|| anyhow::anyhow!("owned witness not ready"))?;
            anyhow::ensure!(
                body["nonce"] == value["nonce"]
                    && body["pid"] == value["pid"]
                    && body["window_id"] == value["window_id"]
                    && body["window_title"] == value["window_title"],
                "operator attestation identity mismatch"
            );
            Ok(())
        })();
        match checked {
            Ok(()) => {
                f.confirmed.store(true, Ordering::SeqCst);
                f.ready_gate.add_permits(1);
                (
                    axum::http::StatusCode::OK,
                    Json(
                        json!({"accepted":true,"scope":"operator CUA attestation; not a PID-pinned product observe"}),
                    ),
                )
            }
            Err(error) => (
                axum::http::StatusCode::CONFLICT,
                Json(json!({"accepted":false,"failure":fail(&f,error)})),
            ),
        }
    }

    async fn model(
        State(f): State<Arc<Fixture>>,
        Json(body): Json<Value>,
    ) -> axum::response::Response {
        let index = f.calls.fetch_add(1, Ordering::SeqCst);
        match reply(&f, &body, index).await {
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

    async fn reply(
        f: &Fixture,
        body: &Value,
        index: usize,
    ) -> anyhow::Result<(Value, &'static str)> {
        anyhow::ensure!(
            index < 12 && f.failure.lock().unwrap().is_none(),
            "fixed element journey refuses extra model requests or retries"
        );
        anyhow::ensure!(
            body["stream"] == true && body["tools"].is_array(),
            "unexpected context/model request"
        );
        let ids = body["messages"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("messages missing"))?
            .iter()
            .filter(|message| message["role"] == "tool")
            .map(|message| message["tool_call_id"].as_str().unwrap_or(""))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            ids == IDS[..index],
            "missing, extra or repeated tool result; raw content omitted"
        );
        if index > 0 {
            let previous = index - 1;
            let value = if previous == 10 {
                let error = raw_result(body, IDS[previous])?;
                anyhow::ensure!(
                    error == public_unsupported(true),
                    "readonly action did not return its specific public Unsupported rejection"
                );
                json!({"error":error})
            } else {
                let value = result(body, IDS[previous])?;
                let text = envelope_text(&value, matches!(previous, 1 | 3 | 5 | 7 | 9))?;
                if previous == 0 {
                    anyhow::ensure!(text==format!("Opened {:?} via macOS LaunchServices. Take a screenshot or run `observe` to see the result.",f.config.app_name),"owned launch handoff mismatch");
                }
                if previous == 2 {
                    anyhow::ensure!(
                        text.contains("fell back to a pixel click at the element center"),
                        "ordinary click did not exercise the actual guarded fallback"
                    );
                }
                value
            };
            f.results.lock().unwrap().push(value);
            if matches!(previous, 2 | 4 | 6 | 8 | 10) {
                let slot = (previous - 2) / 2;
                let value = settled_witness(f, slot).await?;
                if slot == 3 {
                    let baseline = f.value_input_baseline.lock().unwrap();
                    let before = baseline.as_ref().ok_or_else(|| anyhow::anyhow!("native value input baseline missing"))?;
                    anyhow::ensure!(before["input_totals"] == value["input_totals"],
                        "native value input interval received pointer or keyboard events");
                    anyhow::ensure!(before["controls"]["value"]["pointer_total"] == value["controls"]["value"]["pointer_total"]
                        && before["controls"]["value"]["keys"] == value["controls"]["value"]["keys"],
                        "native value target input counters changed during its write");
                }
                if slot == 4 {
                    let before = f.witnesses.lock().unwrap().last().unwrap().clone();
                    anyhow::ensure!(
                        before["input_totals"] == value["input_totals"],
                        "readonly refusal dispatched window input"
                    );
                }
                f.witnesses.lock().unwrap().push(value);
            }
        }
        if index == 1 {
            f.gate_waiting.store(true, Ordering::SeqCst);
            let permit=tokio::time::timeout(Duration::from_secs(90),f.ready_gate.acquire()).await
                .map_err(|_|anyhow::anyhow!("owned-frontmost CUA attestation timed out; no AX observation was requested"))??;
            permit.forget();
            f.gate_waiting.store(false, Ordering::SeqCst);
        }
        if index == 8 {
            let before = witness(&f.config)?.ok_or_else(|| anyhow::anyhow!("native value input baseline unavailable"))?;
            anyhow::ensure!(before["controls"]["value"]["control_value"] == "seed 非空 α😀"
                && before["controls"]["value"]["cell_value"] == "seed 非空 α😀"
                && before["controls"]["value"]["editing_active"] == false,
                "standard value target changed before its single admitted write");
            *f.value_input_baseline.lock().unwrap() = Some(before);
        }
        if index == 11 {
            f.final_emitted.store(true, Ordering::SeqCst);
            return Ok((
                json!({"role":"assistant","content":"独立自有窗口已收到一次左键、一次右键及原生双击事件；唯一 canonical 整值写入工具后，未重写 AX 的标准 NSTextField 实际 control/cell 值由非空 seed 完整替换为指定 Unicode 文本，独立只读模型值见证未收到指针或键盘输入。只读字段按预期在效果前拒绝，值保持，未补发指针或键盘输入。本轮使用本机固定模型与 root 前台见证，没有请求截图；观察沿用前台应用语义，不提供 PID 硬绑定或 SDK 内部调用计数承诺，也不证明真实模型。"}),
                "stop",
            ));
        }
        let (action, arguments) = if index == 0 {
            anyhow::ensure!(
                witness(&f.config)?.is_none(),
                "owned witness predates this turn"
            );
            (
                "computer/launch",
                json!({"action":"launch","target":f.config.app_name}),
            )
        } else if index % 2 == 1 {
            ("computer/a11y.observe", json!({"action":"observe"}))
        } else {
            let slot = (index - 2) / 2;
            let value = result(body, IDS[index - 1])?;
            let (generation, reference) = observe_target(&f.config, &value, CONTROLS[slot])?;
            f.observations.lock().unwrap().push((generation, reference));
            if slot == 4 {
                f.witnesses.lock().unwrap().push(
                    witness(&f.config)?
                        .ok_or_else(|| anyhow::anyhow!("readonly baseline witness missing"))?,
                );
            }
            let mut args =
                json!({"action":ACTIONS[slot],"ref":reference,"expected_generation":generation});
            if slot >= 3 {
                args["text"] = json!(if slot == 3 {
                    REPLACEMENT
                } else {
                    "must not replace readonly"
                });
            }
            ("computer/input", args)
        };
        let name = advertised(body, action, &arguments)?;
        f.emitted.lock().unwrap().push(Emitted {
            id: IDS[index].into(),
            name: name.clone(),
            action: action.into(),
            arguments: arguments.clone(),
        });
        Ok((
            json!({"role":"assistant","tool_calls":[{"index":0,"id":IDS[index],"type":"function",
            "function":{"name":name,"arguments":arguments.to_string()}}]}),
            "tool_calls",
        ))
    }

    fn boundaries() -> Value {
        json!({"provider":"localhost scripted SSE; no live model","screenshots":false,
        "native_recorder_evidence":false,"witness_source":"owned NSWindow/control callbacks plus read-only standard text field/control/cell snapshots",
        "value_witness":"stock editable NSTextField without AX override; sampled real model values, not SDK/internal setter call counts",
        "observation_scope":"frontmost application; root CUA attestation and post-observe nonce checks; no hard PID binding",
        "raw_ax_logged":false,"cross_session_cold_token_drift":"covered separately by real Store plus explicit Fake ports",
        "task_ledger_expected":false,"readonly_expected_tool_error_count":1,
        "readonly_control":"instrumented NSTextField denies the setter selector; not an unmodified standard field"})
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

    async fn report_inner(f: &Fixture) -> anyhow::Result<Value> {
        anyhow::ensure!(
            f.failure.lock().unwrap().is_none(),
            "fixture recorded failure; raw content omitted"
        );
        let id = f
            .session_id
            .get()
            .ok_or_else(|| anyhow::anyhow!("setup not ready"))?;
        let path = nomifun_app::AppConfig {
            data_dir: f.config.data_dir.clone(),
            ..Default::default()
        }
        .database_path();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(false)
                    .read_only(true)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await?;
        let store = AgentSessionStore::from_pool(pool.clone()).await?;
        let checked = canonical_report(f, &store, id).await;
        pool.close().await;
        checked
    }

    async fn canonical_report(
        f: &Fixture,
        store: &AgentSessionStore,
        id: &AgentSessionId,
    ) -> anyhow::Result<Value> {
        let page = store
            .read_events(id, None, nomifun_agent_session::MAX_EVENT_PAGE_SIZE)
            .await?;
        let head = store.head(id).await?;
        anyhow::ensure!(
            page.events.len() < nomifun_agent_session::MAX_EVENT_PAGE_SIZE as usize
                || page.next_cursor.seq == head.last_seq,
            "canonical facts exceeded fixture bound"
        );
        if head.active_turn_id.is_some() || page.next_cursor.seq != head.last_seq {
            return Ok(
                json!({"passed":false,"pending":true,"calls":f.calls.load(Ordering::SeqCst),"boundaries":boundaries()}),
            );
        }
        let starts = page
            .events
            .iter()
            .filter(|event| event.kind.0 == "turn/started")
            .collect::<Vec<_>>();
        let [started] = starts.as_slice() else {
            anyhow::bail!("exactly one audit Turn required");
        };
        let turn = nomifun_agent_contracts::OperationId::from(started.correlation_id.as_ref());
        let receipt = store.read_turn_receipt(id, &turn).await?;
        anyhow::ensure!(
            receipt.status == TurnReceiptStatus::Completed
                && head.status == "ready"
                && f.calls.load(Ordering::SeqCst) == 12
                && f.final_emitted.load(Ordering::SeqCst),
            "fixed audit Turn did not complete"
        );
        let terminal = receipt
            .terminal_event
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("terminal missing"))?;
        let SessionEventPayloadRef::InlineJson(payload) = &terminal.payload else {
            anyhow::bail!("terminal must be inline");
        };
        anyhow::ensure!(
            terminal.kind.0 == "turn/completed"
                && payload.0["model_steps"] == 12
                && payload.0["finish_reason"] == "completed",
            "terminal model steps/finish differ"
        );
        let requests = f.emitted.lock().unwrap().clone();
        let results = f.results.lock().unwrap().clone();
        anyhow::ensure!(
            requests.len() == 11 && results.len() == 11,
            "fixed transcript incomplete"
        );
        let calls = page
            .events
            .iter()
            .filter(|event| event.kind.0 == "tool/call-started")
            .collect::<Vec<_>>();
        let returns = page
            .events
            .iter()
            .filter(|event| event.kind.0 == "tool/result-recorded")
            .collect::<Vec<_>>();
        anyhow::ensure!(
            calls.len() == 11 && returns.len() == 11,
            "canonical tool count differs"
        );
        let mut proposals = Vec::new();
        for event in &page.events {
            if event.kind.0 != "runtime/progress-recorded" {
                continue;
            }
            let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
                anyhow::bail!("unexpected unbounded native progress");
            };
            let kind = payload
                .0
                .pointer("/event/event")
                .and_then(Value::as_str)
                .unwrap_or("");
            if matches!(
                kind,
                "tool_call_completed"
                    | "runtime_modules_activated"
                    | "completion_reported"
                    | "completion_delivered"
                    | "completion_candidate_recorded"
            ) {
                let native: AgentEngineEvent = serde_json::from_value(payload.0["event"].clone())?;
                match native {
                    AgentEngineEvent::ToolCallCompleted { call, .. } => proposals.push(call),
                    AgentEngineEvent::RuntimeModulesActivated { modules, .. } => anyhow::ensure!(
                        !modules.contains(&nomifun_agent_runtime::AgentRuntimeModule::TaskLedger),
                        "unexpected task-ledger activation"
                    ),
                    _ => anyhow::bail!("unexpected completion report in Computer-only journey"),
                }
            }
        }
        anyhow::ensure!(proposals.len() == 11, "canonical proposal count differs");
        let mut tool_events = Vec::new();
        for (i, request) in requests.iter().enumerate() {
            let matched=calls.iter().filter(|event|matches!(&event.payload,SessionEventPayloadRef::InlineJson(value) if value.0["call_id"]==request.id)).collect::<Vec<_>>();
            let [event] = matched.as_slice() else {
                anyhow::bail!("exact unique call fact missing");
            };
            let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
                unreachable!()
            };
            anyhow::ensure!(
                event.kind_version == 1
                    && event.producer_id.as_ref() == "runtime_supervisor"
                    && event.causation_event_id.as_ref() == Some(&started.event_id)
                    && payload.0["capability_id"] == "computer"
                    && payload.0["action_id"] == request.action
                    && payload.0["name"] == request.name
                    && payload.0["operation_id"]
                        == format!("{}:tool:{}", turn.as_ref(), request.id),
                "canonical call identity differs"
            );
            anyhow::ensure!(
                proposals
                    .iter()
                    .filter(|call| call.call_id.as_ref() == request.id
                        && call.name == request.name
                        && call.arguments == StrictJsonValue(request.arguments.clone()))
                    .count()
                    == 1,
                "canonical proposal arguments differ"
            );
            let matched = returns
                .iter()
                .filter(|result| result.causation_event_id.as_ref() == Some(&event.event_id))
                .collect::<Vec<_>>();
            let [result] = matched.as_slice() else {
                anyhow::bail!("exact result missing");
            };
            let SessionEventPayloadRef::InlineJson(payload) = &result.payload else {
                anyhow::bail!("bounded result must be inline");
            };
            if i == 10 {
                anyhow::ensure!(
                    payload.0["output"].is_null()
                        && payload.0["error"] == public_unsupported(false),
                    "readonly canonical result is not proven unsupported"
                );
            } else {
                let output: AgentToolResult = serde_json::from_value(payload.0["output"].clone())?;
                anyhow::ensure!(
                    !output.is_error
                        && output.call_id.as_ref() == request.id
                        && payload.0["error"].is_null()
                        && serde_json::from_str::<Value>(&output.output_text())? == results[i],
                    "canonical output differs"
                );
            }
            if matches!(i, 1 | 3 | 5 | 7 | 9) {
                anyhow::ensure!(
                    results[i]["generation"].as_u64() == Some(event.seq),
                    "observe stamp is not exact call seq"
                );
            }
            if matches!(i, 2 | 4 | 6 | 8 | 10) {
                anyhow::ensure!(
                    request.arguments["expected_generation"] == results[i - 1]["generation"],
                    "input did not copy fresh generation"
                );
            }
            tool_events.push(event.event_id.clone());
        }
        let live = store.get_live_session(id).await?;
        let [binding] = live.agent_binding.typed_resource_bindings.as_slice() else {
            anyhow::bail!("sole Computer binding required");
        };
        anyhow::ensure!(
            binding.resource_kind.as_ref() == "computer"
                && binding.resource_id.as_ref() == "local-desktop"
                && binding.operations
                    == std::collections::BTreeSet::from([
                        "observe".to_owned(),
                        "input".to_owned(),
                        "launch".to_owned()
                    ]),
            "binding scope differs"
        );
        let effects = store.list_effects(id).await?;
        anyhow::ensure!(
            effects.len() == 6 && !store.has_unsettled_effects(id).await?,
            "unexpected pending/unknown effect"
        );
        let mut failed = 0;
        for effect in &effects {
            let i = requests
                .iter()
                .position(|request| {
                    effect.operation_id.as_ref() == format!("{}:tool:{}", turn.as_ref(), request.id)
                })
                .ok_or_else(|| anyhow::anyhow!("effect has no fixed call"))?;
            anyhow::ensure!(
                matches!(i, 0 | 2 | 4 | 6 | 8 | 10)
                    && effect.turn_id == turn
                    && effect.resource_binding_id.as_ref() == Some(&binding.binding_id)
                    && effect.capability_module.as_ref() == "computer"
                    && effect.action_id.as_ref() == requests[i].action,
                "effect scope changed"
            );
            anyhow::ensure!(
                effect.state
                    == if i == 10 {
                        AgentEffectState::Rejected
                    } else {
                        AgentEffectState::Returned
                    }
                    && effect.strategy
                        == if i == 0 {
                            EffectStrategy::ManagedEffect
                        } else {
                            EffectStrategy::ExternalUncertainEffect
                        },
                "effect settlement differs"
            );
            let begin = page
                .events
                .iter()
                .find(|event| event.event_id == effect.started_event_id)
                .ok_or_else(|| anyhow::anyhow!("effect start missing"))?;
            anyhow::ensure!(
                begin.causation_event_id.as_ref() == Some(&tool_events[i]),
                "effect cause differs"
            );
            let kind = if i == 10 {
                "effect/failed"
            } else {
                "effect/succeeded"
            };
            anyhow::ensure!(
                page.events.iter().any(|event| Some(&event.event_id)
                    == effect.terminal_event_id.as_ref()
                    && event.kind.0 == kind),
                "effect terminal missing"
            );
            if i == 10 {
                failed += 1;
                anyhow::ensure!(
                    effect.bounded_observation.as_ref().unwrap()["error"]["code"]
                        == "ROLE_HOST_UNSUPPORTED_INPUT",
                    "readonly effect code differs"
                );
                anyhow::ensure!(
                    effect.bounded_observation.as_ref().unwrap()["error"]["message"]
                        == format!("computer input was rejected before native effects: {UNSUPPORTED}"),
                    "readonly owner effect does not retain its exact pre-native rejection provenance"
                );
            } else {
                anyhow::ensure!(
                    effect
                        .bounded_observation
                        .as_ref()
                        .is_some_and(|observation| observation["result"] == results[i]),
                    "returned owner effect differs from the actual tool transcript"
                );
            }
        }
        anyhow::ensure!(failed == 1, "one expected zero-effect refusal required");
        let value = witness(&f.config)?.ok_or_else(|| anyhow::anyhow!("owned witness missing"))?;
        for slot in 0..5 {
            check_control(&f.config, &value, slot)?;
        }
        anyhow::ensure!(
            value == f.witnesses.lock().unwrap().last().unwrap().clone(),
            "owned witness changed after final response"
        );
        Ok(
            json!({"passed":true,"pending":false,"calls":12,"tool_calls":11,"expected_readonly_rejections":1,
            "session_id":id,"turn_id":turn,"tool_started_event_ids":tool_events,"turn_terminal_event_id":terminal.event_id,
            "effects":effects.iter().map(|effect|json!({"effect_id":effect.effect_id,"state":effect.state,"action":effect.action_id,"terminal_event_id":effect.terminal_event_id})).collect::<Vec<_>>(),
            "observations":f.observations.lock().unwrap().iter().map(|(g,r)|json!({"generation":g,"ref":r})).collect::<Vec<_>>(),
            "native_witness":value,"value_input_baseline":f.value_input_baseline.lock().unwrap().clone(),"boundaries":boundaries()}),
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

    pub(super) async fn run() -> anyhow::Result<()> {
        let f = Arc::new(Fixture {
            config: config()?,
            calls: AtomicUsize::new(0),
            emitted: Mutex::new(Vec::new()),
            observations: Mutex::new(Vec::new()),
            results: Mutex::new(Vec::new()),
            witnesses: Mutex::new(Vec::new()),
            value_input_baseline: Mutex::new(None),
            ready_gate: tokio::sync::Semaphore::new(0),
            gate_waiting: std::sync::atomic::AtomicBool::new(false),
            confirmed: std::sync::atomic::AtomicBool::new(false),
            final_emitted: std::sync::atomic::AtomicBool::new(false),
            failure: Mutex::new(None),
            session_id: std::sync::OnceLock::new(),
            stop: CancellationToken::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let router = Router::new().route("/v1/chat/completions",post(model))
            .route("/confirm-owned-frontmost",post(confirm_frontmost))
            .route("/report",get(report))
            .route("/status",get(|State(f):State<Arc<Fixture>>|async move {Json(json!({
                "calls":f.calls.load(Ordering::SeqCst),"waiting_for_owned_frontmost":f.gate_waiting.load(Ordering::SeqCst),
                "confirmed":f.confirmed.load(Ordering::SeqCst),"final_response_emitted":f.final_emitted.load(Ordering::SeqCst),
                "failure":f.failure.lock().unwrap().clone(),"confirm_endpoint":"/confirm-owned-frontmost","boundaries":boundaries()}))}))
            .route("/shutdown",post(|State(f):State<Arc<Fixture>>|async move {f.stop.cancel();"stopped"})).with_state(f.clone());
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
            let provider = api(&app,"create_provider",reqwest::Method::POST,"/api/providers",json!({"platform":"custom","name":"macOS 元素 Computer 验收固定模型",
                "base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,
                "initial_model":{"model":MODEL,"enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
            let provider = provider["provider_id"].as_str().ok_or_else(||anyhow::anyhow!("provider missing"))?;
            let editor = api(&app,"create_preset_from_template",reqwest::Method::POST,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"macOS 元素 Computer 正式入口验收",
                "model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":MODEL}})).await?;
            let preset = editor["preset"]["preset_id"].as_str().ok_or_else(||anyhow::anyhow!("preset missing"))?;
            let mut draft = editor["draft"].clone();
            let capabilities = json!([{"capability":{"id":"computer"},"action_allowlist":["computer/a11y.observe","computer/input","computer/launch"]}]);
            draft["document"]["enabled_capabilities"] = capabilities.clone();
            draft["document"]["skill_bindings"] = json!([]);
            let typed_draft: nomifun_api_types::AgentPresetDraftDto = serde_json::from_value(draft.clone()).map_err(|error|
                anyhow::anyhow!("fixture setup draft schema preflight failed: category={:?}, line={}, column={}; body omitted",error.classify(),error.line(),error.column()))?;
            anyhow::ensure!(typed_draft.document.skill_bindings.is_empty(), "fixture draft must not retain Package Skills");
            let saved = api(&app,"save_preset_revision",reqwest::Method::POST,&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":draft["current_revision"],
                "draft":draft,"reason":"isolated canonical owned-window element input acceptance"})).await?;
            let saved_capabilities: Vec<nomifun_api_types::CapabilitySelectionDto> = serde_json::from_value(saved["revision"]["document"]["enabled_capabilities"].clone())?;
            let expected_capabilities: Vec<nomifun_api_types::CapabilitySelectionDto> = serde_json::from_value(capabilities)?;
            anyhow::ensure!(saved_capabilities == expected_capabilities, "frozen Computer actions changed during setup");
            let session = api(&app,"create_agent_session",reqwest::Method::POST,"/api/agent-sessions",json!({"preset_id":preset,"title":"macOS 自有窗口元素输入验收",
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
            "MACOS_COMPUTER_ELEMENT_GUI_FIXTURE_READY {}",
            json!({"data_dir":f.config.data_dir,"control":format!("http://{address}"),
            "session_id":session,"app_name":f.config.app_name,"marker":f.config.marker,"bundle_identifier":f.config.bundle_id,"app_path":f.config.app_path,
            "prompt":format!("请启动自有应用 {}，仅在root确认该独立窗口前台后读取其AX树，以最新真实ref依次做一次ordinary/right/double元素点击，然后完整替换可写字段，再对只读字段验证执行前拒绝。只读拒绝是预期结果；其它错误立即停。不重试，不截图，不访问其它应用。",f.config.app_name),
            "setup_backend_stopped":true,"expected_model_responses":12,"confirm_endpoint":"/confirm-owned-frontmost","boundaries":boundaries()})
        );
        f.stop.cancelled().await;
        service.await??;
        Ok(())
    }
}
