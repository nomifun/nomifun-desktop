//! Exact, host-resolved Skill content consumed by the ordinary Skill tool.
//! No filesystem search, package registry or implicit process authority lives here.
use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use nomi_skills::frontmatter::{parse_frontmatter_strict, parse_skill_fields};
use nomi_skills::types::{LoadedFrom, SkillMetadata, SkillSource};

#[async_trait]
pub trait HostSkillAccess: Send + Sync {
    /// Revalidate the owning host's frozen source and current revocation state.
    async fn authorize(&self) -> Result<(), String>;
}

pub struct HostSkill {
    metadata: SkillMetadata,
    resources: BTreeMap<String, Vec<u8>>,
    access: Arc<dyn HostSkillAccess>,
}

impl HostSkill {
    pub fn read_only(
        id: &str,
        description: &str,
        markdown: &str,
        resources: BTreeMap<String, Vec<u8>>,
        access: Arc<dyn HostSkillAccess>,
    ) -> Result<Self, String> {
        let parsed = parse_frontmatter_strict(markdown)?;
        let fields = &parsed.frontmatter;
        if fields
            .context
            .as_deref()
            .is_some_and(|value| value != "inline")
            || fields.hooks.is_some()
            || fields.agent.is_some()
            || fields.model.is_some()
            || fields.effort.is_some()
            || fields.allowed_tools.is_some()
            || nomi_skills::shell::has_shell_commands(&parsed.content, LoadedFrom::Skills)
        {
            return Err(format!(
                "Skill {id} requires an execution mode not yet supported by the hosted read-only loader (shell/fork/hooks/model/tool overrides)"
            ));
        }
        let mut metadata = parse_skill_fields(
            fields,
            &parsed.content,
            id,
            SkillSource::Managed,
            LoadedFrom::Managed,
            None,
        );
        // The catalog's exact identity is the invocation key, not an optional
        // display name from frontmatter or a same-named directory.
        if metadata.description.is_empty() {
            metadata.description = description.to_owned();
        }
        Ok(Self {
            metadata,
            resources,
            access,
        })
    }

    pub fn metadata(&self) -> &SkillMetadata {
        &self.metadata
    }

    /// The host already supplies the body through its selected-context path.
    /// Keep explicit slash invocation without advertising a second model loader.
    pub fn command_only(mut self) -> Self {
        self.metadata.disable_model_invocation = true;
        self
    }

    pub fn command_name(&self) -> String {
        format!("skill:{}", self.metadata.name)
    }

    pub async fn read(
        &self,
        args: Option<&str>,
        resource: Option<&str>,
        session: Option<&str>,
    ) -> Result<String, String> {
        self.access.authorize().await?;
        if let Some(path) = resource {
            let bytes = self.resources.get(path).ok_or_else(|| {
                format!(
                    "Skill {} has no declared resource {path}",
                    self.metadata.name,
                )
            })?;
            return Ok(match std::str::from_utf8(bytes) {
                Ok(text) => {
                    serde_json::json!({"path":path,"encoding":"utf-8","content":text}).to_string()
                }
                Err(_) => {
                    use std::fmt::Write;
                    let mut hex = String::with_capacity(bytes.len() * 2);
                    for byte in bytes {
                        write!(&mut hex, "{byte:02x}").expect("writing to a String");
                    }
                    serde_json::json!({"path":path,"encoding":"hex","content":hex}).to_string()
                }
            });
        }
        let mut content = nomi_skills::substitution::substitute_arguments(
            &self.metadata.content,
            args,
            &self.metadata.argument_names,
            None,
            session,
        );
        // Arguments are data too: never pass substituted hosted content to the
        // standalone shell executor, even if they introduce command syntax.
        if nomi_skills::shell::has_shell_commands(&content, LoadedFrom::Skills) {
            return Err("hosted read-only Skill arguments cannot introduce shell execution".into());
        }
        if !self.resources.is_empty() {
            content.push_str(
                "\n\nDeclared package resources (read using the Skill tool's resource argument):\n",
            );
            for path in self.resources.keys() {
                content.push_str(&format!("- {path}\n"));
            }
        }
        Ok(content)
    }
}

/// Explicit host selections take precedence over incidental directory/MCP
/// names. Discovery remains available for the non-hosted Skill path.
pub fn merge_host_skills(discovered: &mut Vec<SkillMetadata>, hosted: &[Arc<HostSkill>]) {
    discovered.retain(|skill| !hosted.iter().any(|value| value.metadata.name == skill.name));
    discovered.extend(hosted.iter().map(|value| value.metadata.clone()));
}

/// The command and model tool share the exact descriptor and deny policy.
/// This adapter only returns instructions; it owns neither a Session nor an
/// executor and cannot interpret embedded shell or invoke another Agent.
pub(crate) struct HostSkillCommand {
    pub name: String,
    pub skill: Arc<HostSkill>,
    pub checker: Arc<nomi_skills::permissions::SkillPermissionChecker>,
    pub session_id: Option<String>,
}

