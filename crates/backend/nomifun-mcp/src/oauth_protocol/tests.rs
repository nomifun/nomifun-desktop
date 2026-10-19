use super::*;
use std::{collections::HashMap, sync::{Arc, Mutex}};
use axum::{Router, body::{Body, to_bytes}, extract::{Request, State}, http::HeaderValue, response::Response};

#[derive(Clone)]
struct Reply { status: StatusCode, headers: HeaderMap, body: String }
impl Reply {
    fn json(value: Value) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Self { status: StatusCode::OK, headers, body: value.to_string() }
    }
    fn error(status: StatusCode, value: Value) -> Self { Self { status, ..Self::json(value) } }
}
#[derive(Debug)]
struct Seen { path: String, headers: HeaderMap, body: String }
#[derive(Default)]
struct FixtureState { replies: HashMap<String, Reply>, seen: Vec<Seen> }
struct Fixture { base: String, state: Arc<Mutex<FixtureState>>, task: tokio::task::JoinHandle<()> }
impl Drop for Fixture { fn drop(&mut self) { self.task.abort(); } }

async fn handle(State(state): State<Arc<Mutex<FixtureState>>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, MAX_DOCUMENT_BYTES * 2).await.unwrap();
    let mut state = state.lock().unwrap();
    let path = parts.uri.path().to_owned();
    state.seen.push(Seen { path: path.clone(), headers: parts.headers, body: String::from_utf8(body.to_vec()).unwrap() });
    let reply = state.replies.get(&path).cloned().unwrap_or_else(|| Reply::error(StatusCode::NOT_FOUND, json!({})));
    let mut response = Response::new(Body::from(reply.body));
    *response.status_mut() = reply.status; *response.headers_mut() = reply.headers;
    response
}

impl Fixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(FixtureState::default()));
        let router = Router::new().fallback(handle).with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        Self { base, state, task }
    }
    fn route(&self, path: &str, value: Value) { self.reply(path, Reply::json(value)); }
    fn reply(&self, path: &str, reply: Reply) { self.state.lock().unwrap().replies.insert(path.into(), reply); }
    fn paths(&self) -> Vec<String> { self.state.lock().unwrap().seen.iter().map(|seen| seen.path.clone()).collect() }
    fn endpoint(&self) -> String { format!("{}/tenant/mcp", self.base) }
    fn metadata(&self) {
        self.route("/.well-known/oauth-protected-resource/tenant/mcp", json!({
            "resource":self.endpoint(), "authorization_servers":[self.base], "scopes_supported":["files:read"]
        }));
        self.route("/.well-known/oauth-authorization-server", self.server_metadata());
    }
    fn server_metadata(&self) -> Value { json!({
        "issuer":self.base, "authorization_endpoint":format!("{}/authorize",self.base),
        "token_endpoint":format!("{}/token",self.base), "registration_endpoint":format!("{}/register",self.base),
        "code_challenge_methods_supported":["S256"], "response_types_supported":["code"], "token_endpoint_auth_methods_supported":["none","client_secret_basic","client_secret_post"],
        "grant_types_supported":["authorization_code","refresh_token"],
        "authorization_response_iss_parameter_supported":true
    }) }
    async fn context(&self) -> DiscoveryContext {
        self.metadata(); discover(&client(), &self.endpoint(), None, true).await.unwrap()
    }
}
fn client() -> Client { Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap() }
fn registration(context: &DiscoveryContext) -> ClientRegistration {
    ClientRegistration { issuer: context.issuer.clone(), client_id: "nomifun-issued-client".into(), client_secret: None,
        client_secret_expires_at:None,
        token_endpoint_auth_method:"none".into(), redirect_uri:"http://127.0.0.1:12345/callback".into() }
}

