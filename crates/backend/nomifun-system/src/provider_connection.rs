use std::sync::Arc;

use nomifun_api_types::{
    ProviderConnectionInput, ProviderConnectionResponse, ProviderModelCapabilityInput,
    SaveProviderConnectionRequest,
};
use nomifun_common::{AppError, ProviderId, decrypt_string, encrypt_string};
use nomifun_db::{
    IProviderConnectionRepository, IProviderModelCapabilityRepository, IProviderRepository,
    ProviderConnectionRow, UpsertProviderConnectionParams, models::Provider,
};
use nomifun_model_invoke::{AuthMaterial, AuthScheme};

use crate::provider_model::{
    capability_row_to_response, validate_base_url, validate_capability_auth_scheme,
    validate_capability_urls,
};

const RESERVED_DEFAULT_ROLE_MESSAGE: &str =
    "role 'default' is reserved: the provider's own base_url and credentials are the default connection";

#[derive(Clone)]
pub struct ProviderConnectionService {
    repo: Arc<dyn IProviderConnectionRepository>,
    provider_repo: Arc<dyn IProviderRepository>,
    capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
    encryption_key: [u8; 32],
}

impl ProviderConnectionService {
    pub fn new(
        repo: Arc<dyn IProviderConnectionRepository>,
        provider_repo: Arc<dyn IProviderRepository>,
        capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
        encryption_key: [u8; 32],
    ) -> Self {
        Self {
            repo,
            provider_repo,
            capability_repo,
            encryption_key,
        }
    }

    pub async fn list(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderConnectionResponse>, AppError> {
        self.require_provider(provider_id).await?;
        self.repo
            .list_for_provider(provider_id)
            .await?
            .into_iter()
            .map(|row| row_to_response(row, &self.encryption_key))
            .collect()
    }

    pub async fn upsert(
        &self,
        provider_id: &str,
        req: SaveProviderConnectionRequest,
    ) -> Result<ProviderConnectionResponse, AppError> {
        let provider = self.require_provider(provider_id).await?;
        validate_connection_metadata(&req.role, &req.base_url)?;
        let auth_scheme = normalize_auth_scheme(&req.auth_scheme)?;
        let parsed_scheme = parse_auth_scheme(&auth_scheme)?;
        let existing = self.repo.get(provider_id, &req.role).await?;

        let (credentials, credentials_encrypted) = match (&req.credentials, &existing) {
            (None, Some(row)) => {
                let credentials =
                    decrypt_credentials(&row.credentials_encrypted, &self.encryption_key)?;
                (credentials, row.credentials_encrypted.clone())
            }
            (None, None) => {
                return Err(AppError::BadRequest(
                    "credentials are required when creating a connection".into(),
                ));
            }
            (Some(credentials), Some(row)) => {
                let stored_credentials =
                    decrypt_credentials(&row.credentials_encrypted, &self.encryption_key)?;
                let encrypted = if credentials == &stored_credentials {
                    row.credentials_encrypted.clone()
                } else {
                    encrypt_credentials(credentials, &self.encryption_key)?
                };
                (credentials.clone(), encrypted)
            }
            (Some(credentials), None) => (
                credentials.clone(),
                encrypt_credentials(credentials, &self.encryption_key)?,
            ),
        };
        validate_auth_credentials(parsed_scheme, credentials, &auth_scheme)?;
        self.validate_referenced_capabilities(
            provider_id,
            req.role.trim(),
            req.base_url.trim(),
            &auth_scheme,
        )
        .await?;

        let extra = normalize_extra(req.extra)?;
        let extra_json = serde_json::to_string(&extra).map_err(|error| {
            AppError::Internal(format!("failed to serialize connection extra: {error}"))
        })?;
        let row = self
            .repo
            .upsert(
                provider_id,
                provider.config_revision,
                &UpsertProviderConnectionParams {
                    role: req.role.trim(),
                    label: req.label.as_deref(),
                    base_url: req.base_url.trim(),
                    auth_scheme: &auth_scheme,
                    credentials_encrypted: &credentials_encrypted,
                    extra: &extra_json,
                },
            )
            .await?;
        row_to_response(row, &self.encryption_key)
    }

    pub async fn delete(&self, provider_id: &str, role: &str) -> Result<bool, AppError> {
        self.require_provider(provider_id).await?;
        validate_role(role)?;
        Ok(self.repo.delete(provider_id, role).await?)
    }

    async fn require_provider(&self, provider_id: &str) -> Result<Provider, AppError> {
        ProviderId::parse(provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider id: {error}")))?;
        self.provider_repo
            .find_by_id(provider_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "Provider {provider_id} not found"
            )))
    }

