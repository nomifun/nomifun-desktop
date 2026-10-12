//! Mobile pairing and device management. Browser sessions and native trust
//! authorize management; a device credential can only remove itself.
use axum::{Extension, Json, Router};
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, delete};
use nomifun_api_types::{ApiResponse, ConnectionInfo, ConnectedDevice, DeviceLoginRequest, DevicePairRequest, DevicePairResponse, DevicePairingStatus, DevicePairingTicket, LoginRequest, LoginResponse, PublicUser};
use nomifun_common::AppError;
use crate::routes::AuthRouterState;
use crate::middleware::{AuthState, CurrentDevice, CurrentUser, auth_middleware};
use crate::rate_limit::{RateLimiter, auth_rate_limit_middleware, api_rate_limit_middleware};
use std::sync::Arc;

const PROTOCOL_VERSION: u32 = 1;

pub(crate) fn connection_routes(state: AuthRouterState, pair_limiter: Arc<RateLimiter>) -> Router {
    let api_limiter = Arc::new(RateLimiter::api());
    api_limiter.start_cleanup_task(std::time::Duration::from_secs(60));
    let auth = AuthState { jwt_service: state.jwt_service.clone(), user_repo: state.user_repo.clone(), cookie_config: state.cookie_config.clone() };
    let manage = Router::new()
        .route("/api/connections/pairing", post(create_pairing))
        .route("/api/connections/pairing/{ticket_id}", get(pairing_status))
        .route("/api/connections/devices", get(devices))
        .route("/api/connections/devices/{device_id}", delete(revoke_device))
        .route_layer(from_fn_with_state(state.clone(), require_manager))
        .route_layer(from_fn_with_state(auth.clone(), auth_middleware))
        .with_state(state.clone());
    let authenticated = Router::new()
        .route("/api/connections/device", delete(revoke_self))
        .route_layer(from_fn_with_state(auth, auth_middleware))
        .with_state(state.clone());
    let public = Router::new().route("/api/connections/info", get(info)).with_state(state.clone());
    let pair = Router::new().route("/api/auth/device-pair", post(pair_device))
        .route("/api/auth/device-login", post(device_login))
        .route("/api/auth/device-setup", post(device_setup).route_layer(from_fn_with_state(
            state.auth_policy, crate::trust::require_owner_setup_permission_middleware,
        )))
        .route_layer(from_fn_with_state(pair_limiter, auth_rate_limit_middleware)).with_state(state);
    Router::new().merge(manage).merge(authenticated).merge(public).merge(pair)
        .route_layer(from_fn_with_state(api_limiter, connection_rate_limit_middleware))
}

/// Locally trusted callers (Desktop webview, in-process managers) are not
/// budgeted by the per-IP API limiter: they share the loopback "unknown" key
/// and would starve real clients. `trust_resolve_middleware` runs as the
/// outermost app layer, so the marker is already present at route level.
/// Untrusted remote/browser/device callers keep the per-IP budget unchanged.
async fn connection_rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    if request.extensions().get::<crate::trust::LocalTrusted>().is_some() {
        return Ok(next.run(request).await);
    }
    api_rate_limit_middleware(State(limiter), request, next).await
}

async fn require_manager(State(state): State<AuthRouterState>, request: Request, next: Next) -> Result<Response, AppError> {
    let user = request.extensions().get::<CurrentUser>().ok_or_else(|| AppError::Forbidden("Authentication required".into()))?;
    // Installation API/MCP bearer is not a browser or Mobile credential manager.
    if request.extensions().get::<CurrentDevice>().is_some()
        || request.extensions().get::<crate::trust::InstallationTokenTrusted>().is_some()
    { return Err(AppError::Forbidden("Manage connected devices from Desktop or an owner browser session".into())); }
    let owner = state.user_repo.get_system_user().await?
        .ok_or_else(|| AppError::Internal("Installation owner missing".into()))?;
    if owner.user_id != user.id { return Err(AppError::Forbidden("Installation owner access required".into())); }
    Ok(next.run(request).await)
}

/// The anonymous info endpoint answers every edge; the public relay edge must
/// not leak the computer name. A configured `NOMIFUN_DESKTOP_NAME` is the
/// owner's explicit display choice and therefore wins on every edge.
fn desktop_name(public_relay_edge: bool, env: impl Fn(&str) -> Option<String>) -> String {
    let candidate = |name: &str| env(name).filter(|value| !value.trim().is_empty());
    if let Some(name) = candidate("NOMIFUN_DESKTOP_NAME") { return name; }
    if public_relay_edge { return "NomiFun Desktop".into(); }
    candidate("COMPUTERNAME").or_else(|| candidate("HOSTNAME")).unwrap_or_else(|| "NomiFun Desktop".into())
}

