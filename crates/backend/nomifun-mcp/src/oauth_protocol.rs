//! MCP OAuth wire protocol. This module owns no login or application sessions.
//!
//! Callers must use `http_client()` (or an equivalently redirect-disabled test
//! client). Authorization material must never be followed through redirects.

use std::{collections::BTreeMap, fmt, time::Duration};
use base64::{Engine, engine::general_purpose::STANDARD};
use nomifun_api_types::{McpAuthErrorCode as Code, McpAuthStage as Stage};
use oauth2::url::Url;
use reqwest::{Client, StatusCode, header::HeaderMap};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use crate::oauth_failure::McpAuthFailure;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
// Keep accepted OAuth identifiers within the authority store's schema bound.
const MAX_IDENTIFIER_BYTES: usize = 4096;

#[derive(Clone, Serialize, Deserialize)]
pub struct DiscoveryContext {
    pub endpoint: String,
    pub resource: String,
    pub issuer: String,
    pub resource_metadata_url: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub scopes: Vec<String>,
    pub token_auth_methods: Vec<String>,
    pub authorization_response_iss_supported: bool,
    pub cimd_supported: bool,
    #[serde(default)]
    pub refresh_supported: bool,
}

impl fmt::Debug for DiscoveryContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscoveryContext").field("issuer", &self.issuer)
            .field("cimd_supported", &self.cimd_supported)
            .field("authorization_response_iss_supported", &self.authorization_response_iss_supported)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ClientRegistration {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    #[serde(default)]
    pub client_secret_expires_at: Option<i64>,
    pub token_endpoint_auth_method: String,
    pub redirect_uri: String,
}

impl fmt::Debug for ClientRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientRegistration").field("issuer", &self.issuer)
            .field("token_endpoint_auth_method", &self.token_endpoint_auth_method)
            .field("client_secret", &"[REDACTED]").finish_non_exhaustive()
    }
}

#[derive(Clone, Default)]
pub struct ClientOptions {
    pub pre_registered: Option<ClientRegistration>,
    pub cimd_client_id: Option<String>,
}

pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_in: Option<u64>,
    pub scopes: Vec<String>,
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet").field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticateChallenge {
    pub scheme: String,
    pub parameters: BTreeMap<String, String>,
}

/// Uses the shared proxy configuration and never follows redirects.
pub fn http_client() -> Result<Client, McpAuthFailure> {
    nomifun_net::http_client_no_redirect().map_err(|_| failure(Code::Internal, Stage::Connection,
        "The configured OAuth HTTP client could not be initialized"))
}

fn failure(code: Code, stage: Stage, message: &'static str) -> McpAuthFailure {
    McpAuthFailure::new(code, stage, message)
}

