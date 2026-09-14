//! Optional caller-observed source preconditions. These are checked against
//! bounded source bytes before any file in the patch request is published.
use nomifun_common::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentSessionPatchSource {
    /// Compatibility with existing line-only patches, not a version claim.
    #[default]
    Any,
    Existing {
        sha256: String,
    },
    Absent,
}

impl AgentSessionPatchSource {
    pub(crate) fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    pub(crate) fn validate(&self) -> Result<(), AppError> {
        if let Self::Existing { sha256 } = self
            && (sha256.len() != 64
                || !sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        {
            return Err(AppError::BadRequest(
                "patch expected_source requires a lowercase 64-character SHA-256".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn check(&self, existed: bool, source: &[u8]) -> Result<(), AppError> {
        self.validate()?;
        let matches = match self {
            Self::Any => true,
            Self::Absent => !existed,
            Self::Existing { sha256 } => {
                existed && format!("{:x}", Sha256::digest(source)) == *sha256
            }
        };
        if !matches {
            // Neither source text nor expected/actual digests belong in an
            // error receipt. The caller adds only the scoped target identity.
            return Err(AppError::Conflict(
                "patch source precondition changed; re-read the target and reconsider the patch, do not retry by removing the guard".into(),
            ));
        }
        Ok(())
    }
}
