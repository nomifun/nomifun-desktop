//! Real HTTP OAuth and MCP fixtures. No browser, external provider, or user token.
use std::{collections::{BTreeMap, BTreeSet, HashMap}, sync::{Arc, Mutex}, time::Duration};
use axum::{Router, body::{Body, to_bytes}, extract::{Request, State}, http::{HeaderValue, StatusCode}, response::Response};
use base64::{Engine, engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}};
use nomifun_api_types::{McpAuthErrorCode, McpAuthState, McpOAuthAttempt, McpOAuthAttemptStatus, McpOAuthStartRequest, McpServerId};
use nomifun_db::{CreateMcpServerParams, IMcpOAuthRepository, IMcpServerRepository, SqliteMcpOAuthRepository, SqliteMcpServerRepository};
use nomifun_mcp::{McpAuthOptions, McpAuthService, McpConnectionTestService, McpServerTransport};
use oauth2::url::{Url, form_urlencoded};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

const KEY: [u8; 32] = [71; 32];
const HTTP_BUDGET: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct ExpectedCode { resource: String, client_id: String, redirect_uri: String, challenge: String }
#[derive(Clone)]
struct IssuedToken { resource: String, access: String, refresh: String, generation: usize }
#[derive(Clone)]
struct ExchangeBlock { entered: Arc<Notify>, release: Arc<Notify> }
#[derive(Default)]
struct ProviderState {
    base: String,
    initial_ttl: u64,
    clients: HashMap<String, String>,
    codes: HashMap<String, ExpectedCode>,
    tokens: HashMap<String, IssuedToken>,
    registrations: usize,
    exchanges: usize,
    refreshes: usize,
    mcp_authorizations: Vec<Option<String>>,
    mcp_methods: Vec<String>,
    reject_tool_call: bool,
    reject_tool_scope: Option<String>,
    metadata_queries: Vec<String>,
    requests: Vec<String>,
    block: Option<ExchangeBlock>,
}