#[async_trait]
impl crate::commands::SlashCommand for HostSkillCommand {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.skill.metadata.description
    }

    fn is_visible(&self) -> bool {
        self.skill.metadata.user_invocable
            && self.checker.check(&self.skill.metadata)
                == nomi_skills::permissions::SkillPermission::Allow
    }

    fn required_tool(&self) -> Option<&str> {
        Some("Skill")
    }

    async fn execute(
        &self,
        _ctx: &mut crate::commands::CommandContext<'_>,
        args: &str,
    ) -> anyhow::Result<crate::commands::CommandResult> {
        anyhow::ensure!(
            self.skill.metadata.user_invocable,
            "Skill {} is not user-invocable",
            self.name()
        );
        anyhow::ensure!(
            self.checker.check(&self.skill.metadata)
                == nomi_skills::permissions::SkillPermission::Allow,
            "Skill {} is denied by configuration",
            self.name()
        );
        let content = self
            .skill
            .read(Some(args), None, self.session_id.as_deref())
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(crate::commands::CommandResult::Prompt(format!(
            "Selected Skill {}:\n{content}",
            self.name()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_skills::permissions::SkillPermissionChecker;
    use nomi_tools::Tool;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Access(AtomicBool);
    #[async_trait]
    impl HostSkillAccess for Access {
        async fn authorize(&self) -> Result<(), String> {
            if self.0.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err("source withdrawn".into())
            }
        }
    }
    fn access() -> Arc<Access> {
        Arc::new(Access(AtomicBool::new(true)))
    }

    #[tokio::test]
    async fn exact_skill_uses_standard_tool_and_revalidates_every_read() {
        let guard = access();
        let hosted = Arc::new(
            HostSkill::read_only(
                "pkg.guide",
                "fallback",
                "---\nname: display-only\ndescription: exact description\n---\nEXACT $ARGUMENTS",
                BTreeMap::from([("resources/binary".into(), vec![0xff, 0x00])]),
                guard.clone(),
            )
            .unwrap(),
        );
        let old = nomi_skills::frontmatter::parse_skill_fields(
            &Default::default(),
            "WRONG DIRECTORY BODY",
            "pkg.guide",
            SkillSource::Managed,
            LoadedFrom::Managed,
            None,
        );
        let mut discovered = vec![old];
        merge_host_skills(&mut discovered, &[hosted.clone()]);
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "pkg.guide");
        let tool = crate::skill_tool::SkillTool::new(
            Arc::new(discovered),
            ".".into(),
            SkillPermissionChecker::new(vec![]),
        )
        .with_host_skills(&[hosted]);
        let body = tool
            .execute(json!({"skill":"/pkg.guide", "args":"hello"}))
            .await;
        assert!(!body.is_error, "{}", body.content);
        assert!(body.content.contains("EXACT hello"));
        assert!(!body.content.contains("WRONG DIRECTORY"));
        let binary = tool
            .execute(json!({"skill":"pkg.guide", "resource":"resources/binary"}))
            .await;
        assert!(!binary.is_error);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&binary.content).unwrap()["content"],
            "ff00"
        );
        for resource in [json!("../secret"), json!(7)] {
            assert!(
                tool.execute(json!({"skill":"pkg.guide", "resource":resource}))
                    .await
                    .is_error
            );
        }
        guard.0.store(false, Ordering::SeqCst);
        let denied = tool.execute(json!({"skill":"pkg.guide"})).await;
        assert!(denied.is_error);
        assert!(denied.content.contains("withdrawn"));
        assert!(
            tool.execute(json!({"skill":"pkg.guide", "resource":"resources/binary"}))
                .await
                .is_error
        );
    }

    #[tokio::test]
    async fn malformed_and_executable_metadata_never_silently_downgrade() {
        for markdown in [
            "---\ncontext: fork\n---\nbody",
            "---\nhooks: {}\n---\nbody",
            "---\nmodel: other\n---\nbody",
            "---\nallowed-tools: Bash\n---\nbody",
            "---\nagent: other\n---\nbody",
            "---\ncontext: [\n---\nbody",
            "---\nunterminated",
            "Run !`echo danger`",
        ] {
            assert!(
                HostSkill::read_only("pkg.guide", "", markdown, BTreeMap::new(), access()).is_err(),
                "{markdown}"
            );
        }
        for markdown in ["plain body", "---\n\n---\nbody", "---\r\n\r\n---\r\nbody"] {
            assert!(
                HostSkill::read_only("pkg.guide", "", markdown, BTreeMap::new(), access()).is_ok()
            );
        }
        let skill =
            HostSkill::read_only("pkg.guide", "", "$ARGUMENTS", BTreeMap::new(), access()).unwrap();
        assert!(
            skill
                .read(Some("!`echo danger`"), None, None)
                .await
                .is_err()
        );
    }
}
