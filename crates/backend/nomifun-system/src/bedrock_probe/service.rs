use std::time::Duration;

use aws_sdk_bedrock::config::Credentials;
use nomifun_api_types::{BedrockAuthMethod, BedrockConfig};
use nomifun_common::AppError;
use nomifun_model_invoke::{AuthMaterial, AuthScheme};
use tracing::{info, warn};

/// Default Bedrock model for lightweight connection testing.
const DEFAULT_BEDROCK_TEST_MODEL: &str = "anthropic.claude-sonnet-4-5-20250929-v1:0";

/// Timeout for Bedrock connection test.
const BEDROCK_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Service for external connection testing (Bedrock credentials).
#[derive(Clone, Default)]
pub struct ConnectionTestService;

impl ConnectionTestService {
    /// Create a new `ConnectionTestService`.
    ///
    /// Bedrock uses its own AWS SDK HTTP client, so no dependencies are
    /// needed here.
    pub fn new() -> Self {
        Self
    }

    /// Test AWS Bedrock credentials by performing a lightweight API call.
    ///
    /// Constructs an isolated credential provider (no global env pollution)
    /// and calls `get_foundation_model` as a zero-cost validation.
    pub async fn test_bedrock_connection(
        &self,
        config: BedrockConfig,
        credentials: serde_json::Value,
    ) -> Result<(), AppError> {
        let aws_config = build_bedrock_aws_config(&config, &credentials).await?;
        let bedrock_config = aws_sdk_bedrock::config::Builder::from(&aws_config)
            .timeout_config(
                aws_config::timeout::TimeoutConfig::builder()
                    .operation_timeout(BEDROCK_TEST_TIMEOUT)
                    .build(),
            )
            .build();
        let client = aws_sdk_bedrock::Client::from_conf(bedrock_config);

        client
            .get_foundation_model()
            .model_identifier(DEFAULT_BEDROCK_TEST_MODEL)
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "Bedrock connection test failed");
                AppError::UnprocessableEntity(format!("Bedrock credentials invalid: {e}"))
            })?;

        info!("Bedrock connection test passed");
        Ok(())
    }
}