#[test]
fn parses_multiple_headers_challenges_and_quoted_commas() {
    let mut headers = HeaderMap::new();
    headers.append(reqwest::header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"x,y\""));
    headers.append(reqwest::header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer realm=\"escaped\\\"realm\", resource_metadata=\"https://mcp.example/meta\", scope=\"read write\""));
    let challenges = parse_www_authenticate(&headers);
    assert_eq!(challenges.len(), 2);
    assert_eq!(challenges[0].parameters["realm"], "x,y");
    assert_eq!(challenges[1].parameters["realm"], "escaped\"realm");
    assert_eq!(challenges[1].parameters["scope"], "read write");
    assert!(parse_challenges("Bearer resource_metadata=\"https://one\", resource_metadata=\"https://two\"").is_empty());
}

#[test]
fn url_validation_does_not_silently_normalize_control_characters_or_padding() {
    for url in [" https://issuer.example", "https://issuer.example ", "https://issuer.exa\nmple", "https://issuer.example/\tpath"] {
        assert_eq!(checked_url(url,false,Stage::Discovery).unwrap_err().diagnostic.code,Code::MetadataInvalid);
    }
}

#[test]
fn metadata_url_limits_match_the_persistent_issuer_and_resource_bounds() {
    let prefix="https://issuer.example/";
    let maximum=format!("{prefix}{}","a".repeat(MAX_IDENTIFIER_BYTES-prefix.len()));
    assert!(checked_url(&maximum,false,Stage::Discovery).is_ok());
    assert!(issuer_url(&maximum,false).is_ok());
    let oversized=format!("{maximum}a");
    assert_eq!(checked_url(&oversized,false,Stage::Discovery).unwrap_err().diagnostic.code,Code::MetadataInvalid);
    assert_eq!(issuer_url(&oversized,false).unwrap_err().diagnostic.code,Code::MetadataInvalid);
}

#[tokio::test]
async fn client_identifiers_and_public_client_secrets_are_rejected_before_storage() {
    let fixture=Fixture::new().await;let context=fixture.context().await;let mut configured=registration(&context);
    configured.client_id="c".repeat(MAX_IDENTIFIER_BYTES);
    assert!(validate_registration(&context,&configured).is_ok());
    configured.client_id.push('c');
    assert_eq!(validate_registration(&context,&configured).unwrap_err().diagnostic.code,Code::InvalidConfiguration);
    configured.client_id="public-client".into();configured.client_secret=Some("write-only-secret".into());
    assert_eq!(validate_registration(&context,&configured).unwrap_err().diagnostic.code,Code::InvalidConfiguration);
    fixture.route("/register",json!({"client_id":"issued-public","client_secret":"provider-secret-must-not-escape",
        "token_endpoint_auth_method":"none","redirect_uris":["http://127.0.0.1:23456/callback"]}));
    let error=register_client(&client(),&context,&ClientOptions::default(),"http://127.0.0.1:23456/callback",true).await.unwrap_err();
    assert_eq!(error.diagnostic.code,Code::ClientRegistrationRejected);
    assert!(!format!("{error:?}").contains("provider-secret-must-not-escape"));
}

#[tokio::test]
async fn discovers_resource_then_nested_issuer_with_standard_priority() {
    let fixture = Fixture::new().await;
    let issuer = format!("{}/tenant", fixture.base);
    fixture.route("/explicit-resource", json!({"resource":fixture.endpoint(),"authorization_servers":[issuer],"scopes_supported":["baseline"]}));
    let mut metadata = fixture.server_metadata(); metadata["issuer"] = issuer.clone().into();
    fixture.route("/.well-known/openid-configuration/tenant", metadata);
    let challenge = format!("Basic realm=\"other\", Bearer resource_metadata=\"{}/explicit-resource\", scope=\"required:read\"",fixture.base);
    let context = discover(&client(), &fixture.endpoint(), Some(&challenge), true).await.unwrap();
    assert_eq!(context.issuer, issuer);
    assert_eq!(context.resource, fixture.endpoint());
    assert_eq!(context.scopes, vec!["required:read"]);
    assert!(context.authorization_response_iss_supported);
    assert_eq!(fixture.paths(), vec!["/explicit-resource","/.well-known/oauth-authorization-server/tenant","/.well-known/openid-configuration/tenant"]);
}

