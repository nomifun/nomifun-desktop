//! Installation-owned authentication. Credentials rotate independently of
//! immutable Agent/connection contracts; no OAuth data enters Session history.
use std::{collections::HashMap, sync::{Arc, Weak}, time::{Duration, Instant}};

use axum::{extract::{RawQuery, State}, response::{Html, IntoResponse, Response}, routing::get, Router};
use nomifun_api_types::{
    McpAuthErrorCode as Code, McpAuthStage as Stage, McpAuthState, McpAuthStatus,
    McpOAuthAttempt, McpOAuthAttemptStatus as AttemptStatus, McpOAuthClientConfig, McpOAuthStartRequest,
};
use nomifun_common::{decrypt_string, encrypt_string, generate_id, now_ms};
use nomifun_db::{IMcpOAuthRepository, IMcpServerRepository, McpOAuthClientParams,
    McpOAuthGrantParams, McpOAuthGrantRefreshParams, McpOAuthGrantRow};
use oauth2::{CsrfToken, PkceCodeChallenge};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::{Mutex, OwnedMutexGuard}, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tokio_util::task::{TaskTracker, task_tracker::TaskTrackerToken};
use zeroize::{Zeroize, Zeroizing};

use crate::{canonical_mcp_connection_config_ref, oauth_failure::McpAuthFailure,
    oauth_protocol::{self, ClientOptions, ClientRegistration, DiscoveryContext, TokenSet},
    owner::{McpCredential, McpCredentialAuthority, McpCredentialLease, McpCredentialLookup, McpOwnerError},
    types::McpServerTransport};

const MAX_ATTEMPTS: usize = 64;
const MAX_CALLBACK_BYTES: usize = 16 * 1024;
const CALLBACK_PATH: &str = "/api/mcp/oauth/callback";

#[derive(Clone)]
pub struct McpAuthOptions {
    /// An operator-configured, externally reachable HTTPS callback. Never
    /// derived from Host, Origin, or an arbitrary browser request.
    pub callback_url: Option<String>,
    /// A published, product-owned CIMD URL, when the installation has one.
    pub cimd_client_id: Option<String>,
    pub attempt_timeout: Duration,
    /// Explicit injected test/local-provider policy; production defaults false.
    pub allow_http_loopback: bool,
}

impl Default for McpAuthOptions {
    fn default() -> Self {
        Self { callback_url: None, cimd_client_id: None,
            attempt_timeout: Duration::from_secs(180), allow_http_loopback: false }
    }
}

#[derive(Clone)]
pub struct McpAuthService { inner: Arc<Inner> }

struct Inner {
    repo: Arc<dyn IMcpOAuthRepository>,
    servers: Arc<dyn IMcpServerRepository>,
    owner: Arc<str>,
    key: [u8; 32],
    client: Arc<dyn Fn() -> Result<reqwest::Client, McpAuthFailure> + Send + Sync>,
    options: McpAuthOptions,
    attempts: Mutex<HashMap<String, Arc<Attempt>>>,
    gates: Mutex<HashMap<String, Weak<Mutex<()>>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    shutdown: CancellationToken,
    /// In-memory admission generation, including anonymous→grant→anonymous.
    /// This is not a persistent Session/permission/probe ledger.
    generations: Mutex<HashMap<String, u64>>,
    failures: Mutex<HashMap<String, nomifun_api_types::McpAuthDiagnostic>>,
    required: Mutex<HashMap<String, String>>,
    activity: TaskTracker,
}

struct Attempt {
    id: String,
    server_id: String,
    config_ref: String,
    state: String,
    deadline: Instant,
    cancel: CancellationToken,
    view: Mutex<McpOAuthAttempt>,
    secrets: Mutex<Option<AttemptSecrets>>,
}

struct AttemptSecrets {
    discovery: DiscoveryContext,
    registration: ClientRegistration,
    client_ref: String,
    verifier: String,
}

impl Drop for AttemptSecrets {
    fn drop(&mut self) {
        self.verifier.zeroize();
        if let Some(secret) = &mut self.registration.client_secret { secret.zeroize(); }
    }
}

#[derive(Serialize, Deserialize)]
struct TokenEnvelope {
    access_token: String,
    refresh_token: Option<String>,
    /// Exact validated authorization context. Refresh never discovers a new
    /// issuer or selects a different client from a mutable server URL.
    discovery: DiscoveryContext,
}

impl Drop for TokenEnvelope {
    fn drop(&mut self) {
        // Avoid exposing secret material through Debug/serialization outside
        // the explicit encrypted envelope boundary.
        self.access_token.zeroize();
        if let Some(value) = &mut self.refresh_token { value.zeroize(); }
    }
}

fn failure(code: Code, stage: Stage, message: &str) -> McpAuthFailure {
    McpAuthFailure::new(code, stage, message)
}
fn storage_failure() -> McpAuthFailure {
    failure(Code::SecretStorageFailed, Stage::Storage, "MCP credential storage is unavailable")
}
fn changed() -> McpAuthFailure {
    failure(Code::ConnectionChanged, Stage::Connection, "The MCP connection or authorization changed; retry the operation")
}

impl McpAuthService {
    pub fn new(
        repo: Arc<dyn IMcpOAuthRepository>, servers: Arc<dyn IMcpServerRepository>,
        owner: Arc<str>, key: [u8; 32], client: reqwest::Client, options: McpAuthOptions,
    ) -> Self {
        Self::with_factory(repo, servers, owner, key, Arc::new(move || Ok(client.clone())), options)
    }

    pub fn new_dynamic(
        repo: Arc<dyn IMcpOAuthRepository>, servers: Arc<dyn IMcpServerRepository>,
        owner: Arc<str>, key: [u8; 32], options: McpAuthOptions,
    ) -> Self {
        Self::with_factory(repo, servers, owner, key, Arc::new(oauth_protocol::http_client), options)
    }