fn response(status: StatusCode, value: Value) -> Response {
    let mut response = Response::new(Body::from(value.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert("content-type", HeaderValue::from_static("application/json"));
    response
}
fn parse_form(body: &[u8]) -> BTreeMap<String, String> {
    form_urlencoded::parse(body).map(|(key, value)| (key.into_owned(), value.into_owned())).collect()
}

async fn provider(State(state): State<Arc<Mutex<ProviderState>>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 256 * 1024).await.unwrap();
    let path = parts.uri.path();
    {
        let mut data = state.lock().unwrap();
        data.requests.push(path.to_owned());
        if let Some(slug) = path.strip_prefix("/metadata/") {
            data.metadata_queries.push(parts.uri.query().unwrap_or_default().to_owned());
            if parts.uri.query() != Some("tenant=fixture-tenant&revision=17") {
                return response(StatusCode::NOT_FOUND,json!({}));
            }
            return response(StatusCode::OK, json!({"resource":format!("{}/mcp/{slug}",data.base),
                "authorization_servers":[format!("{}/auth",data.base)],"scopes_supported":["mcp:connect"]}));
        }
        if path == "/.well-known/oauth-authorization-server/auth" {
            return response(StatusCode::OK, json!({"issuer":format!("{}/auth",data.base),
                "authorization_endpoint":format!("{}/authorize",data.base),"token_endpoint":format!("{}/token",data.base),
                "registration_endpoint":format!("{}/register",data.base),"response_types_supported":["code"],
                "grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"],
                "token_endpoint_auth_methods_supported":["client_secret_basic"],"authorization_response_iss_parameter_supported":true}));
        }
        if path == "/register" {
            let registration: Value = serde_json::from_slice(&body).unwrap();
            let redirect=registration["redirect_uris"][0].as_str().unwrap();
            let callback=Url::parse(redirect).unwrap();
            assert_eq!(registration["application_type"],if callback.scheme()=="http" { "native" } else { "web" });
            assert_eq!(registration["token_endpoint_auth_method"], "client_secret_basic");
            data.registrations += 1;
            let client_id = format!("issued-client-{}",data.registrations);
            let secret = format!("instance-secret-{}",data.registrations);
            data.clients.insert(client_id.clone(),secret.clone());
            return response(StatusCode::CREATED, json!({"client_id":client_id,"client_secret":secret,
                "token_endpoint_auth_method":"client_secret_basic","redirect_uris":registration["redirect_uris"]}));
        }
        if let Some(slug) = path.strip_prefix("/mcp/") {
            let rpc: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            data.mcp_methods.push(rpc.get("method").and_then(Value::as_str).unwrap_or_default().to_owned());
            let authorization = parts.headers.get("authorization").and_then(|value| value.to_str().ok()).map(str::to_owned);
            data.mcp_authorizations.push(authorization.clone());
            let resource = format!("{}/mcp/{slug}",data.base);
            let valid = authorization.as_deref().is_some_and(|header| data.tokens.values().any(|token|
                token.resource == resource && header == format!("Bearer {}",token.access)));
            if !valid {
                let mut rejection = response(StatusCode::UNAUTHORIZED,json!({}));
                // Custom non-well-known path proves the login flow carries the
                // actual MCP challenge into protected-resource discovery.
                rejection.headers_mut().append("www-authenticate",HeaderValue::from_static("Basic realm=\"other, realm\""));
                rejection.headers_mut().append("www-authenticate",HeaderValue::from_str(&format!(
                    "Bearer resource_metadata=\"{}/metadata/{slug}?tenant=fixture-tenant&revision=17\", scope=\"mcp:connect\"",data.base)).unwrap());
                return rejection;
            }
            return match rpc.get("method").and_then(Value::as_str) {
                Some("initialize") => response(StatusCode::OK,json!({"jsonrpc":"2.0","id":rpc["id"],"result":{
                    "protocolVersion":rpc["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"oauth-fixture","version":"1"}}})),
                Some("notifications/initialized") => {
                    let mut empty=Response::new(Body::empty());*empty.status_mut()=StatusCode::ACCEPTED;empty
                }
                Some("tools/list") => response(StatusCode::OK,json!({"jsonrpc":"2.0","id":rpc["id"],"result":{"tools":[{
                    "name":"fixture_read","description":"Read fixture data","inputSchema":{"type":"object","properties":{}}}]}})),
                Some("tools/call") if data.reject_tool_scope.is_some() => {
                    let mut rejection=response(StatusCode::FORBIDDEN,json!({}));
                    rejection.headers_mut().insert("www-authenticate",HeaderValue::from_str(&format!(
                        "Bearer error=\"insufficient_scope\", scope=\"{}\"",data.reject_tool_scope.as_deref().unwrap())).unwrap());
                    rejection
                }
                Some("tools/call") if data.reject_tool_call => {
                    let mut rejection=response(StatusCode::UNAUTHORIZED,json!({}));
                    rejection.headers_mut().insert("www-authenticate",HeaderValue::from_static("Bearer error=\"invalid_token\""));
                    rejection
                }
                Some("tools/call") => response(StatusCode::OK,json!({"jsonrpc":"2.0","id":rpc["id"],"result":{"content":[{"type":"text","text":"fixture-result"}]}})),
                _ => response(StatusCode::BAD_REQUEST,json!({})),
            };
        }
    }
    if path == "/token" {
        let block = state.lock().unwrap().block.clone();
        if let Some(block) = block { block.entered.notify_one();block.release.notified().await; }
        let form = parse_form(&body);
        let basic = parts.headers.get("authorization").and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Basic ")).and_then(|value| STANDARD.decode(value).ok())
            .and_then(|value| String::from_utf8(value).ok()).unwrap_or_default();
        let (client_id,secret) = basic.split_once(':').unwrap_or(("",""));
        let mut data = state.lock().unwrap();
        if data.clients.get(client_id).map(String::as_str) != Some(secret) {
            return response(StatusCode::UNAUTHORIZED,json!({"error":"invalid_client","error_description":"must-not-leak-client-secret"}));
        }
        let Some(resource) = form.get("resource") else { return response(StatusCode::BAD_REQUEST,json!({"error":"invalid_target"})); };
        let (generation,ttl) = if form.get("grant_type").map(String::as_str) == Some("authorization_code") {
            data.exchanges += 1;
            let Some(expected) = form.get("code").and_then(|code|data.codes.remove(code)) else {
                return response(StatusCode::BAD_REQUEST,json!({"error":"invalid_grant"}));
            };
            assert_eq!(&expected.resource,resource);
            assert_eq!(expected.client_id,client_id);
            assert_eq!(form.get("redirect_uri"),Some(&expected.redirect_uri));
            let verifier = form.get("code_verifier").expect("PKCE verifier");
            assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),expected.challenge);
            (1,data.initial_ttl)
        } else {
            data.refreshes += 1;
            let Some(previous) = data.tokens.get(client_id) else { return response(StatusCode::BAD_REQUEST,json!({"error":"invalid_grant"})); };
            if form.get("refresh_token") != Some(&previous.refresh) || resource != &previous.resource {
                return response(StatusCode::BAD_REQUEST,json!({"error":"invalid_grant","error_description":"must-not-leak-refresh-secret"}));
            }
            (previous.generation+1,3600)
        };
        let token=IssuedToken { resource:resource.clone(),access:format!("access-{client_id}-{generation}"),
            refresh:format!("refresh-{client_id}-{generation}"),generation };
        data.tokens.insert(client_id.into(),token.clone());
        return response(StatusCode::OK,json!({"access_token":token.access,"refresh_token":token.refresh,
            "token_type":"Bearer","expires_in":ttl,"scope":"mcp:connect"}));
    }
    response(StatusCode::NOT_FOUND,json!({}))
}

struct Fixture {
    _database: nomifun_db::Database,
    state: Arc<Mutex<ProviderState>>,
    provider_task: tokio::task::JoinHandle<()>,
    repository: Arc<SqliteMcpOAuthRepository>,
    servers: Arc<SqliteMcpServerRepository>,
    service: McpAuthService,
    owner: Arc<str>,
    client: reqwest::Client,
    base: String,
}
impl Drop for Fixture { fn drop(&mut self) { self.provider_task.abort(); } }

impl Fixture {
    async fn new(initial_ttl: u64) -> Self {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base=format!("http://{}",listener.local_addr().unwrap());
        let state=Arc::new(Mutex::new(ProviderState { base:base.clone(),initial_ttl,..Default::default() }));
        let app=Router::new().fallback(provider).with_state(state.clone());
        let provider_task=tokio::spawn(async move { axum::serve(listener,app).await.unwrap(); });
        let database=nomifun_db::init_database_memory().await.unwrap();
        let owner:Arc<str>=nomifun_db::installation_owner_id(database.pool()).await.unwrap().into();
        let repository=Arc::new(SqliteMcpOAuthRepository::new(database.pool().clone()));
        let servers=Arc::new(SqliteMcpServerRepository::new(database.pool().clone()));
        let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(HTTP_BUDGET).build().unwrap();
        let service=McpAuthService::new(repository.clone(),servers.clone(),owner.clone(),KEY,client.clone(),
            McpAuthOptions { allow_http_loopback:true,attempt_timeout:Duration::from_secs(30),..Default::default() });
        Self { _database:database,state,provider_task,repository,servers,service,owner,client,base }
    }
    async fn server(&self,name:&str,slug:&str)->(McpServerId,McpServerTransport) {
        let url=format!("{}/mcp/{slug}",self.base);
        let config=json!({"url":url}).to_string();
        let row=self.servers.create(CreateMcpServerParams { name,description:None,enabled:true,transport_type:"http",
            transport_config:&config,tools:None,original_json:None,builtin:false }).await.unwrap();
        (McpServerId::parse(row.mcp_server_id).unwrap(),McpServerTransport::Http{url,headers:HashMap::new()})
    }
    async fn start(&self,server:&McpServerId)->McpOAuthAttempt {
        let attempt=self.service.start(server.as_str(),McpOAuthStartRequest::default(),true).await.unwrap();
        assert_eq!(attempt.status,McpOAuthAttemptStatus::WaitingForUser,"{:?}",attempt.diagnostic);
        assert!(attempt.authorization_url.is_some());attempt
    }
    fn callback_query(&self,attempt:&McpOAuthAttempt,code:&str)->String {
        let url=Url::parse(attempt.authorization_url.as_deref().unwrap()).unwrap();
        let params:BTreeMap<_,_>=url.query_pairs().map(|(key,value)|(key.into_owned(),value.into_owned())).collect();
        assert_eq!(params["code_challenge_method"],"S256");
        let expected=ExpectedCode { resource:params["resource"].clone(),client_id:params["client_id"].clone(),
            redirect_uri:params["redirect_uri"].clone(),challenge:params["code_challenge"].clone() };
        self.state.lock().unwrap().codes.insert(code.into(),expected);
        form_urlencoded::Serializer::new(String::new()).append_pair("code",code).append_pair("state",&params["state"])
            .append_pair("iss",&format!("{}/auth",self.base)).finish()
    }
    async fn login(&self,server:&McpServerId,code:&str)->McpOAuthAttempt {
        let attempt=self.start(server).await;let query=self.callback_query(&attempt,code);
        self.service.complete_callback(Some(&attempt.attempt_id),&query).await.unwrap();
        assert_eq!(self.service.attempt(server.as_str(),&attempt.attempt_id).await.unwrap().status,McpOAuthAttemptStatus::Authorized);
        attempt
    }
    fn probe(&self)->McpConnectionTestService {
        McpConnectionTestService::new(self.client.clone()).with_credentials(Arc::new(self.service.clone()))
    }
    async fn shutdown(&self) { tokio::time::timeout(HTTP_BUDGET,self.service.shutdown()).await.expect("OAuth tasks must stop"); }
}

#[tokio::test]
async fn oauth_login_encrypts_secrets_and_saved_connection_reuses_the_exact_grant() {
    let fixture=Fixture::new(3600).await;let (server,transport)=fixture.server("login","one").await;
    assert_eq!(fixture.service.status(server.as_str()).await.unwrap().state,McpAuthState::Anonymous);
    let before=fixture.probe().test_saved_connection(&server,"login",&transport).await;
    assert_eq!(before.needs_auth,Some(true));
    assert!(matches!(before.auth_method,Some(nomifun_api_types::McpAuthMethod::Oauth)));
    fixture.service.observe_probe(server.as_str(),true,false).await;
    assert_eq!(fixture.service.status(server.as_str()).await.unwrap().state,McpAuthState::Required);
    let attempt=fixture.start(&server).await;
    let authorization=Url::parse(attempt.authorization_url.as_deref().unwrap()).unwrap();
    let redirect=authorization.query_pairs().find(|(name,_)|name=="redirect_uri").unwrap().1.into_owned();
    let query=fixture.callback_query(&attempt,"approved-one");
    let callback=fixture.client.get(format!("{redirect}?{query}")).send().await.unwrap();
    assert_eq!(callback.status(),StatusCode::OK);
    assert!(callback.text().await.unwrap().contains("Authorization completed"));
    assert_eq!(fixture.service.attempt(server.as_str(),&attempt.attempt_id).await.unwrap().status,McpOAuthAttemptStatus::Authorized);
    let grant=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    assert!(!grant.token_encrypted.contains("access-issued-client-1-1"));
    let envelope:Value=serde_json::from_str(&nomifun_common::decrypt_string(&grant.token_encrypted,&KEY).unwrap()).unwrap();
    assert_eq!(envelope["access_token"],"access-issued-client-1-1");
    assert_eq!(envelope["discovery"]["issuer"],format!("{}/auth",fixture.base));
    let client=fixture.repository.get_client_by_ref(&fixture.owner,&grant.client_ref).await.unwrap().unwrap();
    let secret=client.client_secret_encrypted.unwrap();assert!(!secret.contains("instance-secret-1"));
    assert_eq!(nomifun_common::decrypt_string(&secret,&KEY).unwrap(),"instance-secret-1");
    fixture.state.lock().unwrap().mcp_authorizations.clear();
    let tested=fixture.probe().test_saved_connection(&server,"login",&transport).await;
    assert!(tested.success,"{tested:?}");assert_eq!(tested.tools.unwrap().len(),1);
    let state=fixture.state.lock().unwrap();
    assert!(state.requests.iter().any(|path|path=="/metadata/one"));
    assert!(state.metadata_queries.iter().all(|query|query=="tenant=fixture-tenant&revision=17"));
    assert!(state.mcp_authorizations.len()>=3);
    assert!(state.mcp_authorizations.iter().all(|value|value.as_deref()==Some("Bearer access-issued-client-1-1")));
    drop(state);
    assert_eq!(fixture.service.status(server.as_str()).await.unwrap().state,McpAuthState::Authorized);
    fixture.shutdown().await;
}

#[tokio::test]
async fn simultaneous_consumers_refresh_once_and_rotate_without_changing_the_connection_or_grant_identity() {
    let fixture=Fixture::new(0).await;let (server,transport)=fixture.server("refresh","one").await;
    fixture.login(&server,"approved-refresh").await;
    let before=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    let probe=fixture.probe();
    let (first,second)=tokio::join!(probe.test_saved_connection(&server,"refresh",&transport),probe.test_saved_connection(&server,"refresh",&transport));
    assert!(first.success,"{first:?}");assert!(second.success,"{second:?}");
    let after=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    assert_eq!(after.grant_id,before.grant_id);assert_eq!(after.client_ref,before.client_ref);
    assert_eq!(after.connection_config_ref,before.connection_config_ref);assert_eq!(after.revision,before.revision+1);
    let envelope:Value=serde_json::from_str(&nomifun_common::decrypt_string(&after.token_encrypted,&KEY).unwrap()).unwrap();
    assert_eq!(envelope["access_token"],"access-issued-client-1-2");assert_eq!(envelope["refresh_token"],"refresh-issued-client-1-2");
    assert_eq!(fixture.state.lock().unwrap().refreshes,1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn logout_during_token_exchange_prevents_late_callback_from_recreating_credentials() {
    let fixture=Fixture::new(3600).await;let (server,_)=fixture.server("late-callback","one").await;
    let attempt=fixture.start(&server).await;let query=fixture.callback_query(&attempt,"late-approved");
    let block=ExchangeBlock { entered:Arc::new(Notify::new()),release:Arc::new(Notify::new()) };
    fixture.state.lock().unwrap().block=Some(block.clone());
    let service=fixture.service.clone();let id=attempt.attempt_id.clone();
    let callback=tokio::spawn(async move { service.complete_callback(Some(&id),&query).await });
    tokio::time::timeout(HTTP_BUDGET,block.entered.notified()).await.unwrap();
    fixture.service.logout(server.as_str()).await.unwrap();
    block.release.notify_one();
    let result=tokio::time::timeout(HTTP_BUDGET,callback).await.unwrap().unwrap();
    assert!(matches!(result.unwrap_err().diagnostic.code,McpAuthErrorCode::Cancelled|McpAuthErrorCode::ConnectionChanged));
    assert!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().is_none());
    assert_eq!(fixture.service.attempt(server.as_str(),&attempt.attempt_id).await.unwrap().status,McpOAuthAttemptStatus::Cancelled);
    fixture.shutdown().await;
}

#[tokio::test]
async fn shutdown_drains_web_callback_exchange_and_rejects_late_authorization_activity() {
    let fixture=Fixture::new(3600).await;let (server,_)=fixture.server("shutdown-web-callback","one").await;
    // Web callbacks run in application HTTP tasks, without a native callback
    // listener JoinHandle. Shutdown must still own and drain their activity.
    let service=McpAuthService::new(fixture.repository.clone(),fixture.servers.clone(),fixture.owner.clone(),KEY,
        fixture.client.clone(),McpAuthOptions {
            callback_url:Some("https://callback.example/api/mcp/oauth/callback".into()),
            allow_http_loopback:true,attempt_timeout:Duration::from_secs(30),..Default::default()
        });
    let attempt=service.start(server.as_str(),McpOAuthStartRequest::default(),false).await.unwrap();
    assert_eq!(attempt.status,McpOAuthAttemptStatus::WaitingForUser,"{:?}",attempt.diagnostic);
    let query=fixture.callback_query(&attempt,"approved-during-shutdown");
    let late_query=query.clone();
    let block=ExchangeBlock { entered:Arc::new(Notify::new()),release:Arc::new(Notify::new()) };
    fixture.state.lock().unwrap().block=Some(block.clone());
    let callback_service=service.clone();let id=attempt.attempt_id.clone();
    let callback=tokio::spawn(async move { callback_service.complete_callback(Some(&id),&query).await });
    tokio::time::timeout(HTTP_BUDGET,block.entered.notified()).await.unwrap();
    // The upstream response remains blocked. A proper shutdown cancels its
    // wait rather than requiring that response or closing SQLite underneath it.
    tokio::time::timeout(HTTP_BUDGET,service.shutdown()).await.expect("shutdown must drain an external WebUI callback");
    let outcome=tokio::time::timeout(HTTP_BUDGET,callback).await.unwrap().unwrap();
    assert!(outcome.is_err());
    assert!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().is_none());
    block.release.notify_one();
    let start=service.start(server.as_str(),McpOAuthStartRequest::default(),false).await.unwrap_err();
    assert_eq!(start.diagnostic.code,McpAuthErrorCode::ConnectionChanged);
    let late=service.complete_callback(Some(&attempt.attempt_id),&late_query).await.unwrap_err();
    assert_eq!(late.diagnostic.code,McpAuthErrorCode::ConnectionChanged);
    assert!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().is_none());
    fixture.shutdown().await;
}

#[tokio::test]
async fn connections_with_the_same_endpoint_keep_authorization_and_logout_isolated() {
    let fixture=Fixture::new(3600).await;
    let (first,first_transport)=fixture.server("same-endpoint-first","shared").await;
    let (second,second_transport)=fixture.server("same-endpoint-second","shared").await;
    fixture.login(&first,"approved-first").await;
    assert_eq!(fixture.service.status(second.as_str()).await.unwrap().state,McpAuthState::Anonymous);
    assert_eq!(fixture.probe().test_saved_connection(&second,"second",&second_transport).await.needs_auth,Some(true));
    fixture.login(&second,"approved-second").await;
    let one=fixture.repository.get_grant(&fixture.owner,first.as_str()).await.unwrap().unwrap();
    let two=fixture.repository.get_grant(&fixture.owner,second.as_str()).await.unwrap().unwrap();
    assert_ne!(one.grant_id,two.grant_id);assert_ne!(one.client_ref,two.client_ref);
    fixture.service.logout(first.as_str()).await.unwrap();
    assert!(fixture.repository.get_grant(&fixture.owner,first.as_str()).await.unwrap().is_none());
    assert_eq!(fixture.repository.get_grant(&fixture.owner,second.as_str()).await.unwrap().unwrap(),two);
    assert!(fixture.probe().test_saved_connection(&second,"second",&second_transport).await.success);
    assert_eq!(fixture.probe().test_saved_connection(&first,"first",&first_transport).await.needs_auth,Some(true));
    fixture.shutdown().await;
}

#[tokio::test]
async fn callback_rejects_missing_issuer_wrong_state_duplicates_and_replay_before_token_exchange() {
    let fixture=Fixture::new(3600).await;let (server,_)=fixture.server("callback","one").await;
    let attempt=fixture.start(&server).await;let valid=fixture.callback_query(&attempt,"approved-callback");
    let mut parts=parse_form(valid.as_bytes());parts.remove("iss");
    let query=form_urlencoded::Serializer::new(String::new()).extend_pairs(parts).finish();
    assert_eq!(fixture.service.complete_callback(Some(&attempt.attempt_id),&query).await.unwrap_err().diagnostic.code,McpAuthErrorCode::IssuerMismatch);
    assert_eq!(fixture.service.complete_callback(Some(&attempt.attempt_id),"code=x&state=wrong").await.unwrap_err().diagnostic.code,McpAuthErrorCode::StateMismatch);
    assert_eq!(fixture.service.complete_callback(Some(&attempt.attempt_id),&format!("{valid}&code=duplicate")).await.unwrap_err().diagnostic.code,McpAuthErrorCode::CallbackInvalid);
    assert_eq!(fixture.state.lock().unwrap().exchanges,0);
    fixture.service.complete_callback(Some(&attempt.attempt_id),&valid).await.unwrap();
    assert!(fixture.service.complete_callback(Some(&attempt.attempt_id),&valid).await.is_err());
    assert_eq!(fixture.state.lock().unwrap().exchanges,1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn expired_access_without_refresh_is_never_sent_even_if_the_server_still_accepts_it() {
    let fixture=Fixture::new(0).await;let (server,transport)=fixture.server("expired-no-refresh","one").await;
    fixture.login(&server,"approved-expired").await;
    let grant=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    let mut envelope:Value=serde_json::from_str(&nomifun_common::decrypt_string(&grant.token_encrypted,&KEY).unwrap()).unwrap();
    envelope["refresh_token"]=Value::Null;
    fixture.repository.compare_and_swap_grant(nomifun_db::McpOAuthGrantRefreshParams {
        owner_id:fixture.owner.to_string(),grant_id:grant.grant_id,expected_revision:grant.revision,
        scopes_json:grant.scopes_json,token_encrypted:nomifun_common::encrypt_string(&envelope.to_string(),&KEY).unwrap(),
        expires_at:Some(nomifun_common::now_ms()-1),obtained_at:grant.obtained_at,
    }).await.unwrap().unwrap();
    fixture.state.lock().unwrap().mcp_authorizations.clear();
    let result=fixture.probe().test_saved_connection(&server,"expired-no-refresh",&transport).await;
    assert!(!result.success);
    assert_eq!(result.details.unwrap()["owner_code"],"MCP_CREDENTIAL_EXPIRED");
    assert!(fixture.state.lock().unwrap().mcp_authorizations.is_empty());
    assert_eq!(fixture.service.status(server.as_str()).await.unwrap().state,McpAuthState::ReauthorizationRequired);
    fixture.shutdown().await;
}

#[tokio::test]
async fn configured_headers_share_the_authority_without_starting_oauth_or_registering_a_client() {
    let fixture=Fixture::new(3600).await;let (server,_)=fixture.server("configured-header","one").await;
    let url=format!("{}/mcp/one",fixture.base);
    let headers=HashMap::from([("Authorization".to_owned(),"Bearer manual-fixture-token".to_owned())]);
    let config=json!({"url":url,"headers":headers}).to_string();
    fixture.servers.update(server.as_str(),nomifun_db::UpdateMcpServerParams {
        transport_config:Some(&config),..Default::default()
    }).await.unwrap();
    fixture.state.lock().unwrap().tokens.insert("manual".into(),IssuedToken {
        resource:url.clone(),access:"manual-fixture-token".into(),refresh:String::new(),generation:1,
    });
    let transport=McpServerTransport::Http { url,headers };
    let result=fixture.probe().test_saved_connection(&server,"configured-header",&transport).await;
    assert!(result.success,"{result:?}");
    assert!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().is_none());
    assert_eq!(fixture.state.lock().unwrap().registrations,0);assert_eq!(fixture.state.lock().unwrap().exchanges,0);
    assert_eq!(fixture.state.lock().unwrap().refreshes,0);
    fixture.shutdown().await;
}

#[tokio::test]
async fn a_provider_rejected_unexpired_access_token_is_refreshed_once_during_initialize() {
    let fixture=Fixture::new(3600).await;let (server,transport)=fixture.server("early-rejection","one").await;
    fixture.login(&server,"approved-early-rejection").await;
    let before=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    {
        let mut state=fixture.state.lock().unwrap();
        state.tokens.get_mut("issued-client-1").unwrap().access="provider-revoked-old-access".into();
        state.mcp_methods.clear();
    }
    let result=fixture.probe().test_saved_connection(&server,"early-rejection",&transport).await;
    assert!(result.success,"{result:?}");
    let after=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    assert_eq!(after.grant_id,before.grant_id);assert_eq!(after.connection_config_ref,before.connection_config_ref);
    assert_eq!(fixture.state.lock().unwrap().refreshes,1);
    assert_eq!(fixture.state.lock().unwrap().mcp_methods,vec!["initialize","initialize","notifications/initialized","tools/list"]);
    assert_eq!(fixture.service.status(server.as_str()).await.unwrap().state,McpAuthState::Authorized);
    fixture.shutdown().await;
}

#[tokio::test]
async fn an_invalid_refresh_grant_cannot_keep_reporting_authorized_or_send_expired_access() {
    let fixture=Fixture::new(0).await;let (server,transport)=fixture.server("invalid-refresh","one").await;
    fixture.login(&server,"approved-invalid-refresh").await;
    {
        let mut state=fixture.state.lock().unwrap();
        state.tokens.get_mut("issued-client-1").unwrap().refresh="provider-revoked-refresh".into();
        state.mcp_methods.clear();
    }
    let result=fixture.probe().test_saved_connection(&server,"invalid-refresh",&transport).await;
    assert!(!result.success);
    assert!(!format!("{result:?}").contains("must-not-leak-refresh-secret"));
    assert!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().is_none());
    let status=fixture.service.status(server.as_str()).await.unwrap();
    assert_eq!(status.state,McpAuthState::ReauthorizationRequired);
    assert_eq!(status.diagnostic.unwrap().code,McpAuthErrorCode::ReauthorizationRequired);
    assert_eq!(fixture.state.lock().unwrap().refreshes,1);assert!(fixture.state.lock().unwrap().mcp_methods.is_empty());
    fixture.shutdown().await;
}

#[tokio::test]
async fn authentication_rejection_after_tool_dispatch_is_not_refreshed_or_replayed() {
    use nomifun_mcp::{McpInvocationControl,McpOwner,McpServerBinding,McpToolBinding,McpToolInvocationRequest};
    let fixture=Fixture::new(3600).await;let (server,transport)=fixture.server("no-replay","one").await;
    fixture.login(&server,"approved-no-replay").await;
    fixture.state.lock().unwrap().reject_tool_call=true;
    fixture.state.lock().unwrap().mcp_methods.clear();
    let reference=nomifun_mcp::canonical_mcp_connection_config_ref(&server,&transport).unwrap();
    let binding=McpServerBinding {
        server_id:server.to_string(),server_owner_id:fixture.owner.to_string(),enabled:true,
        connection_config_ref:reference.clone(),resource_binding_id:"fixture-binding".into(),
        resource_kind:"mcp_server".into(),resource_id:server.to_string(),resource_owner_id:fixture.owner.to_string(),
        granted_operations:BTreeSet::from(["connect".into(),"invoke".into(),"read".into()]),
        resource_connection_config_ref:Some(reference),transport,
    };
    let request=McpToolInvocationRequest {
        principal_kind:"user".into(),principal_id:fixture.owner.to_string(),operation_id:"fixture-no-replay-operation".into(),server:binding,
        tool:McpToolBinding::new(server.to_string(),"fixture.read","a".repeat(64),json!({"type":"object","properties":{}}),"fixture_read").unwrap(),
        arguments:json!({}),
    };
    let owner=McpOwner::new(Arc::new(fixture.service.clone()),fixture.client.clone());
    let result=owner.invoke_controlled(request,McpInvocationControl::default()).await;
    assert!(result.result.is_err());assert!(result.execution.request_dispatched);
    assert_eq!(fixture.state.lock().unwrap().mcp_methods.iter().filter(|method|method.as_str()=="tools/call").count(),1);
    assert_eq!(fixture.state.lock().unwrap().refreshes,0);
    assert_eq!(fixture.service.status(server.as_str()).await.unwrap().state,McpAuthState::ReauthorizationRequired);
    fixture.shutdown().await;
}

#[tokio::test]
async fn insufficient_scope_requires_explicit_scope_upgrade_without_refresh_or_tool_replay() {
    use nomifun_mcp::{McpInvocationControl,McpOwner,McpServerBinding,McpToolBinding,McpToolInvocationRequest};
    let fixture=Fixture::new(3600).await;let (server,transport)=fixture.server("scope-upgrade","one").await;
    fixture.login(&server,"approved-scope-upgrade").await;
    let before=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    // Model an earlier completed scope upgrade. A second upgrade must retain
    // that grant's write scope when this operation now requests read scope.
    let before=fixture.repository.compare_and_swap_grant(nomifun_db::McpOAuthGrantRefreshParams {
        owner_id:fixture.owner.to_string(), grant_id:before.grant_id, expected_revision:before.revision,
        scopes_json:json!(["mcp:connect","design:write"]).to_string(), token_encrypted:before.token_encrypted,
        expires_at:before.expires_at, obtained_at:before.obtained_at,
    }).await.unwrap().unwrap();
    fixture.state.lock().unwrap().reject_tool_scope=Some("design:read".into());
    fixture.state.lock().unwrap().mcp_methods.clear();
    let reference=nomifun_mcp::canonical_mcp_connection_config_ref(&server,&transport).unwrap();
    let binding=McpServerBinding {
        server_id:server.to_string(),server_owner_id:fixture.owner.to_string(),enabled:true,
        connection_config_ref:reference.clone(),resource_binding_id:"fixture-binding".into(),
        resource_kind:"mcp_server".into(),resource_id:server.to_string(),resource_owner_id:fixture.owner.to_string(),
        granted_operations:BTreeSet::from(["connect".into(),"invoke".into(),"read".into()]),
        resource_connection_config_ref:Some(reference),transport,
    };
    let request=McpToolInvocationRequest {
        principal_kind:"user".into(),principal_id:fixture.owner.to_string(),operation_id:"fixture-scope-upgrade-operation".into(),server:binding,
        tool:McpToolBinding::new(server.to_string(),"fixture.read","a".repeat(64),json!({"type":"object","properties":{}}),"fixture_read").unwrap(),
        arguments:json!({}),
    };
    let owner=McpOwner::new(Arc::new(fixture.service.clone()),fixture.client.clone());
    let result=owner.invoke_controlled(request,McpInvocationControl::default()).await;
    assert_eq!(result.result.unwrap_err().code(),"MCP_INSUFFICIENT_SCOPE");
    assert!(result.execution.request_dispatched);
    assert_eq!(fixture.state.lock().unwrap().mcp_methods.iter().filter(|method|method.as_str()=="tools/call").count(),1);
    assert_eq!(fixture.state.lock().unwrap().refreshes,0);
    let status=fixture.service.status(server.as_str()).await.unwrap();
    assert_eq!(status.state,McpAuthState::ReauthorizationRequired);
    let diagnostic=status.diagnostic.unwrap();
    assert_eq!(diagnostic.code,McpAuthErrorCode::InsufficientScope);
    assert_eq!(diagnostic.user_action,nomifun_api_types::McpAuthUserAction::GrantScope);
    assert_eq!(diagnostic.details.unwrap()["required_scopes"],json!(["design:read"]));
    assert_eq!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap(),before);
    let next=fixture.start(&server).await;
    let authorization=Url::parse(next.authorization_url.as_deref().unwrap()).unwrap();
    let scope=authorization.query_pairs().find(|(name,_)|name=="scope").unwrap().1.into_owned();
    let scopes:BTreeSet<_>=scope.split_whitespace().collect();
    assert_eq!(scopes,BTreeSet::from(["mcp:connect","design:write","design:read"]));
    assert_eq!(fixture.state.lock().unwrap().refreshes,0);
    assert_eq!(fixture.state.lock().unwrap().exchanges,1,"requesting an authorization URL must not grant new scopes");
    fixture.shutdown().await;
}

#[tokio::test]
async fn cancelling_an_authorized_attempt_is_a_conflict_and_preserves_the_completed_grant() {
    let fixture=Fixture::new(3600).await;let (server,transport)=fixture.server("cancel-completed","one").await;
    let attempt=fixture.login(&server,"approved-cancel-completed").await;
    let before=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    let error=fixture.service.cancel_attempt(server.as_str(),&attempt.attempt_id).await.unwrap_err();
    assert_eq!(error.status(),StatusCode::CONFLICT);
    assert_eq!(error.diagnostic.code,McpAuthErrorCode::ConnectionChanged);
    assert_eq!(fixture.service.attempt(server.as_str(),&attempt.attempt_id).await.unwrap().status,McpOAuthAttemptStatus::Authorized);
    assert_eq!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap(),before);
    assert_eq!(fixture.service.status(server.as_str()).await.unwrap().state,McpAuthState::Authorized);
    assert!(fixture.probe().test_saved_connection(&server,"cancel-completed",&transport).await.success);
    fixture.shutdown().await;
}