async fn info(State(state): State<AuthRouterState>, edge: Option<Extension<crate::PublicRelayEdge>>) -> Result<Json<ApiResponse<ConnectionInfo>>, AppError> {
    let owner = state.user_repo.get_system_user().await?.ok_or_else(|| AppError::Internal("Installation owner missing".into()))?;
    let name = desktop_name(edge.is_some(), |key| std::env::var(key).ok());
    Ok(Json(ApiResponse::ok(ConnectionInfo {
        desktop_id: owner.user_id.into_string(), desktop_name: name,
        protocol_version: PROTOCOL_VERSION, min_client_protocol: PROTOCOL_VERSION,
        capabilities: ["agent_session", "realtime", "device_pairing", "device_revocation"].into_iter().map(str::to_owned).collect(),
    })))
}

async fn create_pairing(State(state): State<AuthRouterState>) -> Json<ApiResponse<DevicePairingTicket>> {
    Json(ApiResponse::ok(state.qr_token_store.generate_device_ticket()))
}

async fn pairing_status(State(state): State<AuthRouterState>, Path(id): Path<String>) -> Result<Json<ApiResponse<DevicePairingStatus>>, AppError> {
    state.qr_token_store.device_ticket_status(&id).map(|status| Json(ApiResponse::ok(status)))
        .ok_or_else(|| AppError::NotFound("Pairing code no longer available; create a new code".into()))
}

async fn devices(State(state): State<AuthRouterState>, Extension(user): Extension<CurrentUser>) -> Result<Json<ApiResponse<Vec<ConnectedDevice>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.jwt_service.devices.list(user.id.as_str())?)))
}

async fn revoke_device(State(state): State<AuthRouterState>, Extension(user): Extension<CurrentUser>, Path(id): Path<String>) -> Result<Json<ApiResponse<ConnectedDevice>>, AppError> {
    let device = state.jwt_service.devices.revoke(&id, user.id.as_str()).await?;
    state.jwt_service.notify_invalidated();
    Ok(Json(ApiResponse::ok(device)))
}

async fn revoke_self(State(state): State<AuthRouterState>, Extension(user): Extension<CurrentUser>, device: Option<Extension<CurrentDevice>>) -> Result<Json<ApiResponse<ConnectedDevice>>, AppError> {
    let Some(Extension(CurrentDevice(id))) = device else { return Err(AppError::BadRequest("This session is not an individually paired device".into())); };
    let device = state.jwt_service.devices.revoke(&id, user.id.as_str()).await?;
    state.jwt_service.notify_invalidated();
    Ok(Json(ApiResponse::ok(device)))
}

/// Marks a claimed ticket `failed` on drop unless the redemption is explicitly
/// completed or failed first. Axum drops the handler future when the client
/// disconnects mid-issuance, so an armed ticket can never report `used`
/// without a credential.
struct TicketRedemption<'a> {
    store: &'a crate::QrTokenStore,
    qr_token: &'a str,
    armed: bool,
}

impl<'a> TicketRedemption<'a> {
    fn armed(store: &'a crate::QrTokenStore, qr_token: &'a str) -> Self {
        Self { store, qr_token, armed: true }
    }

    fn complete(mut self, device_id: &str) {
        self.store.complete_device_ticket(self.qr_token, device_id);
        self.armed = false;
    }

    fn fail(mut self, code: &'static str) {
        self.store.fail_device_ticket(self.qr_token, code);
        self.armed = false;
    }
}

impl<'a> Drop for TicketRedemption<'a> {
    fn drop(&mut self) {
        if self.armed { self.store.fail_device_ticket(self.qr_token, "DEVICE_PAIRING_FAILED"); }
    }
}

async fn pair_device(State(state): State<AuthRouterState>, Json(req): Json<DevicePairRequest>) -> Response {
    if let Err(response) = validate_device_request(req.protocol_version, &req.device_name) { return response; }
    if let Err(error) = state.qr_token_store.consume_device_ticket(&req.qr_token) { return error.into_response(); }
    let redemption = TicketRedemption::armed(&state.qr_token_store, &req.qr_token);
    match async {
        let user = state.user_repo.get_primary_webui_user().await?.ok_or_else(|| AppError::Internal("Installation owner missing".into()))?;
        Ok::<_, AppError>(issue_device_credential(&state, user, &req.device_name).await?)
    }.await {
        Ok(response) => {
            redemption.complete(&response.device.device_id);
            Json(response).into_response()
        }
        Err(error) => {
            // In this call chain only DeviceStore::register's device cap
            // produces Conflict, so it always means the limit was reached.
            let code = if matches!(error, AppError::Conflict(_)) { "DEVICE_LIMIT_REACHED" } else { "DEVICE_PAIRING_FAILED" };
            redemption.fail(code);
            error.into_response()
        }
    }
}