    fn with_factory(
        repo: Arc<dyn IMcpOAuthRepository>, servers: Arc<dyn IMcpServerRepository>,
        owner: Arc<str>, key: [u8; 32],
        client: Arc<dyn Fn() -> Result<reqwest::Client, McpAuthFailure> + Send + Sync>,
        options: McpAuthOptions,
    ) -> Self {
        Self { inner: Arc::new(Inner { repo, servers, owner, key, client, options,
            attempts: Mutex::new(HashMap::new()), gates: Mutex::new(HashMap::new()),
            tasks: Mutex::new(Vec::new()), shutdown: CancellationToken::new(),
            generations: Mutex::new(HashMap::new()), failures: Mutex::new(HashMap::new()),
            required: Mutex::new(HashMap::new()), activity: TaskTracker::new() }) }
    }

    fn admission(&self) -> Result<TaskTrackerToken, McpAuthFailure> {
        if self.inner.shutdown.is_cancelled() { return Err(changed()); }
        let token = self.inner.activity.token();
        // Covers token acquisition racing close/wait during host teardown.
        if self.inner.shutdown.is_cancelled() { return Err(changed()); }
        Ok(token)
    }

    pub(crate) async fn gate(&self, id: &str) -> Arc<Mutex<()>> {
        let mut gates = self.inner.gates.lock().await;
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(id).and_then(Weak::upgrade) { return gate; }
        let gate = Arc::new(Mutex::new(()));
        gates.insert(id.to_owned(), Arc::downgrade(&gate));
        gate
    }

    async fn server(&self, id: &str) -> Result<(McpServerTransport, String), McpAuthFailure> {
        let _activity = self.admission()?;
        let row = self.inner.servers.find_by_id(id).await.map_err(|_| storage_failure())?
            .filter(|row| row.deleted_at.is_none()).ok_or_else(changed)?;
        let transport = McpServerTransport::from_db(&row.transport_type, &row.transport_config)
            .map_err(|_| failure(Code::InvalidConfiguration, Stage::Connection, "Invalid saved MCP connection"))?;
        let id = nomifun_api_types::McpServerId::parse(row.mcp_server_id).map_err(|_| changed())?;
        let config_ref = canonical_mcp_connection_config_ref(&id, &transport)
            .map_err(|_| changed())?;
        Ok((transport, config_ref))
    }

    pub async fn binding_revision(&self, id: &str) -> Result<String, McpAuthFailure> {
        let _activity = self.admission()?;
        let generation = self.inner.generations.lock().await.get(id).copied().unwrap_or_default();
        let grant = self.inner.repo.get_grant(&self.inner.owner, id).await.map_err(|_| storage_failure())?;
        Ok(format!("{generation}:{}", grant.map(|grant| grant.grant_id).unwrap_or_default()))
    }

    /// A settings/catalog observation gets no invocation authority. Its final
    /// publication shares the same revoke/rebind linearization boundary.
    pub async fn acquire_observation_guard(&self, id: &str, expected_revision: &str) -> Result<OwnedMutexGuard<()>, McpAuthFailure> {
        let guard = self.gate(id).await.lock_owned().await;
        if self.binding_revision(id).await? != expected_revision { return Err(changed()); }
        Ok(guard)
    }

    async fn advance_generation(&self, id: &str) {
        let mut generations = self.inner.generations.lock().await;
        let generation = generations.entry(id.to_owned()).or_default();
        *generation = generation.saturating_add(1);
    }

    pub async fn observe_probe(&self, id: &str, needs_oauth: bool, success: bool) {
        let Ok(_activity) = self.admission() else { return; };
        if needs_oauth {
            if let Ok((_, reference)) = self.server(id).await {
                self.inner.required.lock().await.insert(id.to_owned(), reference);
            }
        } else if success {
            self.inner.required.lock().await.remove(id);
            let mut failures = self.inner.failures.lock().await;
            if failures.get(id).is_none_or(|value| value.code != Code::InsufficientScope) { failures.remove(id); }
        }
    }

    pub async fn status(&self, id: &str) -> Result<McpAuthStatus, McpAuthFailure> {
        let _activity = self.admission()?;
        let (transport, config_ref) = self.server(id).await?;
        let mut state = McpAuthState::Anonymous;
        if !matches!(transport, McpServerTransport::Stdio { .. })
            && self.inner.required.lock().await.get(id) == Some(&config_ref) {
            state = McpAuthState::Required;
        }
        let grant = self.inner.repo.get_grant(&self.inner.owner, id).await.map_err(|_| storage_failure())?;
        let mut expires_at = None;
        let mut scopes = Vec::new();
        if let Some(grant) = grant.filter(|grant| grant.connection_config_ref == config_ref) {
            expires_at = grant.expires_at;
            scopes = serde_json::from_str(&grant.scopes_json).map_err(|_| storage_failure())?;
            let envelope = self.decrypt_tokens(&grant)?;
            state = if grant.expires_at.is_none_or(|expires| expires > now_ms()) || envelope.refresh_token.is_some() {
                McpAuthState::Authorized
            } else { McpAuthState::ReauthorizationRequired };
        }
        let mut diagnostic = {
            let attempts = self.inner.attempts.lock().await;
            let latest = attempts.values().filter(|attempt| attempt.server_id == id && attempt.config_ref == config_ref).max_by_key(|attempt| &attempt.id).cloned();
            drop(attempts);
            if let Some(attempt) = latest { attempt.view.lock().await.diagnostic.clone() } else { None }
        };
        if let Some(observed) = self.inner.failures.lock().await.get(id).cloned() { diagnostic = Some(observed); }
        if (state != McpAuthState::Authorized || expires_at.is_some_and(|expiry| expiry <= now_ms())
            || diagnostic.as_ref().is_some_and(|value| value.code == Code::InsufficientScope))
            && let Some(ref diagnostic) = diagnostic {
            state = match diagnostic.code {
                Code::ClientRegistrationRequired | Code::ClientRegistrationRejected | Code::ClientAuthenticationUnsupported | Code::RedirectUnavailable => McpAuthState::ConfigurationRequired,
                Code::ReauthorizationRequired | Code::InsufficientScope => McpAuthState::ReauthorizationRequired,
                Code::Cancelled | Code::UserDenied | Code::CallbackTimeout => state,
                _ => McpAuthState::Failed,
            };
        }
        Ok(McpAuthStatus { state, expires_at, granted_scopes: scopes, diagnostic })
    }