#[tokio::test]
async fn editing_connection_invalidates_the_grant_and_pending_authorization() {
    use nomifun_api_types::{McpTransport, UpdateMcpServerRequest};
    let fixture=Fixture::new(3600).await;
    let (server,_)=fixture.server("edit-auth","one").await;
    fixture.login(&server,"initial-edit-login").await;
    let pending=fixture.start(&server).await;
    let callback=fixture.callback_query(&pending,"stale-after-edit");
    let config=nomifun_mcp::McpConfigService::new(fixture.servers.clone())
        .with_auth_service(Arc::new(fixture.service.clone()));
    config.edit_server(&server,UpdateMcpServerRequest {
        name:None,description:None,original_json:None,builtin:None,
        transport:Some(McpTransport::Http { url:format!("{}/mcp/two",fixture.base),headers:HashMap::new() }),
    }).await.unwrap();
    assert!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().is_none());
    assert_eq!(fixture.service.attempt(server.as_str(),&pending.attempt_id).await.unwrap().status,McpOAuthAttemptStatus::Cancelled);
    assert_eq!(fixture.service.complete_callback(Some(&pending.attempt_id),&callback).await.unwrap_err().diagnostic.code,McpAuthErrorCode::ConnectionChanged);
    assert_eq!(fixture.state.lock().unwrap().exchanges,1,"the cancelled callback must not exchange another code");
    fixture.shutdown().await;
}

