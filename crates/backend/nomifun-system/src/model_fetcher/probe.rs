//! Provider-level connection probing: is this address an API, and does this key
//! reach it?
//!
//! Before this existed the only way to test a connection was the per-model
//! health check, which is keyed on `(provider_id, model, task)` and fails closed
//! on the capability row. A newly created custom provider therefore had nothing
//! to test until a model capability existed, and the first feedback the user got
//! was a raw failure from a real inference attempt.
//!
//! The probe answers three states rather than pass/fail, because the situation
//! that matters most cannot be expressed as either: a 401/403 means the address
//! is right and the credential is not. It also refuses to call a `200 OK` with a
//! document body a success — that is a gateway serving its web UI at a near-miss
//! path, and reporting it as reachable is how a wrong base URL survives review.

use std::time::{Duration, Instant};

use nomifun_api_types::{
    ModelTask, ProbeCandidateResult, ProbeProviderConnectionAnonymousRequest,
    ProbeProviderConnectionRequest, ProbeProviderConnectionResponse, ProviderHealthCheckErrorKind,
    ProviderReachability,
};
use nomifun_common::AppError;
use nomifun_model_invoke::{
    AuthMaterial, ResolvedConnection, preset_protocol_recommendation, protocol_task_descriptor,
    resolve_submit_url, root_candidates,
};
use nomifun_net::api_response::{NON_API_DIAGNOSTIC, is_non_api_content_type, looks_like_markup};
use nomifun_net::secret_redaction::redact_url_queries;
use tracing::debug;

use super::{FetchConfig, ModelFetchService};

/// Probes are a reachability question, not a generation request; a short cap is
/// enough and keeps the UI responsive.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Model name substituted into an endpoint template's `{model}` placeholder.
///
/// The probe deliberately does not require a real model: its question is whether
/// the address resolves, and an unknown model is answered by the same endpoint.
const PROBE_MODEL: &str = "probe";

/// How many bytes of body to read when deciding whether it is markup.
const BODY_SNIFF_BYTES: usize = 512;

impl ModelFetchService {
    /// Probe a persisted provider's configured connection root.
    pub async fn probe_connection(
        &self,
        provider_id: &str,
        req: &ProbeProviderConnectionRequest,
    ) -> Result<ProbeProviderConnectionResponse, AppError> {
        let config = self.load_provider_config(provider_id).await?;
        self.run_probe(
            &config,
            req.protocol.as_deref(),
            req.task.unwrap_or(ModelTask::Chat),
            req.model.as_deref(),
            req.probe_candidates,
        )
        .await
    }

    /// Probe a proposed connection before any provider row exists.
    pub async fn probe_connection_anonymous(
        &self,
        req: &ProbeProviderConnectionAnonymousRequest,
    ) -> Result<ProbeProviderConnectionResponse, AppError> {
        let config = self.anonymous_probe_config(req)?;
        self.run_probe(
            &config,
            req.protocol.as_deref(),
            req.task.unwrap_or(ModelTask::Chat),
            req.model.as_deref(),
            req.probe_candidates,
        )
        .await
    }