    pub async fn start(
        &self, server_id: &str, input: McpOAuthStartRequest, native_callback: bool,
    ) -> Result<McpOAuthAttempt, McpAuthFailure> {
        let _activity = self.admission()?;
        if self.inner.shutdown.is_cancelled() { return Err(changed()); }
        let row = self.inner.servers.find_by_id(server_id).await.map_err(|_| storage_failure())?.ok_or_else(changed)?;
        if row.builtin {
            return Err(failure(Code::InvalidConfiguration, Stage::Authorization,
                "OAuth grants belong to installed MCP connections; application-owned sources retain their application's authentication policy"));
        }
        let (transport, config_ref) = self.server(server_id).await?;
        let is_legacy = matches!(transport, McpServerTransport::Sse { .. });
        let (endpoint, headers, legacy) = match transport {
            McpServerTransport::Http { url, headers } | McpServerTransport::Sse { url, headers } => {
                if headers.keys().any(|key| credential_header(key)) {
                    return Err(failure(Code::InvalidConfiguration, Stage::Authorization, "This connection uses configured credentials; remove them before selecting OAuth"));
                }
                (url, headers, is_legacy)
            }
            _ => return Err(failure(Code::InvalidConfiguration, Stage::Authorization, "OAuth requires an HTTP MCP connection")),
        };
        let attempt = Arc::new(Attempt {
            id: generate_id(), server_id: server_id.to_owned(), config_ref,
            state: CsrfToken::new_random().secret().clone(),
            deadline: Instant::now() + self.inner.options.attempt_timeout,
            cancel: self.inner.shutdown.child_token(),
            view: Mutex::new(McpOAuthAttempt { attempt_id: String::new(), status: AttemptStatus::WaitingForUser,
                authorization_url: None, diagnostic: None }), secrets: Mutex::new(None),
        });
        {
            let gate = self.gate(server_id).await;
            let _guard = gate.lock().await;
            let mut attempts = self.inner.attempts.lock().await;
            attempts.retain(|_, value| value.deadline > Instant::now());
            if attempts.len() >= MAX_ATTEMPTS {
                return Err(failure(Code::InvalidConfiguration, Stage::Authorization, "Too many pending MCP authorization attempts"));
            }
            for previous in attempts.values().filter(|value| value.server_id == server_id) {
                previous.cancel.cancel();
            }
            attempts.insert(attempt.id.clone(), attempt.clone());
            self.inner.required.lock().await.insert(server_id.to_owned(), attempt.config_ref.clone());
        }
        attempt.view.lock().await.attempt_id = attempt.id.clone();
        let prepared = tokio::select! {
            _ = attempt.cancel.cancelled() => Err(failure(Code::Cancelled, Stage::Authorization, "MCP authorization was cancelled")),
            result = tokio::time::timeout(Duration::from_secs(45), self.prepare(&attempt, &endpoint, &headers, legacy, input.client.unwrap_or_default(), native_callback)) =>
                result.unwrap_or_else(|_| Err(failure(Code::UpstreamUnavailable, Stage::Discovery, "MCP authorization preparation timed out"))),
        };
        match prepared {
            Ok(listener) => {
                let service = self.clone();
                let current = attempt.clone();
                let task = tokio::spawn(async move {
                    let cancel = current.cancel.clone();
                    if let Some(listener) = listener {
                        let router = Router::new().route(CALLBACK_PATH, get(local_callback))
                            .with_state((service.clone(), current.id.clone()));
                        let deadline = current.deadline;
                        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
                            tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::time::sleep_until(deadline.into()) => {} }
                        });
                        let _ = server.await;
                    } else {
                        tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::time::sleep_until(current.deadline.into()) => {} }
                    }
                    if Instant::now() >= current.deadline && !current.cancel.is_cancelled() {
                        service.finish_failure(&current, failure(Code::CallbackTimeout, Stage::Callback, "MCP authorization timed out"), AttemptStatus::Expired).await;
                    }
                });
                let mut tasks = self.inner.tasks.lock().await;
                tasks.retain(|task| !task.is_finished());
                tasks.push(task);
            }
            Err(error) => self.finish_failure(&attempt, error, AttemptStatus::Failed).await,
        }
        Ok(attempt.view.lock().await.clone())
    }

    async fn prepare(
        &self, attempt: &Arc<Attempt>, endpoint: &str, headers: &HashMap<String, String>,
        legacy: bool, config: McpOAuthClientConfig, native: bool,
    ) -> Result<Option<TcpListener>, McpAuthFailure> {
        let explicit_redirect = match &config {
            McpOAuthClientConfig::PreRegistered { redirect_uri, .. } => redirect_uri.clone(),
            _ => None,
        };
        // Reject unavailable WebUI callback configuration before any remote IO.
        let web_target = if !native { Some(self.callback_target(false, explicit_redirect.as_deref()).await?) } else { None };
        let client = (self.inner.client)()?;
        let challenge = crate::owner::discover_network(client.clone(), endpoint, headers, legacy,
            &crate::McpSetupPolicy::uniform(Duration::from_secs(5)), None).await
            .err().and_then(|error| error.authentication_challenge().map(str::to_owned));
        let mut discovery = oauth_protocol::discover(&client, endpoint, challenge.as_deref(), self.inner.options.allow_http_loopback).await?;
        if let Some(diagnostic) = self.inner.failures.lock().await.get(&attempt.server_id)
            && diagnostic.code == Code::InsufficientScope
            && let Some(scopes) = diagnostic.details.as_ref().and_then(|value| value.get("required_scopes")).and_then(serde_json::Value::as_array) {
            for scope in scopes.iter().filter_map(serde_json::Value::as_str) {
                if !discovery.scopes.iter().any(|value| value == scope) { discovery.scopes.push(scope.to_owned()); }
            }
        }
        if let Some(grant) = self.inner.repo.get_grant(&self.inner.owner, &attempt.server_id).await.map_err(|_| storage_failure())? {
            if grant.issuer != discovery.issuer {
                return Err(failure(Code::IssuerMismatch, Stage::Discovery, "The connection's authorization server changed; explicitly disconnect before configuring the new identity"));
            }
            if grant.connection_config_ref != attempt.config_ref { return Err(changed()); }
            // A subsequent scope upgrade retains the same connection's prior
            // permissions rather than silently requesting only the latest 403.
            let granted: Vec<String> = serde_json::from_str(&grant.scopes_json).map_err(|_| storage_failure())?;
            for scope in granted {
                if !discovery.scopes.contains(&scope) { discovery.scopes.push(scope); }
            }
        }
        let profile_key = format!("{}:{}", attempt.server_id, if native { "loopback" } else { "web" });
        let cached = if matches!(config, McpOAuthClientConfig::Auto {}) {
            self.inner.repo.get_client(&self.inner.owner, &discovery.issuer, &profile_key).await.map_err(|_| storage_failure())?
                .filter(|row| row.client_secret_expires_at.is_none_or(|expiry| expiry > now_ms()))
        } else { None };
        let registered_redirect = cached.as_ref().map(|row| serde_json::from_str::<Vec<String>>(&row.redirect_uris_json))
            .transpose().map_err(|_| storage_failure())?.and_then(|values| values.into_iter().next());
        // A pre-registered native client retains its exact registered port.
        // Only dynamic native registrations use RFC 8252 port variation.
        let native_redirect = explicit_redirect.as_deref().or_else(|| cached.as_ref()
            .filter(|row| row.registration_kind != "dynamic").and(registered_redirect.as_deref()));
        let (redirect_uri, listener) = match web_target {
            Some(value) => value,
            None => self.callback_target(true, native_redirect).await?,
        };
        let (registration, client_ref) = if let Some(row) = cached {
            let registered = registered_redirect.as_deref().ok_or_else(storage_failure)?;
            if (!native || row.registration_kind != "dynamic") && registered != redirect_uri {
                return Err(failure(Code::RedirectUnavailable, Stage::ClientRegistration, "The callback changed; configure a client registered for the new callback URL"));
            }
            (ClientRegistration { issuer: row.issuer, client_id: row.client_id,
                client_secret: row.client_secret_encrypted.map(|value| decrypt_string(&value, &self.inner.key)).transpose().map_err(|_| storage_failure())?,
                client_secret_expires_at: row.client_secret_expires_at,
                token_endpoint_auth_method: row.token_endpoint_auth_method, redirect_uri: redirect_uri.clone() }, row.client_ref)
        } else {
            let options = match config {
                McpOAuthClientConfig::Auto {} => ClientOptions { pre_registered: None, cimd_client_id: self.inner.options.cimd_client_id.clone() },
                McpOAuthClientConfig::Cimd { client_id } => ClientOptions { pre_registered: None, cimd_client_id: Some(client_id) },
                McpOAuthClientConfig::PreRegistered { client_id, client_secret, token_endpoint_auth_method, .. } => ClientOptions {
                    pre_registered: Some(ClientRegistration { issuer: discovery.issuer.clone(), client_id,
                        token_endpoint_auth_method: token_endpoint_auth_method.unwrap_or_else(|| if client_secret.is_some() { "client_secret_basic" } else { "none" }.to_owned()),
                        client_secret, client_secret_expires_at: None, redirect_uri: redirect_uri.clone() }), cimd_client_id: None,
                },
            };
            let registration = oauth_protocol::register_client(&client, &discovery, &options, &redirect_uri, self.inner.options.allow_http_loopback).await?;
            let gate = self.gate(&attempt.server_id).await;
            let _guard = gate.lock().await;
            self.ensure_attempt(attempt).await?;
            let row = self.inner.repo.upsert_client(McpOAuthClientParams {
                client_ref: generate_id(), owner_id: self.inner.owner.to_string(), issuer: discovery.issuer.clone(),
                profile_key, registration_kind: if options.pre_registered.is_some() { "static" } else if options.cimd_client_id.is_some() && discovery.cimd_supported { "cimd" } else { "dynamic" }.into(),
                client_id: registration.client_id.clone(), token_endpoint_auth_method: registration.token_endpoint_auth_method.clone(),
                redirect_uris_json: serde_json::to_string(&vec![redirect_uri.clone()]).map_err(|_| storage_failure())?,
                client_secret_encrypted: registration.client_secret.as_ref().map(|value| encrypt_string(value, &self.inner.key)).transpose().map_err(|_| storage_failure())?,
                client_secret_expires_at: registration.client_secret_expires_at,
            }).await.map_err(|_| storage_failure())?;
            (registration, row.client_ref)
        };
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let url = oauth_protocol::authorization_url(&discovery, &registration, &attempt.state, challenge.as_str(), &discovery.scopes)?;
        let gate = self.gate(&attempt.server_id).await;
        let _guard = gate.lock().await;
        self.ensure_attempt(attempt).await?;
        *attempt.secrets.lock().await = Some(AttemptSecrets { discovery, registration, client_ref, verifier: verifier.secret().clone() });
        attempt.view.lock().await.authorization_url = Some(url);
        Ok(listener)
    }

    async fn callback_target(&self, native: bool, explicit: Option<&str>) -> Result<(String, Option<TcpListener>), McpAuthFailure> {
        if native {
            let port = if let Some(raw) = explicit {
                let url = reqwest::Url::parse(raw).map_err(|_| failure(Code::RedirectUnavailable, Stage::Authorization, "Invalid registered callback URL"))?;
                if raw.trim() != raw || raw.chars().any(char::is_control) || !url.username().is_empty() || url.password().is_some()
                    || url.scheme() != "http" || url.host_str() != Some("127.0.0.1") || url.path() != CALLBACK_PATH || url.query().is_some() || url.fragment().is_some() {
                    return Err(failure(Code::RedirectUnavailable, Stage::Authorization, "Native callback must use the registered 127.0.0.1 callback path"));
                }
                url.port().ok_or_else(|| failure(Code::RedirectUnavailable, Stage::Authorization, "Registered callback requires an explicit port"))?
            } else { 0 };
            let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(|_| failure(Code::RedirectUnavailable, Stage::Authorization, "Cannot bind the registered OAuth callback"))?;
            let address = listener.local_addr().map_err(|_| failure(Code::RedirectUnavailable, Stage::Authorization, "Cannot determine the OAuth callback address"))?;
            return Ok((format!("http://127.0.0.1:{}{CALLBACK_PATH}", address.port()), Some(listener)));
        }
        let raw = self.inner.options.callback_url.as_deref().ok_or_else(|| failure(
            Code::RedirectUnavailable, Stage::Authorization, "Configure an externally reachable HTTPS MCP OAuth callback URL for WebUI login"))?;
        let url = reqwest::Url::parse(raw).map_err(|_| failure(Code::RedirectUnavailable, Stage::Authorization, "Invalid configured OAuth callback URL"))?;
        if raw.trim() != raw || raw.chars().any(char::is_control) || url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.path() != CALLBACK_PATH || url.query().is_some() || url.fragment().is_some() || explicit.is_some_and(|value| value != raw) {
            return Err(failure(Code::RedirectUnavailable, Stage::Authorization, "The WebUI callback must match the configured HTTPS callback"));
        }
        Ok((raw.to_owned(), None))
    }

    async fn ensure_attempt(&self, attempt: &Attempt) -> Result<(), McpAuthFailure> {
        if attempt.cancel.is_cancelled() || Instant::now() >= attempt.deadline { return Err(changed()); }
        let (_, config_ref) = self.server(&attempt.server_id).await?;
        if config_ref != attempt.config_ref { return Err(changed()); }
        let attempts = self.inner.attempts.lock().await;
        if !attempts.contains_key(&attempt.id) { return Err(changed()); }
        Ok(())
    }

    pub async fn attempt(&self, server_id: &str, id: &str) -> Result<McpOAuthAttempt, McpAuthFailure> {
        let _activity = self.admission()?;
        self.server(server_id).await?;
        let attempt = self.inner.attempts.lock().await.get(id).filter(|value| value.server_id == server_id).cloned().ok_or_else(changed)?;
        if attempt.cancel.is_cancelled() {
            let mut view = attempt.view.lock().await;
            if matches!(view.status, AttemptStatus::WaitingForUser | AttemptStatus::Exchanging) {
                view.status = AttemptStatus::Cancelled; view.authorization_url = None;
            }
        }
        Ok(attempt.view.lock().await.clone())
    }

    pub async fn cancel_attempt(&self, server_id: &str, id: &str) -> Result<(), McpAuthFailure> {
        let _activity = self.admission()?;
        let gate = self.gate(server_id).await;
        let _guard = gate.lock().await;
        let attempt = self.inner.attempts.lock().await.get(id).filter(|value| value.server_id == server_id).cloned().ok_or_else(changed)?;
        if attempt.view.lock().await.status == AttemptStatus::Authorized {
            return Err(failure(Code::ConnectionChanged, Stage::Authorization,
                "Authorization already completed; disconnect the connection to remove its grant"));
        }
        self.finish_failure(&attempt, failure(Code::Cancelled, Stage::Authorization, "MCP authorization was cancelled"), AttemptStatus::Cancelled).await;
        Ok(())
    }

    async fn finish_failure(&self, attempt: &Attempt, error: McpAuthFailure, status: AttemptStatus) {
        let mut secrets = attempt.secrets.lock().await;
        let mut view = attempt.view.lock().await;
        if matches!(view.status, AttemptStatus::Authorized | AttemptStatus::Failed | AttemptStatus::Cancelled | AttemptStatus::Expired) { return; }
        view.status = status; view.authorization_url = None; view.diagnostic = Some(error.diagnostic);
        secrets.take();
        attempt.cancel.cancel();
    }

    pub async fn complete_callback(&self, attempt_id: Option<&str>, query: &str) -> Result<(), McpAuthFailure> {
        let _activity = self.admission()?;
        if query.len() > MAX_CALLBACK_BYTES { return Err(failure(Code::CallbackInvalid, Stage::Callback, "OAuth callback is oversized")); }
        let url = reqwest::Url::parse(&format!("http://callback.invalid/?{query}")).map_err(|_| failure(Code::CallbackInvalid, Stage::Callback, "Invalid OAuth callback"))?;
        let mut params = HashMap::new();
        for (key, value) in url.query_pairs() {
            if params.insert(key.into_owned(), value.into_owned()).is_some() {
                return Err(failure(Code::CallbackInvalid, Stage::Callback, "Duplicate OAuth callback parameters"));
            }
        }
        let state = params.get("state").ok_or_else(|| failure(Code::StateMismatch, Stage::Callback, "Missing OAuth callback state"))?;
        let attempt = {
            let attempts = self.inner.attempts.lock().await;
            attempts.values().find(|value| value.state == *state && attempt_id.is_none_or(|id| value.id == id)).cloned()
                .ok_or_else(|| failure(Code::StateMismatch, Stage::Callback, "OAuth callback does not match a pending authorization"))?
        };
        let secrets = {
            let gate = self.gate(&attempt.server_id).await;
            let _guard = gate.lock().await;
            self.ensure_attempt(&attempt).await?;
            if params.contains_key("code") && params.contains_key("error") { return Err(failure(Code::CallbackInvalid, Stage::Callback, "Ambiguous OAuth callback")); }
            let mut pending = attempt.secrets.lock().await;
            let secret = pending.as_ref().ok_or_else(changed)?;
            if let Some(issuer) = params.get("iss") {
                if issuer != &secret.discovery.issuer { return Err(failure(Code::IssuerMismatch, Stage::Callback, "OAuth callback issuer does not match the selected authorization server")); }
            } else if secret.discovery.authorization_response_iss_supported {
                return Err(failure(Code::IssuerMismatch, Stage::Callback, "OAuth callback is missing its required issuer"));
            }
            if params.contains_key("error") {
                drop(pending);
                let error = match params.get("error").map(String::as_str) {
                    Some("access_denied") => failure(Code::UserDenied, Stage::Callback, "The authorization server did not grant access"),
                    Some("temporarily_unavailable" | "server_error") => failure(Code::UpstreamUnavailable, Stage::Callback, "The authorization server is temporarily unavailable"),
                    Some("invalid_scope") => failure(Code::InsufficientScope, Stage::Callback, "The requested MCP authorization scope was rejected"),
                    _ => failure(Code::CallbackInvalid, Stage::Callback, "The authorization server rejected the callback"),
                };
                self.finish_failure(&attempt, error.clone(), AttemptStatus::Failed).await;
                return Err(error);
            }
            if params.get("code").is_none_or(|value| value.is_empty()) { return Err(failure(Code::CallbackInvalid, Stage::Callback, "OAuth callback is missing its authorization code")); }
            attempt.view.lock().await.status = AttemptStatus::Exchanging;
            pending.take().ok_or_else(changed)?
        };
        let result: Result<(), McpAuthFailure> = async {
            let client = (self.inner.client)()?;
            let exchange = tokio::time::timeout(Duration::from_secs(30), oauth_protocol::exchange_code(
                &client, &secrets.discovery, &secrets.registration, &params["code"], &secrets.verifier, self.inner.options.allow_http_loopback,
            ));
            let tokens = tokio::select! {
                _ = attempt.cancel.cancelled() => return Err(failure(Code::Cancelled, Stage::Callback, "MCP authorization was cancelled")),
                result = exchange => result.map_err(|_| failure(Code::UpstreamUnavailable, Stage::TokenExchange, "OAuth token exchange timed out"))??,
            };
            let gate = self.gate(&attempt.server_id).await;
            let _guard = gate.lock().await;
            self.ensure_attempt(&attempt).await?;
            let params = self.grant_params(&attempt, &secrets, tokens)?;
            self.inner.repo.replace_grant(params).await.map_err(|_| storage_failure())?;
            self.advance_generation(&attempt.server_id).await;
            self.inner.failures.lock().await.remove(&attempt.server_id);
            let mut view = attempt.view.lock().await;
            view.status = AttemptStatus::Authorized; view.authorization_url = None; view.diagnostic = None;
            attempt.cancel.cancel();
            Ok(())
        }.await;
        if let Err(ref error) = result { self.finish_failure(&attempt, error.clone(), AttemptStatus::Failed).await; }
        result
    }

    fn grant_params(&self, attempt: &Attempt, secret: &AttemptSecrets, tokens: TokenSet) -> Result<McpOAuthGrantParams, McpAuthFailure> {
        let obtained_at = now_ms();
        let expires_at = token_expiry(obtained_at, tokens.expires_in)?;
        let scopes = if tokens.scopes.is_empty() { secret.discovery.scopes.clone() } else { tokens.scopes.clone() };
        let envelope = TokenEnvelope { access_token: tokens.access_token, refresh_token: tokens.refresh_token, discovery: secret.discovery.clone() };
        let token_encrypted = self.encrypt_tokens(&envelope)?;
        Ok(McpOAuthGrantParams { grant_id: generate_id(), owner_id: self.inner.owner.to_string(), server_id: attempt.server_id.clone(),
            connection_config_ref: attempt.config_ref.clone(), issuer: secret.discovery.issuer.clone(), resource: secret.discovery.resource.clone(),
            client_ref: secret.client_ref.clone(), scopes_json: serde_json::to_string(&scopes).map_err(|_| storage_failure())?,
            token_encrypted, expires_at, obtained_at })
    }

    fn encrypt_tokens(&self, envelope: &TokenEnvelope) -> Result<String, McpAuthFailure> {
        let plaintext = Zeroizing::new(serde_json::to_string(envelope).map_err(|_| storage_failure())?);
        encrypt_string(&plaintext, &self.inner.key).map_err(|_| storage_failure())
    }

    fn decrypt_tokens(&self, grant: &McpOAuthGrantRow) -> Result<TokenEnvelope, McpAuthFailure> {
        let value = Zeroizing::new(decrypt_string(&grant.token_encrypted, &self.inner.key).map_err(|_| storage_failure())?);
        let envelope: TokenEnvelope = serde_json::from_str(&value).map_err(|_| storage_failure())?;
        if envelope.discovery.issuer != grant.issuer || envelope.discovery.resource != grant.resource { return Err(storage_failure()); }
        Ok(envelope)
    }

    pub async fn logout(&self, server_id: &str) -> Result<(), McpAuthFailure> {
        let _activity = self.admission()?;
        let gate = self.gate(server_id).await;
        let _guard = gate.lock().await;
        let (_, reference) = self.server(server_id).await?;
        let previously_required = self.inner.required.lock().await.get(server_id) == Some(&reference);
        let known_oauth = previously_required
            || self.inner.repo.get_grant(&self.inner.owner, server_id).await.map_err(|_| storage_failure())?.is_some();
        self.invalidate_locked(server_id).await?;
        // Disconnect credentials, not the already established authentication
        // capability. The ordinary Login action remains available afterwards.
        if known_oauth { self.inner.required.lock().await.insert(server_id.to_owned(), reference); }
        Ok(())
    }

    pub(crate) async fn invalidate_locked(&self, server_id: &str) -> Result<(), McpAuthFailure> {
        let _activity = self.admission()?;
        self.advance_generation(server_id).await;
        self.inner.failures.lock().await.remove(server_id);
        self.inner.required.lock().await.remove(server_id);
        let attempts: Vec<_> = self.inner.attempts.lock().await.values().filter(|value| value.server_id == server_id).cloned().collect();
        for attempt in attempts {
            self.finish_failure(&attempt, failure(Code::Cancelled, Stage::Authorization, "MCP authorization was invalidated"), AttemptStatus::Cancelled).await;
            // A terminal failure belongs to the previous authorization. It
            // must not become the current connection's diagnostic after an
            // explicit disconnect or reconfiguration.
            let terminal_failure = matches!(attempt.view.lock().await.status, AttemptStatus::Failed | AttemptStatus::Expired);
            if terminal_failure {
                self.inner.attempts.lock().await.remove(&attempt.id);
            }
        }
        self.inner.repo.delete_grant(&self.inner.owner, server_id).await.map_err(|_| storage_failure())
    }

    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        self.inner.activity.close();
        self.inner.activity.wait().await;
        let tasks = std::mem::take(&mut *self.inner.tasks.lock().await);
        for task in tasks {
            let _ = task.await;
        }
        self.inner.attempts.lock().await.clear();
    }

    async fn credential(&self, lookup: &McpCredentialLookup) -> Result<Option<McpCredential>, McpAuthFailure> {
        self.credential_bound(lookup, None).await
    }

    async fn credential_bound(&self, lookup: &McpCredentialLookup, expected_grant: Option<&str>) -> Result<Option<McpCredential>, McpAuthFailure> {
        let _activity = self.admission()?;
        let (transport, config_ref) = self.server(&lookup.server_id).await?;
        if lookup.connection_config_ref != config_ref || lookup.resource_id != lookup.server_id { return Err(changed()); }
        let (url, configured) = match transport {
            McpServerTransport::Http { url, headers } | McpServerTransport::Sse { url, headers } => (url, headers),
            _ => return Ok(None),
        };
        if url != lookup.endpoint { return Err(changed()); }
        let static_credentials: HashMap<_, _> = configured.into_iter().filter(|(key, _)| credential_header(key)).collect();
        if !static_credentials.is_empty() {
            let mut credential = McpCredential::configured(&static_credentials)
                .map_err(|_| failure(Code::InvalidConfiguration, Stage::Connection, "Invalid configured MCP credentials"))?;
            credential.set_lease(Arc::new(GrantLease { service: self.clone(), lookup: lookup.clone(),
                grant_id: None, revision: None, expires_at: None }));
            return Ok(Some(credential));
        }
        let gate = self.gate(&lookup.server_id).await;
        // A single gate owns refresh and grant replacement. No database
        // transaction spans network IO; other connections remain independent.
        let _guard = gate.lock().await;
        let Some(mut grant) = self.inner.repo.get_grant(&self.inner.owner, &lookup.server_id).await.map_err(|_| storage_failure())? else {
            return if expected_grant.is_some() { Err(changed()) } else { Ok(None) };
        };
        if expected_grant.is_some_and(|id| id != grant.grant_id) { return Err(changed()); }
        if grant.connection_config_ref != lookup.connection_config_ref { return Err(changed()); }
        let mut envelope = self.decrypt_tokens(&grant)?;
        let margin = grant.expires_at.map(|expires| ((expires - grant.obtained_at) / 10).clamp(0, 60_000)).unwrap_or(0);
        if grant.expires_at.is_some_and(|expires| expires <= now_ms() + margin) {
            if let Some(refresh) = envelope.refresh_token.as_deref() {
                let row = self.inner.repo.get_client_by_ref(&self.inner.owner, &grant.client_ref).await.map_err(|_| storage_failure())?.ok_or_else(storage_failure)?;
                let registration = ClientRegistration { issuer: row.issuer, client_id: row.client_id,
                    client_secret: row.client_secret_encrypted.map(|value| decrypt_string(&value, &self.inner.key)).transpose().map_err(|_| storage_failure())?,
                    token_endpoint_auth_method: row.token_endpoint_auth_method,
                    client_secret_expires_at: row.client_secret_expires_at,
                    redirect_uri: serde_json::from_str::<Vec<String>>(&row.redirect_uris_json).map_err(|_| storage_failure())?.into_iter().next().ok_or_else(storage_failure)? };
                let http = (self.inner.client)()?;
                let refresh_request = tokio::time::timeout(Duration::from_secs(30), oauth_protocol::refresh_token(
                    &http, &envelope.discovery, &registration, refresh, self.inner.options.allow_http_loopback,
                ));
                let refreshed = tokio::select! {
                    _ = self.inner.shutdown.cancelled() => return Err(changed()),
                    result = refresh_request => result.unwrap_or_else(|_| Err(failure(Code::UpstreamUnavailable, Stage::Refresh, "MCP credential refresh timed out"))),
                };
                match refreshed {
                    Ok(tokens) => {
                        let obtained_at = now_ms();
                        let expires_at = token_expiry(obtained_at, tokens.expires_in)?;
                        let scopes = if tokens.scopes.is_empty() { grant.scopes_json.clone() } else { serde_json::to_string(&tokens.scopes).map_err(|_| storage_failure())? };
                        envelope.access_token.zeroize();
                        envelope.access_token = tokens.access_token;
                        if tokens.refresh_token.is_some() {
                            if let Some(value) = &mut envelope.refresh_token { value.zeroize(); }
                            envelope.refresh_token = tokens.refresh_token;
                        }
                        grant = self.inner.repo.compare_and_swap_grant(McpOAuthGrantRefreshParams {
                            owner_id: self.inner.owner.to_string(), grant_id: grant.grant_id.clone(), expected_revision: grant.revision,
                            scopes_json: scopes, token_encrypted: self.encrypt_tokens(&envelope)?, expires_at, obtained_at,
                        }).await.map_err(|_| storage_failure())?.ok_or_else(changed)?;
                    }
                    Err(error) if error.diagnostic.retryable && grant.expires_at.is_some_and(|expires| expires > now_ms()) => {}
                    Err(error) => {
                        self.inner.failures.lock().await.insert(lookup.server_id.clone(), error.diagnostic.clone());
                        if error.diagnostic.code == Code::ReauthorizationRequired {
                            self.advance_generation(&lookup.server_id).await;
                            self.inner.repo.delete_grant(&self.inner.owner, &lookup.server_id).await.map_err(|_| storage_failure())?;
                        }
                        return Err(error);
                    }
                }
            }
        }
        if grant.expires_at.is_some_and(|expires| expires <= now_ms()) {
            return Err(failure(Code::ReauthorizationRequired, Stage::Refresh, "MCP authorization expired; sign in again"));
        }
        let mut credential = McpCredential::bearer(envelope.access_token.clone()).map_err(|_| storage_failure())?;
        let mut failures = self.inner.failures.lock().await;
        if failures.get(&lookup.server_id).is_none_or(|value| value.code != Code::InsufficientScope) {
            failures.remove(&lookup.server_id);
        }
        drop(failures);
        credential.set_lease(Arc::new(GrantLease { service: self.clone(), lookup: lookup.clone(),
            grant_id: Some(grant.grant_id), revision: Some(grant.revision), expires_at: grant.expires_at }));
        Ok(Some(credential))
    }
}