#[tokio::test]
async fn resource_fallback_checks_path_before_root() {
    let fixture = Fixture::new().await;
    fixture.route("/.well-known/oauth-protected-resource", json!({"resource":fixture.base,"authorization_servers":[fixture.base]}));
    fixture.route("/.well-known/oauth-authorization-server", fixture.server_metadata());
    let context = discover(&client(), &fixture.endpoint(), None, true).await.unwrap();
    assert_eq!(context.resource, fixture.base);
    assert_eq!(fixture.paths(), vec!["/.well-known/oauth-protected-resource/tenant/mcp","/.well-known/oauth-protected-resource","/.well-known/oauth-authorization-server"]);
}

#[tokio::test]
async fn oidc_append_is_last_fallback_for_path_issuer() {
    let fixture = Fixture::new().await;
    let issuer = format!("{}/tenant", fixture.base);
    fixture.route("/.well-known/oauth-protected-resource/tenant/mcp", json!({"resource":fixture.endpoint(),"authorization_servers":[issuer]}));
    let mut metadata = fixture.server_metadata(); metadata["issuer"] = issuer.into();
    fixture.route("/tenant/.well-known/openid-configuration", metadata);
    discover(&client(), &fixture.endpoint(), None, true).await.unwrap();
    assert_eq!(fixture.paths()[1..], ["/.well-known/oauth-authorization-server/tenant","/.well-known/openid-configuration/tenant","/tenant/.well-known/openid-configuration"]);
}

#[tokio::test]
async fn rejects_resource_and_issuer_substitution_without_guessing_another_server() {
    let fixture = Fixture::new().await; fixture.metadata();
    fixture.route("/.well-known/oauth-protected-resource/tenant/mcp", json!({"resource":"https://other.example/mcp","authorization_servers":[fixture.base]}));
    let error = discover(&client(), &fixture.endpoint(), None, true).await.unwrap_err();
    assert_eq!(error.diagnostic.code, Code::ResourceMismatch);
    fixture.metadata();
    let mut metadata = fixture.server_metadata(); metadata["issuer"] = "https://other.example".into();
    fixture.route("/.well-known/oauth-authorization-server", metadata);
    let error = discover(&client(), &fixture.endpoint(), None, true).await.unwrap_err();
    assert_eq!(error.diagnostic.code, Code::IssuerMismatch);
    assert!(!fixture.paths().iter().any(|path| path == "/.well-known/openid-configuration"));
}

#[tokio::test]
async fn refuses_missing_s256_and_insecure_production_urls() {
    let fixture = Fixture::new().await; fixture.metadata();
    let mut metadata = fixture.server_metadata(); metadata.as_object_mut().unwrap().remove("code_challenge_methods_supported");
    fixture.route("/.well-known/oauth-authorization-server", metadata);
    assert_eq!(discover(&client(), &fixture.endpoint(), None, true).await.unwrap_err().diagnostic.code, Code::PkceUnsupported);
    assert_eq!(discover(&client(), &fixture.endpoint(), None, false).await.unwrap_err().diagnostic.code, Code::MetadataInvalid);
    fixture.metadata();
    let mut metadata = fixture.server_metadata(); metadata["token_endpoint"] = "http://nonlocal.example/token".into();
    fixture.route("/.well-known/oauth-authorization-server", metadata);
    assert_eq!(discover(&client(), &fixture.endpoint(), None, true).await.unwrap_err().diagnostic.code, Code::MetadataInvalid);
}

#[tokio::test]
async fn outage_or_invalid_document_does_not_become_missing_oauth() {
    let fixture = Fixture::new().await; fixture.metadata();
    fixture.reply("/.well-known/oauth-protected-resource/tenant/mcp", Reply::error(StatusCode::SERVICE_UNAVAILABLE,json!({"error":"secret-provider-body"})));
    let error = discover(&client(), &fixture.endpoint(), None, true).await.unwrap_err();
    assert_eq!(error.diagnostic.code, Code::UpstreamUnavailable);
    assert!(!format!("{error:?}").contains("secret-provider-body"));
    assert_eq!(fixture.paths().len(),1);
    fixture.reply("/.well-known/oauth-protected-resource/tenant/mcp", Reply { body:"x".repeat(MAX_DOCUMENT_BYTES+1), ..Reply::json(json!({})) });
    assert_eq!(discover(&client(), &fixture.endpoint(), None, true).await.unwrap_err().diagnostic.code,Code::MetadataInvalid);
}

