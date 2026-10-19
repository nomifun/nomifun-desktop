//! Opt-in real HTTP fixture. Product routes and responses are unmodified.
//! The original input-preparation owner is held by a test guard; there is no
//! model response, mocked IPC, real user dataset, or native-device action.
use super::*;
use axum::{
    http::HeaderMap,
    routing::{get, post},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct Input {
    session: AgentSessionId,
    event: EventId,
}
struct Command {
    action: String,
    finished: tokio::sync::oneshot::Sender<()>,
}
#[derive(Clone)]
struct Control {
    nonce: Arc<str>,
    state: NomiCoreAgentApiState,
    inputs: [Input; 2],
    model_requests: Arc<AtomicUsize>,
    commands: tokio::sync::mpsc::Sender<Command>,
}

fn authorized(headers: &HeaderMap, control: &Control) -> Result<(), StatusCode> {
    if headers
        .get("x-nomifun-activity-fixture")
        .and_then(|value| value.to_str().ok())
        != Some(control.nonce.as_ref())
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
async fn input_status(control: &Control, input: &Input) -> Result<Value, StatusCode> {
    let store = control.state.session_owner.canonical.store();
    let delivery = store
        .session_input_delivery(&input.session, &input.event)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let claim = store
        .session_input_claim(&input.session, &input.event)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let preparation = control
        .state
        .session_owner
        .input_preparation_observations(&input.session)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let current = preparation.iter().find(|(event, _)| event == &input.event);
    Ok(
        json!({"cancelled":delivery["state"]=="cancelled","claimed":claim.is_some(),"preparation_in_flight":current.is_some(),"signal_requested":current.is_some_and(|(_,signal)|*signal)}),
    )
}
async fn status(
    State(control): State<Control>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    authorized(&headers, &control)?;
    let turns: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_turns WHERE session_id IN (?,?)",
    )
    .bind(control.inputs[0].session.as_ref())
    .bind(control.inputs[1].session.as_ref())
    .fetch_one(&control.state.session_owner.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        json!({"alpha":input_status(&control,&control.inputs[0]).await?,"beta":input_status(&control,&control.inputs[1]).await?,
        "turns":turns,"model_requests":control.model_requests.load(Ordering::SeqCst)}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandBody {
    action: String,
}
async fn command(
    State(control): State<Control>,
    headers: HeaderMap,
    Json(body): Json<CommandBody>,
) -> Result<Json<Value>, StatusCode> {
    authorized(&headers, &control)?;
    if !matches!(
        body.action.as_str(),
        "release_alpha" | "release_beta" | "finish"
    ) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (done, finished) = tokio::sync::oneshot::channel();
    control
        .commands
        .send(Command {
            action: body.action,
            finished: done,
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    finished
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({"accepted":true})))
}

pub(super) async fn run() {
    let supplied = std::env::var_os("NOMIFUN_ACTIVITY_BROWSER_FIXTURE_DIR")
        .expect("explicit browser fixture root required");
    let supplied = std::path::PathBuf::from(supplied);
    assert!(supplied.is_absolute(), "fixture root must be absolute");
    let root = std::fs::canonicalize(&supplied).unwrap();
    let temporary = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    assert!(
        root.starts_with(&temporary) && root != temporary,
        "only a new temporary fixture root is permitted"
    );
    assert!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("nomifun-activity-browser-")
    );
    let marker: Value = serde_json::from_slice(
        &std::fs::read(root.join(".activity-browser-fixture.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(marker["kind"], "nomifun.agent-activity-browser.v1");
    let nonce = marker["nonce"].as_str().unwrap().to_owned();
    assert!(
        uuid::Uuid::parse_str(&nonce).is_ok(),
        "random fixture sentinel required"
    );
    assert!(
        !root.join("data").exists() && !root.join("work").exists(),
        "fixture data must never be reused"
    );

    let model_requests = Arc::new(AtomicUsize::new(0));
    let counter = model_requests.clone();
    // A transport tripwire, never a scripted LLM: any accidental model HTTP
    // request fails the fixture instead of returning an assistant response.
    let tripwire = axum::Router::new().fallback(move || {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "model calls are forbidden in this fixture",
            )
        }
    });
    let model_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_url = format!("http://{}/v1", model_listener.local_addr().unwrap());
    let model_stop = tokio_util::sync::CancellationToken::new();
    let model_end = model_stop.clone();
    let model_server = tokio::spawn(async move {
        axum::serve(model_listener, tripwire)
            .with_graceful_shutdown(model_end.cancelled_owned())
            .await
            .unwrap();
    });
    let (services, router, state, alpha, principal) = fixture_in(&root, &provider_url).await;
    let binding = state
        .session_owner
        .canonical
        .store()
        .get_live_session(&alpha)
        .await
        .unwrap()
        .agent_binding;
    let second = request(
        &router,
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":binding.preset_revision_ref.preset_id,"title":"Lifecycle input Beta"}),
    )
    .await;
    let beta = AgentSessionId::from(second["agent_session_id"].as_str().unwrap());
    request(
        &router,
        "PUT",
        "/api/settings/client",
        json!({"language":"en-US"}),
    )
    .await;
    let first=state.session_owner.canonical.accept_session_input(&principal,&alpha,"browser-alpha",json!({"content":"synthetic Alpha pending input","files":[],"inject_skills":[],"hidden":false})).await.unwrap().record.unwrap().event_id;
    let second=state.session_owner.canonical.accept_session_input(&principal,&beta,"browser-beta",json!({"content":"synthetic Beta pending input","files":[],"inject_skills":[],"hidden":false})).await.unwrap().record.unwrap().event_id;
    let mut alpha_guard = Some(
        state
            .session_owner
            .hold_input_preparation_for_test(&alpha, &first)
            .unwrap(),
    );
    let mut beta_guard = Some(
        state
            .session_owner
            .hold_input_preparation_for_test(&beta, &second)
            .unwrap(),
    );
    let (commands, mut incoming) = tokio::sync::mpsc::channel(4);
    let control = Control {
        nonce: nonce.clone().into(),
        state: state.clone(),
        inputs: [
            Input {
                session: alpha.clone(),
                event: first.clone(),
            },
            Input {
                session: beta.clone(),
                event: second.clone(),
            },
        ],
        model_requests,
        commands,
    };
    let private = axum::Router::new()
        .route("/__agent-activities-fixture/status", get(status))
        .route("/__agent-activities-fixture/command", post(command))
        .with_state(control.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let stop = tokio_util::sync::CancellationToken::new();
    let end = stop.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router.merge(private))
            .with_graceful_shutdown(end.cancelled_owned())
            .await
            .unwrap();
    });
    std::fs::write(root.join("ready.json"),serde_json::to_vec_pretty(&json!({"kind":"nomifun.agent-activity-browser.v1","nonce":nonce,"port":port,"pid":std::process::id(),
        "alpha":{"session":alpha,"input":first},"beta":{"session":beta,"input":second}})).unwrap()).unwrap();
    println!(
        "NOMIFUN_ACTIVITY_BROWSER_READY {}",
        root.join("ready.json").display()
    );
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(240);
    let mut finished = false;
    while let Ok(Some(command)) = tokio::time::timeout_at(deadline, incoming.recv()).await {
        match command.action.as_str() {
            "release_alpha" => {
                drop(alpha_guard.take());
            }
            "release_beta" => {
                drop(beta_guard.take());
            }
            "finish" => {
                finished = true;
            }
            _ => unreachable!(),
        }
        let _ = command.finished.send(());
        if finished {
            break;
        }
    }
    drop(alpha_guard.take());
    drop(beta_guard.take());
    let proof = status(State(control.clone()), {
        let mut headers = HeaderMap::new();
        headers.insert("x-nomifun-activity-fixture", nonce.parse().unwrap());
        headers
    })
    .await
    .unwrap()
    .0;
    let passed = finished
        && proof["alpha"]["cancelled"] == true
        && proof["beta"]["cancelled"] == true
        && proof["alpha"]["claimed"] == false
        && proof["beta"]["claimed"] == false
        && proof["turns"] == 0
        && proof["model_requests"] == 0;
    std::fs::write(
        root.join("backend-proof.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":passed,"finished":finished,"observation":proof}),
        )
        .unwrap(),
    )
    .unwrap();
    services.shutdown_nomi_core_host().await.unwrap();
    stop.cancel();
    if tokio::time::timeout(std::time::Duration::from_secs(10), &mut server)
        .await
        .is_err()
    {
        server.abort();
        let _ = server.await;
    }
    model_stop.cancel();
    model_server.await.unwrap();
    services.database.close().await;
    assert!(
        passed,
        "browser fixture must cancel the original two inputs without a Turn or model request"
    );
}