struct GrantLease {
    service: McpAuthService, lookup: McpCredentialLookup, grant_id: Option<String>,
    revision: Option<i64>, expires_at: Option<i64>,
}

#[async_trait::async_trait]
impl McpCredentialLease for GrantLease {
    async fn acquire(&self) -> Result<OwnedMutexGuard<()>, McpOwnerError> {
        let _activity = self.service.admission().map_err(owner_error)?;
        let guard = self.service.gate(&self.lookup.server_id).await.lock_owned().await;
        let (_, config_ref) = self.service.server(&self.lookup.server_id).await.map_err(owner_error)?;
        let grant = self.service.inner.repo.get_grant(&self.service.inner.owner, &self.lookup.server_id).await
            .map_err(|_| owner_error(storage_failure()))?;
        let invalid_grant = self.grant_id.as_ref().is_some_and(|id| grant.as_ref().is_none_or(
            |value| &value.grant_id != id || Some(value.revision) != self.revision));
        if config_ref != self.lookup.connection_config_ref || invalid_grant {
            return Err(owner_error(changed()));
        }
        if self.expires_at.is_some_and(|expiry| expiry <= now_ms()) {
            return Err(owner_error(failure(Code::ReauthorizationRequired, Stage::Refresh, "The MCP credential expired before request dispatch")));
        }
        Ok(guard)
    }