    async fn validate_referenced_capabilities(
        &self,
        provider_id: &str,
        role: &str,
        base_url: &str,
        auth_scheme: &str,
    ) -> Result<(), AppError> {
        for row in self
            .capability_repo
            .list_for_provider(provider_id)
            .await?
            .into_iter()
            .filter(|row| row.connection_role == role)
        {
            let response = capability_row_to_response(row)?;
            let capability = ProviderModelCapabilityInput {
                task: response.task,
                traits: response.traits,
                protocol: response.protocol,
                connection_role: response.connection_role,
                base_url_override: response.base_url_override,
                endpoint: response.endpoint,
                poll_endpoint: response.poll_endpoint,
                content_endpoint: response.content_endpoint,
                realtime_endpoint: response.realtime_endpoint,
                allow_cross_origin_credentials: response.allow_cross_origin_credentials,
                provider_params: response.provider_params,
                context_limit: response.context_limit,
                output_limit: response.output_limit,
            };
            validate_capability_auth_scheme(&capability, auth_scheme)?;
            validate_capability_urls(&capability, base_url)?;
        }
        Ok(())
    }
}

pub(crate) struct PreparedProviderConnection {
    role: String,
    label: Option<String>,
    base_url: String,
    auth_scheme: String,
    credentials_encrypted: String,
    extra: String,
}

impl PreparedProviderConnection {
    pub(crate) fn role(&self) -> &str {
        &self.role
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub(crate) fn auth_scheme(&self) -> &str {
        &self.auth_scheme
    }

    pub(crate) fn as_db(&self) -> UpsertProviderConnectionParams<'_> {
        UpsertProviderConnectionParams {
            role: &self.role,
            label: self.label.as_deref(),
            base_url: &self.base_url,
            auth_scheme: &self.auth_scheme,
            credentials_encrypted: &self.credentials_encrypted,
            extra: &self.extra,
        }
    }
}

pub(crate) fn prepare_new_connection(
    req: &ProviderConnectionInput,
    encryption_key: &[u8; 32],
) -> Result<PreparedProviderConnection, AppError> {
    validate_connection_metadata(&req.role, &req.base_url)?;
    let auth_scheme = normalize_auth_scheme(&req.auth_scheme)?;
    let parsed_scheme = parse_auth_scheme(&auth_scheme)?;
    let credentials = req.credentials.clone();
    validate_auth_credentials(parsed_scheme, credentials.clone(), &auth_scheme)?;
    let credentials_encrypted = encrypt_credentials(&credentials, encryption_key)?;
    let extra = serde_json::to_string(&normalize_extra(req.extra.clone())?).map_err(|error| {
        AppError::Internal(format!("failed to serialize connection extra: {error}"))
    })?;
    Ok(PreparedProviderConnection {
        role: req.role.trim().to_owned(),
        label: req.label.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned),
        base_url: req.base_url.trim().to_owned(),
        auth_scheme,
        credentials_encrypted,
        extra,
    })
}

fn validate_connection_metadata(role: &str, base_url: &str) -> Result<(), AppError> {
    validate_role(role.trim())?;
    validate_base_url(base_url)?;
    Ok(())
}

fn normalize_extra(extra: Option<serde_json::Value>) -> Result<serde_json::Value, AppError> {
    let extra = extra.unwrap_or_else(|| serde_json::json!({}));
    if !extra.is_object() {
        return Err(AppError::BadRequest(
            "connection extra must be a JSON object".into(),
        ));
    }
    Ok(extra)
}

pub(crate) fn encrypt_credentials(
    credentials: &serde_json::Value,
    encryption_key: &[u8; 32],
) -> Result<String, AppError> {
    let plaintext = serde_json::to_string(credentials).map_err(|error| {
        AppError::Internal(format!("failed to serialize connection credentials: {error}"))
    })?;
    encrypt_string(&plaintext, encryption_key)
}

pub(crate) fn decrypt_credentials(
    credentials_encrypted: &str,
    encryption_key: &[u8; 32],
) -> Result<serde_json::Value, AppError> {
    // Migration 031 intentionally clears legacy credential ciphertext because
    // its old plaintext shape cannot be authenticated as the new typed JSON
    // contract. Keep the provider list/edit surface available so users can
    // re-enter credentials; invocation/save validation remains fail-closed.
    if credentials_encrypted.is_empty() {
        return Ok(serde_json::json!({}));
    }
    let plaintext = decrypt_string(credentials_encrypted, encryption_key).map_err(|error| {
        AppError::Internal(format!(
            "failed to decrypt stored connection credentials: {error}"
        ))
    })?;
    serde_json::from_str(&plaintext).map_err(|error| {
        AppError::Internal(format!(
            "stored connection credentials are not valid JSON: {error}"
        ))
    })
}

