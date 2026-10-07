use nomifun_common::AppError;
use serde::Serialize;

/// Canonical AutoWork configuration shared by REST, Gateway, boot recovery,
/// and the live runner.
///
/// Tags are normalized exactly once at the command boundary. Persisted values
/// are parsed through the same rules, so a restart cannot silently change the
/// binding identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AutoWorkConfig {
    pub enabled: bool,
    pub tag: Option<String>,
    pub max_requirements: Option<u32>,
}

impl AutoWorkConfig {
    pub fn normalize(
        enabled: bool,
        tag: Option<&str>,
        max_requirements: Option<u32>,
    ) -> Result<Self, AppError> {
        let tag = match tag {
            Some(raw) => {
                let normalized = raw.trim();
                if normalized.is_empty() {
                    return Err(AppError::BadRequest(
                        "AutoWork tag must not be empty or whitespace".to_owned(),
                    ));
                }
                Some(normalized.to_owned())
            }
            None => None,
        };
        if enabled && tag.is_none() {
            return Err(AppError::BadRequest(
                "tag is required when enabling autowork".to_owned(),
            ));
        }
        Ok(Self {
            enabled,
            tag,
            max_requirements,
        })
    }

    pub fn enabled_tag(&self) -> Result<&str, AppError> {
        if !self.enabled {
            return Err(AppError::Conflict(
                "AutoWork cannot start from a disabled configuration".to_owned(),
            ));
        }
        self.tag.as_deref().ok_or_else(|| {
            AppError::Conflict("enabled AutoWork configuration has no tag".to_owned())
        })
    }

}

/// Owner-scoped persisted AutoWork configuration plus its optimistic
/// concurrency token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoWorkConfigSnapshot {
    pub config: AutoWorkConfig,
    pub revision: String,
    pub operation_id: Option<String>,
}

impl AutoWorkConfigSnapshot {
    pub fn new(
        config: AutoWorkConfig,
        revision: impl Into<String>,
        operation_id: Option<String>,
    ) -> Result<Self, AppError> {
        let revision = revision.into();
        if revision.trim().is_empty() {
            return Err(AppError::Conflict(
                "AutoWork config revision must not be empty".to_owned(),
            ));
        }
        if operation_id
            .as_deref()
            .is_some_and(|operation_id| operation_id.trim().is_empty())
        {
            return Err(AppError::Conflict(
                "AutoWork config operation identity must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            config,
            revision,
            operation_id,
        })
    }
}

/// Atomic Session-side config mutation requested by Requirement.
///
/// The host must verify `owner_id`, compare `expected_revision`, and merge only
/// its AutoWork-owned metadata in one storage transaction. Replaying the same
/// `operation_id` with a different `config` must return `Conflict`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoWorkSessionConfigCommand {
    pub owner_id: String,
    pub session_id: String,
    pub config: AutoWorkConfig,
    pub expected_revision: String,
    pub operation_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_normalization_trims_once_and_rejects_blank_tags() {
        assert_eq!(
            AutoWorkConfig::normalize(true, Some("  release  "), Some(3)).unwrap(),
            AutoWorkConfig {
                enabled: true,
                tag: Some("release".to_owned()),
                max_requirements: Some(3),
            }
        );
        assert!(AutoWorkConfig::normalize(true, Some(" \t "), None).is_err());
        assert!(AutoWorkConfig::normalize(false, Some(" \n "), None).is_err());
        assert!(AutoWorkConfig::normalize(true, None, None).is_err());
    }

    #[test]
    fn snapshot_rejects_blank_revision_and_operation_identity() {
        let config = AutoWorkConfig::default();
        assert!(AutoWorkConfigSnapshot::new(config.clone(), "", None).is_err());
        assert!(
            AutoWorkConfigSnapshot::new(config, "session:1", Some("  ".to_owned())).is_err()
        );
    }
}