    async fn rejected(&self, code: Code, scopes: Vec<String>) -> Result<Option<nomifun_api_types::McpAuthDiagnostic>, McpOwnerError> {
        let _activity = self.service.admission().map_err(owner_error)?;
        let Some(id) = &self.grant_id else { return Ok(None); };
        let gate = self.service.gate(&self.lookup.server_id).await;
        let _guard = gate.lock().await;
        let Some(grant) = self.service.inner.repo.get_grant(&self.service.inner.owner, &self.lookup.server_id).await
            .map_err(|_| owner_error(storage_failure()))? else { return Ok(None); };
        if &grant.grant_id != id || Some(grant.revision) != self.revision { return Ok(None); }
        let error = if code == Code::InsufficientScope {
            failure(code, Stage::Connection, "The MCP operation requires additional authorization scope")
                .with_details(serde_json::json!({"required_scopes": scopes}))
        } else {
            failure(Code::ReauthorizationRequired, Stage::Connection, "The MCP service rejected this authorization")
        };
        if code != Code::InsufficientScope {
        self.service.inner.repo.compare_and_swap_grant(McpOAuthGrantRefreshParams {
            owner_id: self.service.inner.owner.to_string(), grant_id: grant.grant_id,
            expected_revision: grant.revision, scopes_json: grant.scopes_json,
            token_encrypted: grant.token_encrypted, expires_at: Some(now_ms().saturating_sub(1)),
            obtained_at: grant.obtained_at,
        }).await.map_err(|_| owner_error(storage_failure()))?;
        }
        self.service.inner.failures.lock().await.insert(self.lookup.server_id.clone(), error.diagnostic.clone());
        Ok(Some(error.diagnostic))
    }