#[tokio::test]
async fn metadata_toggle_same_connection_and_rejected_rename_preserve_authorization() {
    use nomifun_api_types::{CreateMcpServerRequest, McpTransport, UpdateMcpServerRequest};
    let fixture=Fixture::new(3600).await;
    let (server,transport)=fixture.server("metadata-auth","one").await;
    fixture.login(&server,"metadata-login").await;
    let before=fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap();
    let config=nomifun_mcp::McpConfigService::new(fixture.servers.clone())
        .with_auth_service(Arc::new(fixture.service.clone()));
    config.edit_server(&server,UpdateMcpServerRequest {
        name:Some("metadata-auth".into()),description:Some(Some("Updated description".into())),
        transport:Some(McpTransport::from(transport.clone())),original_json:None,builtin:None,
    }).await.unwrap();
    config.toggle_server(&server).await.unwrap();
    assert!(config.edit_server(&server,UpdateMcpServerRequest {
        name:Some("not-an-allowed-rename".into()),description:None,transport:None,original_json:None,builtin:None,
    }).await.is_err());
    config.add_server(CreateMcpServerRequest { name:"metadata-auth".into(),description:None,
        transport:McpTransport::from(transport),original_json:None,builtin:false }).await.unwrap();
    assert_eq!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().unwrap(),before);
    fixture.shutdown().await;
}