    async fn run_probe(
        &self,
        config: &FetchConfig,
        protocol: Option<&str>,
        task: ModelTask,
        model: Option<&str>,
        probe_candidates: bool,
    ) -> Result<ProbeProviderConnectionResponse, AppError> {
        let started = Instant::now();
        let protocol = resolve_probe_protocol(&config.platform, protocol, task)?;
        let descriptor = protocol_task_descriptor(&protocol, task).ok_or_else(|| {
            AppError::BadRequest(format!(
                "protocol {protocol:?} does not serve task {task:?}"
            ))
        })?;
        let root_shape = descriptor.root_shape.ok_or_else(|| {
            AppError::BadRequest(format!(
                "protocol {protocol:?} uses an SDK transport and has no URL to probe"
            ))
        })?;
        let template = descriptor
            .endpoints
            .iter()
            .find(|endpoint| endpoint.field == "endpoint")
            .map(|endpoint| endpoint.default_value.clone())
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "protocol {protocol:?} declares no submit endpoint for task {task:?}"
                ))
            })?;

        let client = self.http_client();
        let model = model.map(str::trim).filter(|value| !value.is_empty()).unwrap_or(PROBE_MODEL);
        let configured = self
            .probe_one(
                &client,
                &config.base_url,
                &protocol,
                task,
                &template,
                model,
                &config.auth,
            )
            .await?;

        let mut candidates = Vec::new();
        let mut suggested_base_url = None;
        if configured.reachability == ProviderReachability::Unreachable && probe_candidates {
            // Deterministic order, and never a doubled version segment.
            for candidate_root in root_candidates(&config.base_url) {
                if candidate_root == config.base_url.trim().trim_end_matches('/') {
                    continue;
                }
                let outcome = self
                    .probe_one(
                        &client,
                        &candidate_root,
                        &protocol,
                        task,
                        &template,
                        model,
                        &config.auth,
                    )
                    .await?;
                let better = outcome.reachability != ProviderReachability::Unreachable;
                candidates.push(outcome.into());
                if better && suggested_base_url.is_none() {
                    suggested_base_url = Some(candidate_root);
                }
            }
        }

        debug!(
            platform = %config.platform,
            %protocol,
            reachability = ?configured.reachability,
            suggested = ?suggested_base_url,
            "provider connection probe finished"
        );

        Ok(ProbeProviderConnectionResponse {
            reachability: configured.reachability,
            protocol,
            task,
            root_shape,
            attempted_url: configured.attempted_url,
            http_status: configured.http_status,
            error_kind: Some(error_kind_for(
                configured.reachability,
                configured.http_status,
                configured.content_type.is_some(),
            )),
            content_type: configured.content_type,
            message: configured.message,
            elapsed_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
            suggested_base_url,
            candidates,
        })
    }

    /// Send one probe request against one root and classify the answer.
    async fn probe_one(
        &self,
        client: &reqwest::Client,
        base_url: &str,
        protocol: &str,
        task: ModelTask,
        template: &str,
        model: &str,
        auth: &AuthMaterial,
    ) -> Result<ProbeOutcome, AppError> {
        // The URL is built by the SAME function real inference uses, so a probe
        // can never confirm an address that inference would then miss.
        let connection = ResolvedConnection {
            role: "default".to_owned(),
            base_url: base_url.to_owned(),
            auth: auth.clone(),
            extra: serde_json::Value::Null,
        };
        let url = resolve_submit_url(&connection, protocol, task, template, model, false)
            .map_err(|error| AppError::BadRequest(error.message))?;
        let redacted_url = redact_url_queries(&url);

        let request = client
            .post(&url)
            .timeout(PROBE_TIMEOUT)
            // An empty JSON object is enough to prove the path exists: a correct
            // endpoint answers 400, which costs no tokens and produces no content.
            .json(&serde_json::json!({}));
        let request = auth
            .apply(request)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;

        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                return Ok(ProbeOutcome {
                    base_url: base_url.to_owned(),
                    attempted_url: redacted_url,
                    reachability: ProviderReachability::Unreachable,
                    http_status: None,
                    content_type: None,
                    message: Some(transport_message(&error)),
                });
            }
        };

        let status = response.status();
        let content_type = is_non_api_content_type(response.headers());
        let redactor = nomifun_net::secret_redaction::SecretRedactor::new(auth.secrets());
        let body = response.bytes().await.unwrap_or_default();
        let sniffed = &body[..body.len().min(BODY_SNIFF_BYTES)];
        let markup = content_type.is_some() || looks_like_markup(sniffed);

        let reachability = if markup {
            // A document at any status means there is no API here, even at 200.
            ProviderReachability::Unreachable
        } else if matches!(status.as_u16(), 401 | 403) {
            ProviderReachability::CredentialsRejected
        } else if status.as_u16() == 404 || status.is_server_error() {
            ProviderReachability::Unreachable
        } else {
            // 200/400/422/429 all prove the endpoint exists and parsed the request.
            ProviderReachability::Reachable
        };

        let message = if markup {
            Some(NON_API_DIAGNOSTIC.to_owned())
        } else if reachability == ProviderReachability::Reachable {
            None
        } else {
            Some(redactor.redact(&body_excerpt(&body)))
        };

        Ok(ProbeOutcome {
            base_url: base_url.to_owned(),
            attempted_url: redacted_url,
            reachability,
            http_status: Some(status.as_u16()),
            content_type,
            message,
        })
    }

    fn anonymous_probe_config(
        &self,
        req: &ProbeProviderConnectionAnonymousRequest,
    ) -> Result<FetchConfig, AppError> {
        crate::provider::validate_provider_base_url(&req.platform, &req.base_url)?;
        crate::provider::validate_provider_auth(
            &req.platform,
            &req.auth_scheme,
            &req.credentials,
            None,
        )?;
        Ok(FetchConfig {
            platform: req.platform.clone(),
            base_url: req.base_url.clone(),
            auth: AuthMaterial {
                scheme: nomifun_model_invoke::AuthScheme::parse(&req.auth_scheme)
                    .map_err(|error| AppError::BadRequest(error.to_string()))?,
                credentials: req.credentials.clone(),
            },
            bedrock_config: None,
        })
    }
}