    async fn renew(&self) -> Result<Option<McpCredential>, McpOwnerError> {
        let Some(id) = &self.grant_id else { return Ok(None); };
        // Never switch an initializing transport to another user's new grant.
        self.service.credential_bound(&self.lookup, Some(id)).await.map_err(owner_error)
    }
}

#[async_trait::async_trait]
impl McpCredentialAuthority for McpAuthService {
    async fn resolve(&self, lookup: McpCredentialLookup) -> Result<Option<McpCredential>, McpOwnerError> {
        self.credential(&lookup).await.map_err(owner_error)
    }
}

fn owner_error(error: McpAuthFailure) -> McpOwnerError {
    let code = match error.diagnostic.code {
        Code::ReauthorizationRequired => "MCP_CREDENTIAL_EXPIRED",
        Code::ConnectionChanged => "MCP_CONNECTION_CONFIG_CHANGED",
        Code::InsufficientScope => "MCP_INSUFFICIENT_SCOPE",
        _ => "MCP_CREDENTIAL_AUTHORITY_FAILED",
    };
    McpOwnerError::new(code, error.diagnostic.message.clone()).with_auth_diagnostic(error.diagnostic)
}

pub(crate) fn credential_header(key: &str) -> bool {
    matches!(key.to_ascii_lowercase().as_str(), "authorization" | "cookie" | "x-api-key" | "api-key" | "x-auth-token")
}

