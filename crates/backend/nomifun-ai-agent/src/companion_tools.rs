//! Learned companion skill access through the host-owned skill store.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use nomi_types::tool::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};

/// 一个可调用技能的精简描述，用于每轮的 when_to_use 索引注入。
#[derive(Debug, Clone)]
pub struct SkillListing {
    pub name: String,
    pub when_to_use: String,
}

/// Backend seam for the companion's self-evolved skills. Implemented by
/// `nomifun-companion` over its store + `skill_service`; the Runtime depends
/// on this (engine stays host-agnostic).
#[async_trait]
pub trait CompanionSkillSink: Send + Sync {
    /// This companion's currently-active skills (for per-turn `when_to_use` injection).
    /// Must be cheap — called once per turn.
    async fn active_skills(&self) -> Vec<SkillListing>;
    /// The SKILL.md body of a named active skill, or `None` if unknown.
    async fn load_skill_body(&self, name: &str) -> Option<String>;
}

/// `companion_skill` — invoke a learned skill by name to fetch its playbook.
pub struct CompanionSkillTool {
    sink: Arc<dyn CompanionSkillSink>,
}

impl CompanionSkillTool {
    pub fn new(sink: Arc<dyn CompanionSkillSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl Tool for CompanionSkillTool {
    fn name(&self) -> &str {
        "companion_skill"
    }

    fn description(&self) -> &str {
        "调用你已学会的某个技能，获取它的操作手册（步骤），然后照着执行。\
         当当前任务匹配系统提示里列出的某个技能的适用场景时使用。"
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "skill": {"type": "string", "description": "技能名（见系统提示里列出的可用技能）"}
            },
            "required": ["skill"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let name = input.get("skill").and_then(|v| v.as_str()).unwrap_or("").trim();
        if name.is_empty() {
            return ToolResult {
                content: "skill 不能为空".into(),
                is_error: true,
                images: Vec::new(),
            };
        }
        match self.sink.load_skill_body(name).await {
            Some(body) => ToolResult {
                content: body,
                is_error: false,
                images: Vec::new(),
            },
            None => ToolResult {
                content: format!("未找到技能：{name}"),
                is_error: true,
                images: Vec::new(),
            },
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

/// 每轮把该伙伴 active 技能的 `when_to_use` 索引注入系统提示（design §7）。
/// 空技能集 → `None`（no-op 快路，引擎据此每轮零成本跳过）。
pub struct CompanionSkillContributor {
    sink: Arc<dyn CompanionSkillSink>,
}

impl CompanionSkillContributor {
    pub fn new(sink: Arc<dyn CompanionSkillSink>) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl crate::context_contributor::ContextContributor for CompanionSkillContributor {
    async fn pre_turn_context(&self) -> Option<String> {
        let skills = self.sink.active_skills().await;
        if skills.is_empty() {
            return None;
        }
        let mut s = String::from(
            "<system-reminder>\n你已经学会以下技能。遇到匹配场景时，用 companion_skill 工具按名调用以获取操作手册并照做：\n",
        );
        for sk in &skills {
            s.push_str(&format!("- {}: {}\n", sk.name, sk.when_to_use));
        }
        s.push_str("</system-reminder>");
        Some(s)
    }

    fn label(&self) -> &str {
        "companion_skills"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_contributor::ContextContributor;

    struct FakeSkillSink {
        skills: Vec<SkillListing>,
    }
    #[async_trait]
    impl CompanionSkillSink for FakeSkillSink {
        async fn active_skills(&self) -> Vec<SkillListing> {
            self.skills.clone()
        }
        async fn load_skill_body(&self, name: &str) -> Option<String> {
            self.skills.iter().find(|s| s.name == name).map(|s| format!("# {}\nbody", s.name))
        }
    }

    #[tokio::test]
    async fn skill_contributor_is_noop_when_empty() {
        let sink = Arc::new(FakeSkillSink { skills: vec![] });
        let c = CompanionSkillContributor::new(sink);
        assert!(c.pre_turn_context().await.is_none());
    }

    #[tokio::test]
    async fn skill_contributor_lists_when_to_use() {
        let sink = Arc::new(FakeSkillSink {
            skills: vec![SkillListing { name: "weekly-report".into(), when_to_use: "周五出周报".into() }],
        });
        let c = CompanionSkillContributor::new(sink);
        let out = c.pre_turn_context().await.unwrap();
        assert!(out.contains("weekly-report"));
        assert!(out.contains("周五出周报"));
        assert!(out.contains("companion_skill"));
    }

    #[tokio::test]
    async fn skill_tool_returns_body_or_error() {
        let sink = Arc::new(FakeSkillSink {
            skills: vec![SkillListing { name: "fmt".into(), when_to_use: "x".into() }],
        });
        let tool = CompanionSkillTool::new(sink);
        assert!(tool.execute(json!({})).await.is_error);
        let ok = tool.execute(json!({"skill": "fmt"})).await;
        assert!(!ok.is_error);
        assert!(ok.content.contains("fmt"));
        assert!(tool.execute(json!({"skill": "nope"})).await.is_error);
    }
}