struct ProbeOutcome {
    base_url: String,
    attempted_url: String,
    reachability: ProviderReachability,
    http_status: Option<u16>,
    content_type: Option<String>,
    message: Option<String>,
}

impl From<ProbeOutcome> for ProbeCandidateResult {
    fn from(outcome: ProbeOutcome) -> Self {
        Self {
            base_url: outcome.base_url,
            attempted_url: outcome.attempted_url,
            reachability: outcome.reachability,
            http_status: outcome.http_status,
            content_type: outcome.content_type,
        }
    }
}

/// Pick the protocol whose endpoint template defines the path to probe.
///
/// Fails closed rather than guessing. `custom`/`new-api` have no recommendation
/// by design, so for those the caller must say which protocol it means — the UI
/// always knows, because the user picked one.
fn resolve_probe_protocol(
    platform: &str,
    requested: Option<&str>,
    task: ModelTask,
) -> Result<String, AppError> {
    if let Some(protocol) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(protocol.to_owned());
    }
    preset_protocol_recommendation(platform, task)
        .map(|route| route.protocol.to_owned())
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "platform {platform:?} has no recommended protocol for task {task:?}; specify `protocol` explicitly"
            ))
        })
}

/// Map a probe outcome onto the shared health-check vocabulary so the UI renders
/// one set of diagnoses regardless of which surface produced them.
fn error_kind_for(
    reachability: ProviderReachability,
    http_status: Option<u16>,
    markup: bool,
) -> ProviderHealthCheckErrorKind {
    use ProviderHealthCheckErrorKind as K;
    if markup {
        return K::NonApiResponse;
    }
    match (reachability, http_status) {
        (ProviderReachability::CredentialsRejected, Some(403)) => K::Forbidden,
        (ProviderReachability::CredentialsRejected, _) => K::Unauthorized,
        (ProviderReachability::Unreachable, Some(404)) => K::NotFound,
        (ProviderReachability::Unreachable, Some(_)) => K::ApiError,
        (ProviderReachability::Unreachable, None) => K::ConnectionError,
        (ProviderReachability::Reachable, Some(429)) => K::RateLimited,
        (ProviderReachability::Reachable, _) => K::Unknown,
    }
}

/// Never expose reqwest's Display text: it includes the request URL, which can
/// carry credentials in a query parameter.
fn transport_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "the request timed out".to_owned()
    } else if error.is_connect() {
        "could not connect; check DNS, TLS, firewall and proxy settings".to_owned()
    } else {
        "the request failed before a response was received".to_owned()
    }
}

fn body_excerpt(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(&body[..body.len().min(300)]);
    text.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_body_is_never_reachable_whatever_the_status() {
        assert_eq!(error_kind_for(ProviderReachability::Unreachable, Some(200), true), ProviderHealthCheckErrorKind::NonApiResponse);
        assert_eq!(error_kind_for(ProviderReachability::Unreachable, Some(404), true), ProviderHealthCheckErrorKind::NonApiResponse);
    }

    #[test]
    fn a_rejected_credential_is_distinguished_from_a_missing_address() {
        assert_eq!(
            error_kind_for(ProviderReachability::CredentialsRejected, Some(401), false),
            ProviderHealthCheckErrorKind::Unauthorized
        );
        assert_eq!(
            error_kind_for(ProviderReachability::CredentialsRejected, Some(403), false),
            ProviderHealthCheckErrorKind::Forbidden
        );
        assert_eq!(
            error_kind_for(ProviderReachability::Unreachable, Some(404), false),
            ProviderHealthCheckErrorKind::NotFound
        );
    }

    #[test]
    fn a_custom_platform_must_name_its_protocol_rather_than_be_guessed() {
        let error = resolve_probe_protocol("custom", None, ModelTask::Chat).unwrap_err();
        assert!(
            error.to_string().contains("specify `protocol`"),
            "got: {error}"
        );
        assert_eq!(
            resolve_probe_protocol("custom", Some("openai.chat_text"), ModelTask::Chat).unwrap(),
            "openai.chat_text"
        );
    }

    #[test]
    fn a_builtin_platform_falls_back_to_its_recommendation() {
        assert_eq!(
            resolve_probe_protocol("openai", None, ModelTask::Chat).unwrap(),
            "openai.chat_text"
        );
    }
}