/// Validate the non-secret Bedrock metadata together with its write-only
/// typed credentials. Each auth method is fail-closed; no missing field may
/// silently select a different AWS credential chain.
pub(crate) fn validate_bedrock_auth(
    config: &BedrockConfig,
    credentials: &serde_json::Value,
) -> Result<(), AppError> {
    if config.region.trim().is_empty() {
        return Err(AppError::BadRequest("bedrock region is required".into()));
    }
    AuthMaterial {
        scheme: AuthScheme::Bedrock,
        credentials: credentials.clone(),
    }
    .validate_credentials()
    .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let object = credentials.as_object().ok_or_else(|| {
        AppError::BadRequest("bedrock credentials must be a JSON object".into())
    })?;
    match config.auth_method {
        BedrockAuthMethod::AccessKey => {
            if config.profile.is_some() {
                return Err(AppError::BadRequest(
                    "bedrock profile must be omitted for accessKey auth".into(),
                ));
            }
            if object.is_empty() {
                return Err(AppError::BadRequest(
                    "bedrock accessKey auth requires access_key_id and secret_access_key credentials"
                        .into(),
                ));
            }
        }
        BedrockAuthMethod::Profile => {
            if config
                .profile
                .as_deref()
                .map(str::trim)
                .filter(|profile| !profile.is_empty())
                .is_none()
            {
                return Err(AppError::BadRequest(
                    "bedrock profile is required for profile auth".into(),
                ));
            }
            if !object.is_empty() {
                return Err(AppError::BadRequest(
                    "bedrock profile auth requires empty credentials".into(),
                ));
            }
        }
        BedrockAuthMethod::DefaultChain => {
            if config.profile.is_some() {
                return Err(AppError::BadRequest(
                    "bedrock profile must be omitted for defaultChain auth".into(),
                ));
            }
            if !object.is_empty() {
                return Err(AppError::BadRequest(
                    "bedrock defaultChain auth requires empty credentials".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Build AWS SDK config without placing secrets in global process state.
pub(crate) async fn build_bedrock_aws_config(
    config: &BedrockConfig,
    credentials: &serde_json::Value,
) -> Result<aws_config::SdkConfig, AppError> {
    validate_bedrock_auth(config, credentials)?;
    let region = aws_config::Region::new(config.region.trim().to_owned());

    let sdk_config = match config.auth_method {
        BedrockAuthMethod::AccessKey => {
            let object = credentials.as_object().ok_or_else(|| {
                AppError::BadRequest("bedrock credentials must be a JSON object".into())
            })?;
            let access_key_id = object
                .get("access_key_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| AppError::BadRequest("access_key_id is required".into()))?;
            let secret_access_key = object
                .get("secret_access_key")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| AppError::BadRequest("secret_access_key is required".into()))?;
            let session_token = object
                .get("session_token")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let credentials = Credentials::new(
                access_key_id,
                secret_access_key,
                session_token,
                None,
                "nomifun-bedrock",
            );
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(region)
                .credentials_provider(credentials)
                .load()
                .await
        }
        BedrockAuthMethod::Profile => {
            let profile = config.profile.as_deref().ok_or_else(|| {
                AppError::BadRequest("bedrock profile is required for profile auth".into())
            })?;
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(region)
                .profile_name(profile)
                .load()
                .await
        }
        BedrockAuthMethod::DefaultChain => {
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(region)
                .load()
                .await
        }
    };
    Ok(sdk_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(auth_method: BedrockAuthMethod, profile: Option<&str>) -> BedrockConfig {
        BedrockConfig {
            auth_method,
            region: "us-east-1".into(),
            profile: profile.map(str::to_owned),
        }
    }

    #[test]
    fn access_key_accepts_only_typed_secret_credentials() {
        let access_config = config(BedrockAuthMethod::AccessKey, None);
        validate_bedrock_auth(
            &access_config,
            &json!({
                "access_key_id":"AKIA",
                "secret_access_key":"secret",
                "session_token":"session"
            }),
        )
        .unwrap();
        for invalid in [
            json!({}),
            json!({"access_key_id":"AKIA"}),
            json!({"access_key_id":"AKIA","secret_access_key":""}),
            json!({"access_key_id":"AKIA","secret_access_key":"secret","unknown":"x"}),
        ] {
            assert!(validate_bedrock_auth(&access_config, &invalid).is_err());
        }
        assert!(
            validate_bedrock_auth(
                &config(BedrockAuthMethod::AccessKey, Some("forbidden")),
                &json!({"access_key_id":"AKIA","secret_access_key":"secret"}),
            )
            .is_err()
        );
    }

    #[test]
    fn profile_and_default_chain_require_empty_credentials_without_fallback() {
        validate_bedrock_auth(
            &config(BedrockAuthMethod::Profile, Some("work")),
            &json!({}),
        )
        .unwrap();
        validate_bedrock_auth(
            &config(BedrockAuthMethod::DefaultChain, None),
            &json!({}),
        )
        .unwrap();

        assert!(
            validate_bedrock_auth(&config(BedrockAuthMethod::Profile, None), &json!({})).is_err()
        );
        assert!(
            validate_bedrock_auth(
                &config(BedrockAuthMethod::Profile, Some("work")),
                &json!({"access_key_id":"AKIA","secret_access_key":"secret"}),
            )
            .is_err()
        );
        assert!(
            validate_bedrock_auth(
                &config(BedrockAuthMethod::DefaultChain, Some("forbidden")),
                &json!({}),
            )
            .is_err()
        );
    }

    #[test]
    fn every_method_requires_nonblank_region() {
        for auth_method in [
            BedrockAuthMethod::AccessKey,
            BedrockAuthMethod::Profile,
            BedrockAuthMethod::DefaultChain,
        ] {
            let mut config = config(
                auth_method,
                (auth_method == BedrockAuthMethod::Profile).then_some("work"),
            );
            config.region = " ".into();
            let credentials = if auth_method == BedrockAuthMethod::AccessKey {
                json!({"access_key_id":"AKIA","secret_access_key":"secret"})
            } else {
                json!({})
            };
            assert!(validate_bedrock_auth(&config, &credentials).is_err());
        }
    }

    #[test]
    fn test_default_bedrock_test_model() {
        assert!(DEFAULT_BEDROCK_TEST_MODEL.starts_with("anthropic.claude"));
    }
}
