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

    /// Revalidate source authority and exact resource metadata without reading
    /// or substituting Skill content. Execution performs these checks again.
    pub async fn preflight_hook(&self, resource: Option<&str>) -> Result<(), String> {
        self.access.authorize().await?;
        if resource.is_some_and(|path| !self.resources.contains_key(path)) {
            return Err("Skill resource is not declared by the selected package".into());
        }
        Ok(())
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