#[tokio::test]
async fn client_selection_prefers_issuer_bound_registration_and_checks_redirect_profile() {
    let fixture = Fixture::new().await; let mut context=fixture.context().await; context.cimd_supported=true;
    let existing=registration(&context);
    let options=ClientOptions { pre_registered:Some(existing.clone()),cimd_client_id:Some("https://nomifun.example/oauth/client.json".into()) };
    let registered=register_client(&client(),&context,&options,"http://127.0.0.1:34567/callback",true).await.unwrap();
    assert_eq!(registered.client_id,existing.client_id);
    assert_eq!(registered.redirect_uri,"http://127.0.0.1:34567/callback");
    assert!(!fixture.paths().iter().any(|path|path=="/register"));
    assert_eq!(register_client(&client(),&context,&options,"http://127.0.0.1:34567/other",true).await.unwrap_err().diagnostic.code,Code::RedirectUnavailable);
    context.issuer="https://another.example".into();
    assert_eq!(register_client(&client(),&context,&options,&existing.redirect_uri,true).await.unwrap_err().diagnostic.code,Code::IssuerMismatch);
}

#[tokio::test]
async fn cimd_requires_its_advertised_authentication_capability_and_never_invents_identity() {
    let fixture=Fixture::new().await;let mut context=fixture.context().await;context.cimd_supported=true;
    let options=ClientOptions{pre_registered:None,cimd_client_id:Some("https://nomifun.example/oauth/client.json".into())};
    let registered=register_client(&client(),&context,&options,"http://127.0.0.1:23456/callback",true).await.unwrap();
    assert_eq!(registered.client_id,"https://nomifun.example/oauth/client.json");
    assert!(!fixture.paths().iter().any(|path|path=="/register"));
    let unsafe_options=ClientOptions{pre_registered:None,cimd_client_id:Some("https://nomifun.example/oauth/%2e%2e/client.json".into())};
    assert_eq!(register_client(&client(),&context,&unsafe_options,"http://127.0.0.1:23456/callback",true).await.unwrap_err().diagnostic.code,Code::InvalidConfiguration);
    context.cimd_supported=false;context.registration_endpoint=None;
    assert_eq!(register_client(&client(),&context,&options,"http://127.0.0.1:23456/callback",true).await.unwrap_err().diagnostic.code,Code::ClientRegistrationRequired);
}

#[tokio::test]
async fn dcr_provisions_per_instance_secret_and_correct_native_application_type() {
    let fixture=Fixture::new().await;let mut context=fixture.context().await;
    context.token_auth_methods=vec!["client_secret_basic".into()];
    let expires_seconds=(nomifun_common::now_ms()/1000+3600) as u64;
    fixture.route("/register",json!({"client_id":"issued-id","client_secret":"instance-secret","token_endpoint_auth_method":"client_secret_basic","client_secret_expires_at":expires_seconds,
        "redirect_uris":["http://127.0.0.1:23456/callback"]}));
    let registered=register_client(&client(),&context,&ClientOptions::default(),"http://127.0.0.1:23456/callback",true).await.unwrap();
    assert_eq!(registered.client_secret.as_deref(),Some("instance-secret"));
    assert_eq!(registered.client_secret_expires_at,Some(expires_seconds as i64*1000));
    assert!(!format!("{registered:?}").contains("instance-secret"));
    let guard=fixture.state.lock().unwrap();let sent=guard.seen.iter().find(|seen|seen.path=="/register").unwrap();
    let body:Value=serde_json::from_str(&sent.body).unwrap();
    assert_eq!(body["application_type"],"native");
    assert_eq!(body["token_endpoint_auth_method"],"client_secret_basic");
    assert_eq!(body["grant_types"],json!(["authorization_code","refresh_token"]));
}

