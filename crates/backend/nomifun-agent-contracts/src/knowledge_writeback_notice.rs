//! Read-only presentation of actual Knowledge writes on a Turn terminal.
//! This notice grants no authority and is not executable recovery state.
use serde::{Deserialize, Serialize};

use crate::ResourceId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeWritebackNoticeStatus { Written, Partial }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeWritebackNoticeFile {
    pub kb_id: ResourceId,
    pub rel_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeWritebackNotice {
    pub status: KnowledgeWritebackNoticeStatus,
    pub written: Vec<KnowledgeWritebackNoticeFile>,
}

impl KnowledgeWritebackNotice {
    pub fn validate(&self) -> Result<(), String> {
        if self.written.is_empty() || self.written.len() > 8 {
            return Err("Knowledge write-back notice requires bounded successful targets".into());
        }
        let mut targets = std::collections::BTreeSet::new();
        for file in &self.written {
            if uuid::Uuid::parse_str(file.kb_id.as_ref()).ok()
                .is_none_or(|id| id.get_version_num() != 7 || id.to_string() != file.kb_id.as_ref())
                || file.rel_path.is_empty() || file.rel_path.len() > 4096
                || file.rel_path.starts_with('/') || file.rel_path.contains('\\')
                || file.rel_path.chars().any(char::is_control)
                || file.rel_path.as_bytes().get(1) == Some(&b':')
                || file.rel_path.split('/').any(|part| part.is_empty() || matches!(part, "." | ".."))
                || !targets.insert((file.kb_id.as_ref(), file.rel_path.as_str())) {
                return Err("invalid Knowledge write-back notice target".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn notice() -> KnowledgeWritebackNotice {
        KnowledgeWritebackNotice {
            status: KnowledgeWritebackNoticeStatus::Written,
            written: vec![KnowledgeWritebackNoticeFile {
                kb_id: "0190f5fe-7c00-7a00-8000-000000000001".into(), rel_path: "notes/loops.md".into(),
            }],
        }
    }
    #[test]
    fn knowledge_notice_requires_real_bounded_successful_targets() {
        let mut value = notice();
        value.validate().unwrap();
        value.status = KnowledgeWritebackNoticeStatus::Partial;
        value.validate().unwrap();
        value.written.push(value.written[0].clone());
        assert!(value.validate().is_err());
        value.written.clear();
        assert!(value.validate().is_err());
    }
    #[test]
    fn knowledge_notice_rejects_unsafe_navigation_and_unrecognized_status() {
        for path in ["../other.md", "/tmp/other.md", "C:/other.md", "notes\\other.md", "notes//other.md"] {
            let mut value = notice(); value.written[0].rel_path = path.into();
            assert!(value.validate().is_err(), "{path}");
        }
        assert!(serde_json::from_value::<KnowledgeWritebackNotice>(serde_json::json!({
            "status":"extracting", "written":notice().written,
        })).is_err());
    }
}
