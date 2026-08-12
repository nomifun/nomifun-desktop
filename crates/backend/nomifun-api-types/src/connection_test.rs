use serde::Deserialize;

use super::provider::BedrockConfig;

/// Request body for `POST /api/bedrock/test-connection`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestBedrockConnectionRequest {
    pub bedrock_config: BedrockConfig,
    pub credentials: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- TestBedrockConnectionRequest --

    #[test]
    fn test_bedrock_request_access_key() {
        let raw = json!({
            "bedrock_config": {
                "auth_method": "accessKey",
                "region": "us-east-1"
            },
            "credentials": {
                "access_key_id": "AKIAIOSFODNN7",
                "secret_access_key": "wJalrXUtnFEMI",
                "session_token": "optional-token"
            }
        });
        let req: TestBedrockConnectionRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(
            req.bedrock_config.auth_method,
            crate::BedrockAuthMethod::AccessKey
        );
        assert_eq!(req.bedrock_config.region, "us-east-1");
        assert_eq!(req.credentials["access_key_id"], "AKIAIOSFODNN7");
    }

    #[test]
    fn test_bedrock_request_profile() {
        let raw = json!({
            "bedrock_config": {
                "auth_method": "profile",
                "region": "eu-west-1",
                "profile": "my-profile"
            },
            "credentials": {}
        });
        let req: TestBedrockConnectionRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(
            req.bedrock_config.auth_method,
            crate::BedrockAuthMethod::Profile
        );
        assert_eq!(req.bedrock_config.profile.as_deref(), Some("my-profile"));
    }

    #[test]
    fn test_bedrock_request_missing_config() {
        let raw = json!({});
        let result = serde_json::from_value::<TestBedrockConnectionRequest>(raw);
        assert!(result.is_err());
    }

    #[test]
    fn test_bedrock_request_requires_explicit_credentials() {
        let result = serde_json::from_value::<TestBedrockConnectionRequest>(json!({
            "bedrock_config": {
                "auth_method": "defaultChain",
                "region": "us-east-1"
            }
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_bedrock_request_missing_region() {
        let raw = json!({
            "bedrock_config": {
                "auth_method": "accessKey"
            },
            "credentials": {
                "access_key_id": "AKIA...",
                "secret_access_key": "secret"
            }
        });
        let result = serde_json::from_value::<TestBedrockConnectionRequest>(raw);
        assert!(result.is_err());
    }

    #[test]
    fn bedrock_config_rejects_secret_fields_and_accepts_default_chain() {
        assert!(
            serde_json::from_value::<TestBedrockConnectionRequest>(json!({
                "bedrock_config": {
                    "auth_method": "accessKey",
                    "region": "us-east-1",
                    "secret_access_key": "must-not-live-here"
                },
                "credentials": {}
            }))
            .is_err()
        );
        let request: TestBedrockConnectionRequest = serde_json::from_value(json!({
            "bedrock_config": {
                "auth_method": "defaultChain",
                "region": "us-west-2"
            },
            "credentials": {}
        }))
        .unwrap();
        assert_eq!(
            request.bedrock_config.auth_method,
            crate::BedrockAuthMethod::DefaultChain
        );
    }
}