#[tokio::test]
async fn dcr_does_not_demand_a_refresh_grant_from_code_only_servers() {
    let fixture=Fixture::new().await;let mut context=fixture.context().await;context.refresh_supported=false;
    fixture.route("/register",json!({"client_id":"code-only-id","token_endpoint_auth_method":"none",
        "redirect_uris":["http://127.0.0.1:23456/callback"]}));
    register_client(&client(),&context,&ClientOptions::default(),"http://127.0.0.1:23456/callback",true).await.unwrap();
    let state=fixture.state.lock().unwrap();let sent=state.seen.iter().find(|seen|seen.path=="/register").unwrap();
    let body:Value=serde_json::from_str(&sent.body).unwrap();assert_eq!(body["grant_types"],json!(["authorization_code"]));
}

#[tokio::test]
async fn authorization_and_exchange_bind_resource_client_pkce_and_scope() {
    let fixture=Fixture::new().await;let context=fixture.context().await;let mut registered=registration(&context);
    registered.client_id="issued:id with space".into();registered.client_secret=Some("secret:with space".into());
    registered.token_endpoint_auth_method="client_secret_basic".into();
    let verifier="A".repeat(43);
    let authorization=authorization_url(&context,&registered,"random-state",&verifier,&["files:read".into()]).unwrap();
    let url=Url::parse(&authorization).unwrap();let query:BTreeMap<_,_>=url.query_pairs().map(|(k,v)|(k.into_owned(),v.into_owned())).collect();
    assert_eq!(query["resource"],fixture.endpoint());assert_eq!(query["scope"],"files:read");
    assert_eq!(query["code_challenge_method"],"S256");
    fixture.route("/token",json!({"access_token":"access-secret","refresh_token":"refresh-secret","token_type":"Bearer","expires_in":3600,"scope":"files:read"}));
    let tokens=exchange_code(&client(),&context,&registered,"authorization-secret",&verifier,true).await.unwrap();
    assert_eq!(tokens.expires_in,Some(3600));assert_eq!(tokens.scopes,vec!["files:read"]);
    assert!(!format!("{tokens:?}").contains("access-secret"));
    let guard=fixture.state.lock().unwrap();let sent=guard.seen.iter().find(|seen|seen.path=="/token").unwrap();
    let form:BTreeMap<_,_>=oauth2::url::form_urlencoded::parse(sent.body.as_bytes()).map(|(k,v)|(k.into_owned(),v.into_owned())).collect();
    assert_eq!(form["resource"],fixture.endpoint());assert_eq!(form["code_verifier"],verifier);
    assert_eq!(form["redirect_uri"],registered.redirect_uri);
    assert_eq!(sent.headers[reqwest::header::AUTHORIZATION],format!("Basic {}",STANDARD.encode("issued%3Aid+with+space:secret%3Awith+space")));
    assert!(!form.contains_key("client_secret"));
}

#[tokio::test]
async fn refresh_classifies_invalid_grant_and_discards_arbitrary_upstream_diagnostics() {
    let fixture=Fixture::new().await;let context=fixture.context().await;let registered=registration(&context);
    fixture.reply("/token",Reply::error(StatusCode::BAD_REQUEST,json!({"error":"invalid_grant","error_description":"secret-reflection","access_token":"secret-body"})));
    let error=refresh_token(&client(),&context,&registered,"refresh-secret",true).await.unwrap_err();
    assert_eq!(error.diagnostic.code,Code::ReauthorizationRequired);assert_eq!(error.diagnostic.stage,Stage::Refresh);
    let diagnostic=format!("{error:?}");assert!(!diagnostic.contains("secret-reflection"));assert!(!diagnostic.contains("secret-body"));
    assert!(!diagnostic.contains("refresh-secret"));
    fixture.reply("/token",Reply::error(StatusCode::BAD_REQUEST,json!({"error":"temporarily_unavailable","error_description":"private-provider-detail"})));
    let error=refresh_token(&client(),&context,&registered,"refresh-secret",true).await.unwrap_err();
    assert_eq!(error.diagnostic.code,Code::UpstreamUnavailable);assert!(error.diagnostic.retryable);
}