#[tokio::test]
async fn replacing_connection_by_import_invalidates_existing_authorization() {
    use nomifun_api_types::{CreateMcpServerRequest, McpTransport};
    let fixture=Fixture::new(3600).await;
    let (server,_)=fixture.server("upsert-auth","one").await;
    fixture.login(&server,"upsert-login").await;
    let config=nomifun_mcp::McpConfigService::new(fixture.servers.clone())
        .with_auth_service(Arc::new(fixture.service.clone()));
    config.add_server(CreateMcpServerRequest { name:"upsert-auth".into(),description:None,
        transport:McpTransport::Http { url:format!("{}/mcp/two",fixture.base),headers:HashMap::new() },
        original_json:None,builtin:false }).await.unwrap();
    assert!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().is_none());
    fixture.shutdown().await;
}

#[tokio::test]
async fn deleting_connection_during_exchange_prevents_late_callback_from_recreating_credentials() {
    let fixture=Fixture::new(3600).await;
    let (server,_)=fixture.server("delete-auth","one").await;
    let attempt=fixture.start(&server).await;
    let query=fixture.callback_query(&attempt,"delete-late-approved");
    let block=ExchangeBlock { entered:Arc::new(Notify::new()),release:Arc::new(Notify::new()) };
    fixture.state.lock().unwrap().block=Some(block.clone());
    let auth=fixture.service.clone();let id=attempt.attempt_id.clone();
    let callback=tokio::spawn(async move { auth.complete_callback(Some(&id),&query).await });
    tokio::time::timeout(HTTP_BUDGET,block.entered.notified()).await.unwrap();
    let config=nomifun_mcp::McpConfigService::new(fixture.servers.clone())
        .with_auth_service(Arc::new(fixture.service.clone()));
    tokio::time::timeout(HTTP_BUDGET,config.delete_server(&server)).await.unwrap().unwrap();
    block.release.notify_one();
    let failure=tokio::time::timeout(HTTP_BUDGET,callback).await.unwrap().unwrap().unwrap_err();
    assert!(matches!(failure.diagnostic.code,McpAuthErrorCode::Cancelled|McpAuthErrorCode::ConnectionChanged));
    assert!(fixture.repository.get_grant(&fixture.owner,server.as_str()).await.unwrap().is_none());
    assert!(fixture.servers.find_by_id(server.as_str()).await.unwrap().is_none());
    fixture.shutdown().await;
}

