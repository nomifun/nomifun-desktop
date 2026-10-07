use nomifun_common::AppError;

/// Skill library domain errors.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Path traversal detected: {0}")]
    PathTraversal(String),

    #[error("Cannot delete built-in skill: {0}")]
    BuiltinSkillDeletion(String),

    #[error("Skill not found: {0}")]
    SkillNotFound(String),

    #[error("Invalid skill path: {0}")]
    InvalidSkillPath(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    JsonParse(#[from] serde_json::Error),
}

impl From<SkillError> for AppError {
    fn from(err: SkillError) -> Self {
        match err {
            SkillError::PathTraversal(path) => AppError::BadRequest(format!("Path traversal detected: {path}")),
            SkillError::BuiltinSkillDeletion(name) => {
                AppError::BadRequest(format!("Cannot delete built-in skill: {name}"))
            }
            SkillError::SkillNotFound(name) => AppError::NotFound(format!("Skill not found: {name}")),
            SkillError::InvalidSkillPath(path) => AppError::BadRequest(format!("Invalid skill path: {path}")),
            SkillError::Io(error) => AppError::Internal(error.to_string()),
            SkillError::JsonParse(error) => AppError::BadRequest(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_traversal_maps_to_bad_request() {
        let app_error: AppError = SkillError::PathTraversal("../secret".into()).into();
        assert!(matches!(app_error, AppError::BadRequest(_)));
    }

    #[test]
    fn missing_skill_maps_to_not_found() {
        let app_error: AppError = SkillError::SkillNotFound("missing".into()).into();
        assert!(matches!(app_error, AppError::NotFound(_)));
    }
}