fn token_expiry(obtained_at: i64, expires_in: Option<u64>) -> Result<Option<i64>, McpAuthFailure> {
    expires_in.map(|seconds| i64::try_from(seconds).ok().and_then(|value| value.checked_mul(1000))
        .and_then(|value| obtained_at.checked_add(value)).ok_or_else(|| failure(
            Code::TokenExchangeRejected, Stage::TokenExchange, "Invalid OAuth token expiration"))).transpose()
}

async fn local_callback(
    State((service, attempt_id)): State<(McpAuthService, String)>, RawQuery(query): RawQuery,
) -> Response {
    callback_response(service.complete_callback(Some(&attempt_id), query.as_deref().unwrap_or_default()).await)
}

pub fn callback_response(result: Result<(), McpAuthFailure>) -> Response {
    let mut response = match result {
        Ok(()) => Html("<!doctype html><title>NomiFun authorization</title><p>Authorization completed. Return to NomiFun to check the connection.</p>").into_response(),
        Err(error) => error.into_response(),
    };
    response.headers_mut().insert(axum::http::header::CACHE_CONTROL, axum::http::HeaderValue::from_static("no-store"));
    response.headers_mut().insert(axum::http::header::REFERRER_POLICY, axum::http::HeaderValue::from_static("no-referrer"));
    response
}