fn loopback(url: &Url) -> bool {
    match url.host() {
        Some(oauth2::url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(oauth2::url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(oauth2::url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

fn checked_url(value: &str, allow_loopback: bool, stage: Stage) -> Result<Url, McpAuthFailure> {
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(failure(Code::MetadataInvalid, stage, "The OAuth URL exceeds the supported length limit"));
    }
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(failure(Code::MetadataInvalid, stage, "OAuth URLs must not contain whitespace padding or control characters"));
    }
    let url = Url::parse(value).map_err(|_| failure(Code::MetadataInvalid, stage.clone(),
        "OAuth metadata contains an invalid URL"))?;
    if url.host().is_none() || !url.username().is_empty() || url.password().is_some()
        || url.fragment().is_some()
        || (url.scheme() != "https" && !(allow_loopback && url.scheme() == "http" && loopback(&url)))
    {
        return Err(failure(Code::MetadataInvalid, stage, "OAuth endpoints must use HTTPS without embedded credentials or fragments"));
    }
    Ok(url)
}

fn issuer_url(value: &str, allow_loopback: bool) -> Result<Url, McpAuthFailure> {
    let url = checked_url(value, allow_loopback, Stage::Discovery)?;
    if url.query().is_some() { return Err(failure(Code::MetadataInvalid, Stage::Discovery,
        "The authorization server issuer must not contain a query")); }
    Ok(url)
}

/// Splits challenge parameters, preserving commas and escaped quotes inside quoted strings.
fn comma_parts(input: &str) -> Vec<&str> {
    let (mut quoted, mut escaped, mut start) = (false, false, 0);
    let mut parts = Vec::new();
    for (offset, character) in input.char_indices() {
        if escaped { escaped = false; continue; }
        if quoted && character == '\\' { escaped = true; continue; }
        if character == '"' { quoted = !quoted; }
        if character == ',' && !quoted { parts.push(&input[start..offset]); start = offset + 1; }
    }
    if !quoted && !escaped { parts.push(&input[start..]); }
    parts
}

fn token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn parameter(input: &str) -> Option<(String, String)> {
    let (name, value) = input.split_once('=')?;
    let name = name.trim();
    if !token(name) { return None; }
    let value = value.trim();
    let decoded = if let Some(quoted) = value.strip_prefix('"') {
        let quoted = quoted.strip_suffix('"')?;
        let mut result = String::new();
        let mut escape = false;
        for character in quoted.chars() {
            if escape { result.push(character); escape = false; }
            else if character == '\\' { escape = true; }
            else if character == '"' || character.is_control() { return None; }
            else { result.push(character); }
        }
        if escape { return None; }
        result
    } else {
        if !token(value) { return None; }
        value.to_owned()
    };
    Some((name.to_ascii_lowercase(), decoded))
}

pub fn parse_challenges(input: &str) -> Vec<AuthenticateChallenge> {
    let mut result: Vec<AuthenticateChallenge> = Vec::new();
    for part in comma_parts(input) {
        let part = part.trim();
        if part.is_empty() { continue; }
        if let Some((name, value)) = parameter(part) {
            if let Some(challenge) = result.last_mut() {
                // Ambiguous security parameters invalidate this challenge.
                if challenge.parameters.insert(name.clone(), value).is_some() {
                    challenge.parameters.clear(); challenge.scheme.clear();
                }
            }
            continue;
        }
        let (scheme, rest) = part.split_once(char::is_whitespace).unwrap_or((part, ""));
        if !token(scheme) { continue; }
        let mut challenge = AuthenticateChallenge { scheme: scheme.to_ascii_lowercase(), parameters: BTreeMap::new() };
        if !rest.trim().is_empty() {
            if let Some((name, value)) = parameter(rest.trim()) { challenge.parameters.insert(name, value); }
        }
        result.push(challenge);
    }
    result.retain(|challenge| !challenge.scheme.is_empty());
    result
}

pub fn parse_www_authenticate(headers: &HeaderMap) -> Vec<AuthenticateChallenge> {
    headers.get_all(reqwest::header::WWW_AUTHENTICATE).iter()
        .filter_map(|value| value.to_str().ok()).flat_map(parse_challenges).collect()
}

#[derive(Deserialize)]
struct ResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    #[serde(default)] scopes_supported: Vec<String>,
}

#[derive(Deserialize)]
struct ServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    #[serde(default)] code_challenge_methods_supported: Vec<String>,
    response_types_supported: Vec<String>,
    grant_types_supported: Option<Vec<String>>,
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
    #[serde(default)] authorization_response_iss_parameter_supported: bool,
    #[serde(default)] client_id_metadata_document_supported: bool,
}

enum Document<T> { Present(T), Missing }

async fn bounded_json<T: DeserializeOwned>(mut response: reqwest::Response, stage: Stage) -> Result<T, McpAuthFailure> {
    if response.content_length().is_some_and(|length| length > MAX_DOCUMENT_BYTES as u64) {
        return Err(failure(Code::MetadataInvalid, stage, "The OAuth response exceeded its size limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failure(Code::UpstreamUnavailable, stage.clone(),
        "The OAuth server response could not be read"))? {
        if bytes.len().saturating_add(chunk.len()) > MAX_DOCUMENT_BYTES {
            return Err(failure(Code::MetadataInvalid, stage, "The OAuth response exceeded its size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| failure(Code::MetadataInvalid, stage,
        "The OAuth server returned an invalid JSON document"))
}

async fn document<T: DeserializeOwned>(client: &Client, url: &str, allow_loopback: bool) -> Result<Document<T>, McpAuthFailure> {
    let requested = checked_url(url, allow_loopback, Stage::Discovery)?;
    let response = client.get(requested.clone()).timeout(REQUEST_TIMEOUT).send().await
        .map_err(|_| failure(Code::UpstreamUnavailable, Stage::Discovery,
            "The OAuth metadata endpoint could not be reached"))?;
    if response.url() != &requested || response.status().is_redirection() {
        return Err(failure(Code::MetadataInvalid, Stage::Discovery, "OAuth metadata redirects are not accepted"));
    }
    let status = response.status();
    if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) { return Ok(Document::Missing); }
    if status != StatusCode::OK {
        return Err(failure(Code::UpstreamUnavailable, Stage::Discovery, "The OAuth metadata endpoint rejected the request")
            .with_details(json!({"http_status":status.as_u16()})));
    }
    let is_json = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(';').next().is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json")));
    if !is_json { return Err(failure(Code::MetadataInvalid, Stage::Discovery, "OAuth metadata must be an application/json document")); }
    bounded_json(response, Stage::Discovery).await.map(Document::Present)
}

fn origin(url: &Url) -> String { url.origin().ascii_serialization() }

fn resource_candidates(endpoint: &str, url: &Url, challenge: Option<&str>) -> Vec<(String, String)> {
    if let Some(metadata) = challenge.and_then(|value| parse_challenges(value).into_iter()
        .find_map(|challenge| challenge.parameters.get("resource_metadata").cloned()))
    { return vec![(metadata, endpoint.to_owned())]; }
    let root = origin(url);
    let mut candidates = Vec::new();
    if url.path() != "/" {
        let mut path = format!("{root}/.well-known/oauth-protected-resource{}", url.path());
        if let Some(query) = url.query() { path.push('?'); path.push_str(query); }
        candidates.push((path, endpoint.to_owned()));
    }
    let root_resource = if url.path() == "/" { endpoint.to_owned() } else { root.clone() };
    candidates.push((format!("{root}/.well-known/oauth-protected-resource"), root_resource));
    candidates
}

fn server_candidates(issuer: &str, parsed: &Url) -> Vec<String> {
    let root = origin(parsed);
    let path = parsed.path().trim_end_matches('/');
    if path.is_empty() {
        vec![format!("{root}/.well-known/oauth-authorization-server"), format!("{root}/.well-known/openid-configuration")]
    } else {
        vec![format!("{root}/.well-known/oauth-authorization-server{path}"),
            format!("{root}/.well-known/openid-configuration{path}"),
            format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'))]
    }
}

fn scope_values(scope: &str) -> Result<Vec<String>, McpAuthFailure> {
    let mut scopes = Vec::new();
    for value in scope.split(' ').filter(|value| !value.is_empty()) {
        if !value.bytes().all(|byte| byte == 0x21 || (0x23..=0x5b).contains(&byte) || (0x5d..=0x7e).contains(&byte)) {
            return Err(failure(Code::MetadataInvalid, Stage::Discovery, "The OAuth scope set is invalid"));
        }
        if !scopes.iter().any(|existing| existing == value) { scopes.push(value.to_owned()); }
    }
    Ok(scopes)
}

pub async fn discover(client: &Client, endpoint: &str, challenge: Option<&str>, allow_loopback: bool) -> Result<DiscoveryContext, McpAuthFailure> {
    let endpoint_url = checked_url(endpoint, allow_loopback, Stage::Discovery)
        .map_err(|error| error.with_details(json!({"field":"endpoint"})))?;
    let mut protected = None;
    for (metadata_url, resource) in resource_candidates(endpoint, &endpoint_url, challenge) {
        match document::<ResourceMetadata>(client, &metadata_url, allow_loopback).await? {
            Document::Missing => continue,
            Document::Present(metadata) => {
                if metadata.resource != resource {
                    return Err(failure(Code::ResourceMismatch, Stage::Discovery, "Protected resource metadata identifies a different resource"));
                }
                checked_url(&metadata.resource, allow_loopback, Stage::Discovery)?;
                if metadata.authorization_servers.is_empty() || metadata.authorization_servers.len() > 16 {
                    return Err(failure(Code::MetadataInvalid, Stage::Discovery, "Protected resource metadata has no usable authorization server"));
                }
                protected = Some((metadata_url, metadata)); break;
            }
        }
    }
    let (resource_metadata_url, metadata) = protected.ok_or_else(|| failure(Code::ResourceMetadataUnavailable, Stage::Discovery,
        "This MCP server did not provide protected resource metadata"))?;
    // Selection is deterministic. Registrations are never tried against another issuer.
    let issuer = metadata.authorization_servers.first().expect("nonempty checked above");
    let issuer_parsed = issuer_url(issuer, allow_loopback)?;
    let mut discovered = None;
    for candidate in server_candidates(issuer, &issuer_parsed) {
        match document::<ServerMetadata>(client, &candidate, allow_loopback).await? {
            Document::Missing => continue,
            Document::Present(server) => { discovered = Some(server); break; }
        }
    }
    let server = discovered.ok_or_else(|| failure(Code::ResourceMetadataUnavailable, Stage::Discovery,
        "The selected authorization server did not publish discovery metadata"))?;
    if server.issuer != *issuer { return Err(failure(Code::IssuerMismatch, Stage::Discovery,
        "Authorization server metadata identifies a different issuer")); }
    if !server.code_challenge_methods_supported.iter().any(|method| method == "S256") {
        return Err(failure(Code::PkceUnsupported, Stage::Discovery, "The authorization server does not advertise S256 PKCE"));
    }
    if !server.response_types_supported.iter().any(|value| value == "code")
        || server.grant_types_supported.as_ref().is_some_and(|types| !types.iter().any(|value| value == "authorization_code"))
    { return Err(failure(Code::MetadataInvalid, Stage::Discovery, "The authorization server does not support authorization code grants")); }
    for url in [Some(server.authorization_endpoint.as_str()), Some(server.token_endpoint.as_str()),
        server.registration_endpoint.as_deref()].into_iter().flatten()
    { checked_url(url, allow_loopback, Stage::Discovery)?; }
    let scopes = challenge.and_then(|value| parse_challenges(value).into_iter()
        .filter(|entry| entry.scheme == "bearer").find_map(|entry| entry.parameters.get("scope").cloned()));
    let scopes = match scopes { Some(scope) => scope_values(&scope)?, None => {
        let mut scopes = Vec::new();
        for scope in metadata.scopes_supported { scopes.extend(scope_values(&scope)?); }
        scopes.sort(); scopes.dedup(); scopes
    } };
    Ok(DiscoveryContext {
        endpoint: endpoint.to_owned(), resource: metadata.resource, issuer: server.issuer,
        resource_metadata_url, authorization_endpoint: server.authorization_endpoint,
        token_endpoint: server.token_endpoint, registration_endpoint: server.registration_endpoint,
        scopes,
        token_auth_methods: server.token_endpoint_auth_methods_supported.unwrap_or_else(|| vec!["client_secret_basic".into()]),
        authorization_response_iss_supported: server.authorization_response_iss_parameter_supported,
        cimd_supported: server.client_id_metadata_document_supported,
        refresh_supported: server.grant_types_supported.as_ref().is_some_and(|types| types.iter().any(|value| value == "refresh_token")),
    })
}

fn validate_redirect(value: &str) -> Result<(), McpAuthFailure> {
    checked_url(value, true, Stage::ClientRegistration).map_err(|_| failure(Code::RedirectUnavailable, Stage::ClientRegistration,
        "The redirect URI must use HTTPS or an explicit loopback HTTP address"))?;
    Ok(())
}

pub fn validate_registration(context: &DiscoveryContext, registration: &ClientRegistration) -> Result<(), McpAuthFailure> {
    if registration.issuer != context.issuer { return Err(failure(Code::IssuerMismatch, Stage::ClientRegistration,
        "Client registration belongs to a different authorization server")); }
    if registration.client_id.is_empty() || registration.client_id.len() > MAX_IDENTIFIER_BYTES || registration.client_id.chars().any(char::is_control) {
        return Err(failure(Code::InvalidConfiguration, Stage::ClientRegistration, "The OAuth client identifier is invalid"));
    }
    if registration.client_secret_expires_at.is_some_and(|expires| expires <= nomifun_common::now_ms()) {
        return Err(failure(Code::ClientRegistrationRequired, Stage::ClientRegistration, "The registered OAuth client credential has expired"));
    }
    validate_redirect(&registration.redirect_uri)?;
    let method = registration.token_endpoint_auth_method.as_str();
    if method == "none" && registration.client_secret.is_some() {
        return Err(failure(Code::InvalidConfiguration, Stage::ClientRegistration,
            "A public OAuth client using no client authentication must not include a client secret"));
    }
    if !matches!(method, "none" | "client_secret_basic" | "client_secret_post")
        || !context.token_auth_methods.iter().any(|value| value == method)
        || (method != "none" && registration.client_secret.as_ref().is_none_or(|secret| secret.is_empty() || secret.chars().any(char::is_control)))
    { return Err(failure(Code::ClientAuthenticationUnsupported, Stage::ClientRegistration,
        "The registered OAuth client authentication method is unavailable or unsupported")); }
    Ok(())
}

#[derive(Deserialize)]
struct RegistrationResponse {
    client_id: String,
    client_secret: Option<String>,
    token_endpoint_auth_method: Option<String>,
    redirect_uris: Vec<String>,
    client_secret_expires_at: Option<u64>,
}

pub async fn register_client(client: &Client, context: &DiscoveryContext, options: &ClientOptions,
    redirect_uri: &str, allow_loopback: bool) -> Result<ClientRegistration, McpAuthFailure>
{
    validate_redirect(redirect_uri)?;
    if let Some(registered) = &options.pre_registered {
        let mut registered = registered.clone();
        let original = Url::parse(&registered.redirect_uri).map_err(|_| failure(Code::RedirectUnavailable,
            Stage::ClientRegistration, "The registered redirect URI is invalid"))?;
        let requested = Url::parse(redirect_uri).map_err(|_| failure(Code::RedirectUnavailable,
            Stage::ClientRegistration, "The requested redirect URI is invalid"))?;
        let native_port_variation = loopback(&original) && loopback(&requested)
            && original.scheme() == "http" && requested.scheme() == "http"
            && original.host() == requested.host() && original.path() == requested.path()
            && original.query() == requested.query() && original.fragment().is_none() && requested.fragment().is_none();
        if registered.redirect_uri != redirect_uri && !native_port_variation {
            return Err(failure(Code::RedirectUnavailable, Stage::ClientRegistration,
                "The requested redirect does not match this client registration"));
        }
        registered.redirect_uri = redirect_uri.to_owned();
        validate_registration(context, &registered)?;
        return Ok(registered);
    }
    if context.cimd_supported && let Some(client_id) = &options.cimd_client_id {
        let cimd = checked_url(client_id, false, Stage::ClientRegistration)?;
        // URL parsing removes dot segments; inspect the supplied document
        // identity first rather than accepting a silently normalized client ID.
        let dot_segment = client_id.split_once("://").and_then(|(_, rest)| rest.split_once('/'))
            .map(|(_, path)| path.split('?').next().unwrap_or_default().split('/').any(|segment| {
                matches!(segment.to_ascii_lowercase().replace("%2e", ".").as_str(), "." | "..")
            })).unwrap_or(false);
        if cimd.path() == "/" || dot_segment {
            return Err(failure(Code::InvalidConfiguration, Stage::ClientRegistration, "The client metadata identifier must be an HTTPS document URL"));
        }
        let registration = ClientRegistration { issuer: context.issuer.clone(), client_id: client_id.clone(),
            client_secret: None, client_secret_expires_at: None, token_endpoint_auth_method: "none".into(), redirect_uri: redirect_uri.to_owned() };
        validate_registration(context, &registration)?;
        return Ok(registration);
    }
    let endpoint = context.registration_endpoint.as_ref().ok_or_else(|| failure(Code::ClientRegistrationRequired, Stage::ClientRegistration,
        "This authorization server requires registered client information"))?;
    let endpoint = checked_url(endpoint, allow_loopback, Stage::ClientRegistration)?;
    let method = if context.token_auth_methods.iter().any(|value| value == "none") { "none" }
        else if context.token_auth_methods.iter().any(|value| value == "client_secret_basic") { "client_secret_basic" }
        else if context.token_auth_methods.iter().any(|value| value == "client_secret_post") { "client_secret_post" }
        else { return Err(failure(Code::ClientAuthenticationUnsupported, Stage::ClientRegistration,
            "The authorization server requires an unsupported client authentication method")); };
    let redirect = Url::parse(redirect_uri).map_err(|_| failure(Code::RedirectUnavailable, Stage::ClientRegistration, "The redirect URI is invalid"))?;
    let grant_types = if context.refresh_supported { vec!["authorization_code", "refresh_token"] } else { vec!["authorization_code"] };
    let response = client.post(endpoint.clone()).timeout(REQUEST_TIMEOUT).json(&json!({
        "client_name":"NomiFun", "redirect_uris":[redirect_uri], "response_types":["code"],
        "grant_types":grant_types, "token_endpoint_auth_method":method,
        "application_type":if loopback(&redirect) { "native" } else { "web" }
    })).send().await.map_err(|_| failure(Code::UpstreamUnavailable, Stage::ClientRegistration,
        "The OAuth client registration endpoint could not be reached"))?;
    if response.url() != &endpoint || response.status().is_redirection() { return Err(failure(Code::ClientRegistrationRejected,
        Stage::ClientRegistration, "OAuth client registration redirects are not accepted")); }
    if !response.status().is_success() { return Err(oauth_rejection(response, Stage::ClientRegistration, Code::ClientRegistrationRejected).await); }
    let registered: RegistrationResponse = bounded_json(response, Stage::ClientRegistration).await?;
    let client_secret_expires_at = registered.client_secret_expires_at.filter(|expiry| *expiry != 0)
        .map(|expiry| i64::try_from(expiry).ok().and_then(|seconds| seconds.checked_mul(1000))
            .ok_or_else(|| failure(Code::ClientRegistrationRejected, Stage::ClientRegistration,
                "The OAuth client credential expiry is invalid"))).transpose()?;
    if !registered.redirect_uris.iter().any(|value| value == redirect_uri)
        || client_secret_expires_at.is_some_and(|expiry| expiry <= nomifun_common::now_ms())
    { return Err(failure(Code::ClientRegistrationRejected, Stage::ClientRegistration,
        "The registered client does not accept this redirect or has expired")); }
    let registration = ClientRegistration { issuer: context.issuer.clone(), client_id: registered.client_id,
        client_secret: registered.client_secret,
        client_secret_expires_at,
        token_endpoint_auth_method: registered.token_endpoint_auth_method.unwrap_or_else(|| "client_secret_basic".into()),
        redirect_uri: redirect_uri.to_owned() };
    validate_registration(context, &registration).map_err(|error| {
        if error.diagnostic.code == Code::InvalidConfiguration {
            failure(Code::ClientRegistrationRejected, Stage::ClientRegistration,
                "The OAuth server returned inconsistent client registration data")
        } else { error }
    })?;
    Ok(registration)
}

pub fn authorization_url(context: &DiscoveryContext, registration: &ClientRegistration, state: &str,
    pkce_challenge: &str, requested_scopes: &[String]) -> Result<String, McpAuthFailure>
{
    validate_registration(context, registration)?;
    if state.is_empty() || pkce_challenge.len() != 43 || !pkce_challenge.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte)) {
        return Err(failure(Code::InvalidConfiguration, Stage::Authorization, "The authorization request is missing secure state or S256 PKCE"));
    }
    let mut url = Url::parse(&context.authorization_endpoint).map_err(|_| failure(Code::MetadataInvalid,
        Stage::Authorization, "The authorization endpoint is invalid"))?;
    let reserved = ["response_type","client_id","redirect_uri","state","code_challenge","code_challenge_method","resource","scope"];
    if url.query_pairs().any(|(name, _)| reserved.contains(&name.as_ref())) { return Err(failure(Code::MetadataInvalid,
        Stage::Authorization, "The authorization endpoint contains conflicting OAuth parameters")); }
    let mut pairs = url.query_pairs_mut();
    pairs.append_pair("response_type", "code").append_pair("client_id", &registration.client_id)
        .append_pair("redirect_uri", &registration.redirect_uri).append_pair("state", state)
        .append_pair("code_challenge", pkce_challenge).append_pair("code_challenge_method", "S256")
        .append_pair("resource", &context.resource);
    if !requested_scopes.is_empty() { pairs.append_pair("scope", &requested_scopes.join(" ")); }
    drop(pairs);
    Ok(url.into())
}

/// Only standardized error codes are retained. Provider bodies/descriptions,
/// request URLs, codes and credentials are never diagnostic data.
async fn oauth_rejection(response: reqwest::Response, stage: Stage, fallback: Code) -> McpAuthFailure {
    let status = response.status();
    let body = bounded_json::<Value>(response, stage.clone()).await.ok();
    let reported = body.as_ref().and_then(|value| value.get("error")).and_then(Value::as_str);
    let safe_error = reported.filter(|value| matches!(*value,
        "invalid_request" | "invalid_client" | "invalid_grant" | "unauthorized_client" | "unsupported_grant_type"
        | "invalid_scope" | "access_denied" | "server_error" | "temporarily_unavailable" | "invalid_redirect_uri"
        | "invalid_client_metadata" | "invalid_target"));
    let code = if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS { Code::UpstreamUnavailable }
        else { match safe_error {
            Some("invalid_grant") if matches!(&stage, Stage::Refresh) => Code::ReauthorizationRequired,
            Some("invalid_client" | "unauthorized_client") => Code::ClientRegistrationRejected,
            Some("invalid_scope") => Code::InsufficientScope,
            Some("server_error" | "temporarily_unavailable") => Code::UpstreamUnavailable,
            Some("access_denied") => Code::UserDenied,
            Some("invalid_redirect_uri") => Code::RedirectUnavailable,
            _ => fallback,
        } };
    failure(code, stage, "The authorization server rejected the OAuth request")
        .with_details(json!({"http_status":status.as_u16(),"oauth_error":safe_error}))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: Option<u64>,
    scope: Option<String>,
}

async fn token_request(client: &Client, context: &DiscoveryContext, registration: &ClientRegistration,
    mut form: Vec<(&str, String)>, stage: Stage, allow_loopback: bool) -> Result<TokenSet, McpAuthFailure>
{
    validate_registration(context, registration).map_err(|failure| failure.at_stage(stage.clone()))?;
    let endpoint = checked_url(&context.token_endpoint, allow_loopback, stage.clone())?;
    form.push(("resource", context.resource.clone()));
    let mut request = client.post(endpoint.clone()).timeout(REQUEST_TIMEOUT);
    match registration.token_endpoint_auth_method.as_str() {
        "client_secret_basic" => {
            // RFC 6749 §2.3.1 requires form-encoding each credential before Basic.
            let encode = |value: &str| oauth2::url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
            let encoded = STANDARD.encode(format!("{}:{}", encode(&registration.client_id), encode(registration.client_secret.as_deref().unwrap_or_default())));
            request = request.header(reqwest::header::AUTHORIZATION, format!("Basic {encoded}"));
        }
        "client_secret_post" => {
            form.push(("client_id", registration.client_id.clone()));
            form.push(("client_secret", registration.client_secret.clone().unwrap_or_default()));
        }
        "none" => { form.push(("client_id", registration.client_id.clone())); }
        _ => unreachable!("registration authentication is validated above"),
    }
    let response = request.form(&form).send().await.map_err(|_| failure(Code::UpstreamUnavailable,
        stage.clone(), "The OAuth token endpoint could not be reached"))?;
    if response.url() != &endpoint || response.status().is_redirection() { return Err(failure(Code::TokenExchangeRejected,
        stage, "OAuth token endpoint redirects are not accepted")); }
    if !response.status().is_success() { return Err(oauth_rejection(response, stage, Code::TokenExchangeRejected).await); }
    let token: TokenResponse = bounded_json(response, stage.clone()).await?;
    if !token.token_type.eq_ignore_ascii_case("bearer") || token.access_token.is_empty()
        || token.access_token.chars().any(|character| character.is_control() || character.is_whitespace())
        || token.refresh_token.as_ref().is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
    { return Err(failure(Code::TokenExchangeRejected, stage, "The OAuth server returned an unsupported or invalid token")); }
    let scopes = match token.scope { Some(scope) => scope_values(&scope).map_err(|failure| failure.at_stage(stage))?, None => Vec::new() };
    Ok(TokenSet { access_token: token.access_token, refresh_token: token.refresh_token,
        token_type: "Bearer".into(), expires_in: token.expires_in, scopes })
}

pub async fn exchange_code(client: &Client, context: &DiscoveryContext, registration: &ClientRegistration,
    code: &str, verifier: &str, allow_loopback: bool) -> Result<TokenSet, McpAuthFailure>
{
    if code.is_empty() || !(43..=128).contains(&verifier.len()) || !verifier.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)) {
        return Err(failure(Code::CallbackInvalid, Stage::TokenExchange, "The authorization code or PKCE verifier is invalid"));
    }
    token_request(client, context, registration, vec![("grant_type", "authorization_code".into()),
        ("code", code.into()), ("code_verifier", verifier.into()), ("redirect_uri", registration.redirect_uri.clone())],
        Stage::TokenExchange, allow_loopback).await
}

pub async fn refresh_token(client: &Client, context: &DiscoveryContext, registration: &ClientRegistration,
    refresh: &str, allow_loopback: bool) -> Result<TokenSet, McpAuthFailure>
{
    if refresh.is_empty() { return Err(failure(Code::ReauthorizationRequired, Stage::Refresh, "No refresh token is available")); }
    token_request(client, context, registration, vec![("grant_type", "refresh_token".into()),
        ("refresh_token", refresh.into())], Stage::Refresh, allow_loopback).await
}

#[cfg(test)]
mod tests;