#[tokio::test]
async fn probe_cannot_publish_after_an_unauthorized_authorized_unauthorized_cycle() {
    let fixture=Fixture::new(3600).await;
    let (server,transport)=fixture.server("probe-auth-aba","one").await;
    let config=nomifun_mcp::McpConfigService::new(fixture.servers.clone())
        .with_auth_service(Arc::new(fixture.service.clone()));
    let pending_probe=config.begin_probe(&server,"probe-auth-aba",&transport).await.unwrap().unwrap();
    let expected_auth=fixture.service.binding_revision(server.as_str()).await.unwrap();
    fixture.login(&server,"probe-aba-login").await;
    fixture.service.logout(server.as_str()).await.unwrap();
    let outcome=nomifun_api_types::McpConnectionTestResult {
        success:true,tools:Some(vec![]),error:None,code:None,details:None,
        needs_auth:None,auth_method:None,
    };
    assert!(matches!(config.finish_authenticated_probe(pending_probe,&outcome,expected_auth).await,
        Err(nomifun_mcp::McpError::StaleProbe)));
    assert_eq!(fixture.servers.find_by_id(server.as_str()).await.unwrap().unwrap().last_test_status,"disconnected");
    fixture.shutdown().await;
}

#[tokio::test]
async fn disconnected_or_reconfigured_connections_do_not_reuse_previous_attempt_diagnostics() {
    use nomifun_api_types::{McpTransport, UpdateMcpServerRequest};
    let fixture = Fixture::new(3600).await;
    let (server, _) = fixture.server("old-diagnostic", "one").await;
    for reconfigure in [false, true] {
        let attempt = fixture.start(&server).await;
        let authorization = Url::parse(attempt.authorization_url.as_deref().unwrap()).unwrap();
        let state = authorization.query_pairs().find(|(name, _)| name == "state").unwrap().1.into_owned();
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("error", "invalid_scope").append_pair("state", &state)
            .append_pair("iss", &format!("{}/auth", fixture.base)).finish();
        assert_eq!(fixture.service.complete_callback(Some(&attempt.attempt_id), &query).await.unwrap_err()
            .diagnostic.code, McpAuthErrorCode::InsufficientScope);
        assert_eq!(fixture.service.status(server.as_str()).await.unwrap().state, McpAuthState::ReauthorizationRequired);
        if reconfigure {
            nomifun_mcp::McpConfigService::new(fixture.servers.clone())
                .with_auth_service(Arc::new(fixture.service.clone()))
                .edit_server(&server, UpdateMcpServerRequest {
                    name: None, description: None, original_json: None, builtin: None,
                    transport: Some(McpTransport::Http { url: format!("{}/mcp/two", fixture.base), headers: HashMap::new() }),
                }).await.unwrap();
        } else {
            fixture.service.logout(server.as_str()).await.unwrap();
        }
        let status = fixture.service.status(server.as_str()).await.unwrap();
        assert!(status.diagnostic.is_none());
        assert_eq!(status.state, if reconfigure { McpAuthState::Anonymous } else { McpAuthState::Required });
    }
    fixture.shutdown().await;
}