async fn device_login(State(state): State<AuthRouterState>, Json(req): Json<DeviceLoginRequest>) -> Response {
    if let Err(response) = validate_device_request(req.protocol_version, &req.device_name) { return response; }
    async {
        let login = LoginRequest { username: req.username, password: req.password };
        let user = crate::routes::authenticate_password(&state, &login).await?;
        Ok::<_, AppError>(Json(issue_device_credential(&state, user, &req.device_name).await?))
    }.await.map(IntoResponse::into_response).unwrap_or_else(IntoResponse::into_response)
}

async fn device_setup(State(state): State<AuthRouterState>, Json(req): Json<DeviceLoginRequest>) -> Response {
    if let Err(response) = validate_device_request(req.protocol_version, &req.device_name) { return response; }
    async {
        let user = crate::routes::initialize_owner(&state, Ok(LoginRequest { username: req.username, password: req.password })).await?;
        Ok::<_, AppError>(Json(issue_device_credential(&state, user, &req.device_name).await?))
    }.await.map(IntoResponse::into_response).unwrap_or_else(IntoResponse::into_response)
}

async fn issue_device_credential(state: &AuthRouterState, user: nomifun_db::models::User, name: &str) -> Result<DevicePairResponse, AppError> {
    let device = state.jwt_service.devices.register(user.user_id.as_str(), name).await?;
    let token = state.jwt_service.sign_for_device(user.user_id.as_str(), &user.username, &device.device_id)?;
    if let Err(error) = state.user_repo.update_last_login(user.user_id.as_str()).await {
        tracing::debug!(%error, "device login activity update deferred");
    }
    state.jwt_service.verify(&token)?;
    Ok(DevicePairResponse { login: LoginResponse::new(PublicUser { user_id: user.user_id, username: user.username }, token), device })
}

fn validate_device_request(protocol: u32, name: &str) -> Result<(), Response> {
    if protocol != PROTOCOL_VERSION {
        return Err((StatusCode::UPGRADE_REQUIRED, Json(serde_json::json!({"success":false,"code":"CONNECTION_PROTOCOL_INCOMPATIBLE","error":"Update Mobile and Desktop to compatible versions before pairing.","details":{"protocol_version":PROTOCOL_VERSION,"min_client_protocol":PROTOCOL_VERSION}}))).into_response());
    }
    validate_name(name).map_err(IntoResponse::into_response)
}

fn validate_name(name: &str) -> Result<(), AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(AppError::BadRequest("Device name must contain 1 to 80 visible characters".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::desktop_name;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| vars.iter().find(|(name, _)| *name == key).map(|(_, value)| value.to_string())
    }

    #[test]
    fn configured_name_wins_on_every_edge() {
        let vars = env(&[("NOMIFUN_DESKTOP_NAME", "Studio Box"), ("COMPUTERNAME", "DESKTOP-1")]);
        assert_eq!(desktop_name(true, &vars), "Studio Box");
        assert_eq!(desktop_name(false, &vars), "Studio Box");
        // A blank configured name is not a name at all.
        let vars = env(&[("NOMIFUN_DESKTOP_NAME", "   "), ("HOSTNAME", "host-2")]);
        assert_eq!(desktop_name(true, &vars), "NomiFun Desktop");
        assert_eq!(desktop_name(false, &vars), "host-2");
    }

    #[test]
    fn relay_edge_never_publishes_the_machine_name() {
        let vars = env(&[("COMPUTERNAME", "DESKTOP-1"), ("HOSTNAME", "host-2")]);
        assert_eq!(desktop_name(true, &vars), "NomiFun Desktop");
    }

    #[test]
    fn local_edge_falls_through_blank_candidates() {
        assert_eq!(desktop_name(false, env(&[("COMPUTERNAME", "DESKTOP-1"), ("HOSTNAME", "host-2")])), "DESKTOP-1");
        assert_eq!(desktop_name(false, env(&[("COMPUTERNAME", ""), ("HOSTNAME", "host-2")])), "host-2");
        assert_eq!(desktop_name(false, env(&[("COMPUTERNAME", ""), ("HOSTNAME", " ")])), "NomiFun Desktop");
    }
}