pub(crate) fn credentials_have_values(credentials: &serde_json::Value) -> bool {
    match credentials {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(values) => values.iter().any(credentials_have_values),
        serde_json::Value::Object(values) => values.values().any(credentials_have_values),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

pub(crate) fn parse_auth_scheme(auth_scheme: &str) -> Result<AuthScheme, AppError> {
    AuthScheme::parse(auth_scheme)
        .map_err(|error| unsupported_auth_scheme_error(auth_scheme, &error.message))
}

pub(crate) fn validate_auth_credentials(
    scheme: AuthScheme,
    credentials: serde_json::Value,
    auth_scheme: &str,
) -> Result<(), AppError> {
    AuthMaterial {
        scheme,
        credentials,
    }
    .validate_credentials()
    .map_err(|error| {
        AppError::BadRequest(format!(
            "credentials do not match auth_scheme {auth_scheme:?}: {}",
            error.message
        ))
    })
}

/// Enforce `^[a-z][a-z0-9_-]{0,31}$`; `default` belongs to the provider row.
pub(crate) fn validate_role(role: &str) -> Result<(), AppError> {
    let mut bytes = role.bytes();
    let starts_lowercase = bytes.next().is_some_and(|byte| byte.is_ascii_lowercase());
    let tail_ok = bytes.all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
    });
    if !starts_lowercase || !tail_ok || role.len() > 32 {
        return Err(AppError::BadRequest(
            "role must match ^[a-z][a-z0-9_-]{0,31}$".into(),
        ));
    }
    if role == "default" {
        return Err(AppError::BadRequest(
            RESERVED_DEFAULT_ROLE_MESSAGE.into(),
        ));
    }
    Ok(())
}

pub(crate) fn normalize_auth_scheme(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("auth_scheme is required".into()));
    }
    if let Some((prefix, argument)) = trimmed.split_once(':') {
        let prefix = prefix.trim().to_ascii_lowercase();
        if prefix == "header_key" {
            return Ok(format!(
                "{prefix}:{}",
                argument.trim().to_ascii_lowercase()
            ));
        }
        if prefix == "query_key" {
            return Ok(format!("{prefix}:{}", argument.trim()));
        }
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn unsupported_auth_scheme_error(scheme: &str, parser_message: &str) -> AppError {
    let hint = match scheme {
        "api_key" | "x-api-key" | "x_api_key" => {
            "; use header_key:x-api-key (or another explicit header_key:<name>)"
        }
        "bedrock" | "aws_sigv4" => {
            "; Bedrock/AWS SigV4 is valid only for the provider default connection"
        }
        _ => "",
    };
    AppError::BadRequest(format!(
        "invalid auth_scheme {scheme:?}: {parser_message}{hint}"
    ))
}

fn row_to_response(
    row: ProviderConnectionRow,
    encryption_key: &[u8; 32],
) -> Result<ProviderConnectionResponse, AppError> {
    let extra = serde_json::from_str(&row.extra).map_err(|error| {
        AppError::Internal(format!("failed to parse connection extra JSON: {error}"))
    })?;
    Ok(ProviderConnectionResponse {
        connection_id: row.connection_id,
        provider_id: row.provider_id,
        role: row.role,
        label: row.label,
        base_url: row.base_url,
        auth_scheme: row.auth_scheme,
        has_credentials: credentials_have_values(&decrypt_credentials(
            &row.credentials_encrypted,
            encryption_key,
        )?),
        extra,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: [u8; 32] = [0x55; 32];

    fn row(credentials_encrypted: String) -> ProviderConnectionRow {
        ProviderConnectionRow {
            id: 1,
            connection_id: "0190f5fe-7c00-7a00-8abc-000000000001".into(),
            provider_id: "0190f5fe-7c00-7a00-8abc-000000000002".into(),
            role: "voice".into(),
            label: None,
            base_url: "https://voice.example/v1".into(),
            auth_scheme: "bedrock".into(),
            credentials_encrypted,
            extra: "{}".into(),
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn role_validation_reserves_default_and_accepts_named_roles() {
        validate_role("voice").unwrap();
        validate_role("video-poll_2").unwrap();
        for role in ["default", "Voice", "9voice", "voice role", ""] {
            assert!(validate_role(role).is_err(), "{role:?} must be rejected");
        }
    }

    #[test]
    fn auth_scheme_normalization_canonicalizes_header_names_only() {
        assert_eq!(
            normalize_auth_scheme(" HEADER_KEY:X-Api-Key ").unwrap(),
            "header_key:x-api-key"
        );
        assert_eq!(
            normalize_auth_scheme(" QUERY_KEY:ApiKey ").unwrap(),
            "query_key:ApiKey"
        );
        assert_eq!(normalize_auth_scheme(" Bearer ").unwrap(), "bearer");
    }

    #[test]
    fn response_reports_empty_typed_or_migrated_credentials_as_absent() {
        let encrypted_empty = encrypt_credentials(&serde_json::json!({}), &TEST_KEY).unwrap();
        assert!(!row_to_response(row(encrypted_empty), &TEST_KEY)
            .unwrap()
            .has_credentials);
        assert!(!row_to_response(row(String::new()), &TEST_KEY)
            .unwrap()
            .has_credentials);
    }
}