#[tokio::test]
async fn token_redirect_never_forwards_code_or_refresh_material() {
    let fixture=Fixture::new().await;let context=fixture.context().await;let registered=registration(&context);
    let mut redirect=Reply::error(StatusCode::TEMPORARY_REDIRECT,json!({}));
    redirect.headers.insert(reqwest::header::LOCATION,HeaderValue::from_str(&format!("{}/stolen",fixture.base)).unwrap());
    fixture.reply("/token",redirect);
    let error=refresh_token(&client(),&context,&registered,"refresh-secret",true).await.unwrap_err();
    assert_eq!(error.diagnostic.code,Code::TokenExchangeRejected);
    assert!(!fixture.paths().iter().any(|path|path=="/stolen"));
}

#[tokio::test]
async fn discovery_redirect_is_rejected_before_a_second_metadata_request() {
    let fixture=Fixture::new().await;
    let mut redirect=Reply::error(StatusCode::FOUND,json!({}));
    redirect.headers.insert(reqwest::header::LOCATION,HeaderValue::from_str(&format!("{}/untrusted-metadata",fixture.base)).unwrap());
    fixture.reply("/.well-known/oauth-protected-resource/tenant/mcp",redirect);
    assert_eq!(discover(&client(),&fixture.endpoint(),None,true).await.unwrap_err().diagnostic.code,Code::MetadataInvalid);
    assert_eq!(fixture.paths(),vec!["/.well-known/oauth-protected-resource/tenant/mcp"]);
}

#[tokio::test]
async fn token_post_authentication_and_refresh_keep_credentials_in_the_form_body() {
    let fixture=Fixture::new().await;let context=fixture.context().await;let mut registered=registration(&context);
    registered.token_endpoint_auth_method="client_secret_post".into();registered.client_secret=Some("client-secret".into());
    fixture.route("/token",json!({"access_token":"new-access","token_type":"bearer"}));
    let tokens=refresh_token(&client(),&context,&registered,"old-refresh",true).await.unwrap();
    assert!(tokens.refresh_token.is_none());assert!(tokens.scopes.is_empty());assert_eq!(tokens.token_type,"Bearer");
    let state=fixture.state.lock().unwrap();let sent=state.seen.iter().find(|seen|seen.path=="/token").unwrap();
    assert!(sent.headers.get(reqwest::header::AUTHORIZATION).is_none());
    let form:BTreeMap<_,_>=oauth2::url::form_urlencoded::parse(sent.body.as_bytes()).map(|(k,v)|(k.into_owned(),v.into_owned())).collect();
    assert_eq!(form["client_secret"],"client-secret");assert_eq!(form["refresh_token"],"old-refresh");
    assert_eq!(form["resource"],fixture.endpoint());assert_eq!(form["grant_type"],"refresh_token");
    assert_eq!(sent.path,"/token");
}

#[tokio::test]
async fn registration_rejection_and_malformed_tokens_do_not_escape_as_credentials() {
    let fixture=Fixture::new().await;let context=fixture.context().await;
    fixture.reply("/register",Reply::error(StatusCode::BAD_REQUEST,json!({"error":"invalid_client_metadata","error_description":"reflected-client-secret"})));
    let error=register_client(&client(),&context,&ClientOptions::default(),"http://127.0.0.1:23456/callback",true).await.unwrap_err();
    assert_eq!(error.diagnostic.code,Code::ClientRegistrationRejected);
    assert!(!format!("{error:?}").contains("reflected-client-secret"));
    fixture.route("/token",json!({"access_token":"opaque-secret","token_type":"DPoP"}));
    assert_eq!(refresh_token(&client(),&context,&registration(&context),"old-refresh",true).await.unwrap_err().diagnostic.code,Code::TokenExchangeRejected);
    fixture.route("/token",json!({"access_token":"bad\r\nheader","token_type":"Bearer"}));
    assert_eq!(refresh_token(&client(),&context,&registration(&context),"old-refresh",true).await.unwrap_err().diagnostic.code,Code::TokenExchangeRejected);
}
