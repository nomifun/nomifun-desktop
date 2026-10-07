//! Turn-local routing for Plugin work.
//!
//! Two modes share one route. An explicit `plugin_delivery` requirement on
//! the accepted request creates a host-verified delivery obligation: the
//! instruction names the exact authorized `plugin.development` functions and
//! the completion gate runs. A conversation that merely owns managed drafts
//! gets neutral context instead: only the authoring functions are readied,
//! the drafts are listed as data, and its host check stays dormant until the
//! request plans a draft — no new requirement and no obligation.
//! The frozen plan is the authority; this route only names the authorized
//! function bindings for it, never widens the authorized surface.

use std::collections::BTreeSet;

use nomifun_agent_contracts::plugin::PLUGIN_ACTION_ID_PREFIX;
use nomifun_agent_runtime::AgentToolPlan;

use super::engine_session_host::ConversationPluginDraft;

const DELIVERY_HEAD: &str = "Plugin delivery task (host-verified). Deliver the accepted request as an installed NomiFun plugin / small app (插件/小程序) built with the plugin development functions below. Workspace files, code in chat or a description do not count: the host checks the installed plugin, and this task cannot complete until install succeeds for the planned outputs.\n";

const WORKFLOW: &str = concat!(
    "Workflow:\n",
    "1. Call list once; it returns the authoritative package and SDK guide.\n",
    "2. Open the working draft as described above.\n",
    "3. Call plan before editing: record outputs (one output_key for this draft; kind ui for a UI-only small app), required features and exact acceptance cases with the selectors you will implement. A persistent feature needs a UI case that runs reopen and then a text or count assertion (or a Service case with restart true). Each UI case starts from fresh preview storage, so make every UI case self-contained (create the items it checks). Add current_conversation_case only when the user explicitly asks to use the new tool in this same conversation.\n",
    "4. Call apply with complete file contents for every changed file (update the name and description in nomifun.plugin.json; implement ui/index.html and, only if needed, service/main.mjs).\n",
    "5. Call check, then preview, then run every planned case one call at a time (each call returns the revision for the next call) with exactly the planned case_name and steps via test_ui (UI) or test_action (Service). Every test_ui case restarts the preview with fresh storage, which also resets the Service storage used by test_action: for a plugin with both UI and Service, run the test_action cases first (keep each write and its restart read consecutive), then the test_ui cases. When something fails, fix it with apply and repeat check, preview and the cases; do not weaken the plan.\n",
    "6. Call install with the verification_digest returned by the last successful test, then inspect the installed plugin. Do not call test_action or test_ui after install; install itself checks the installed plugin and its UI.\n",
    "Always pass the latest revision returned by the previous plugin call as expected_revision. On an error, read the diagnostic, fix the cause and continue with these functions; do not switch to workspace files. Ask the user only for information that is genuinely missing. After install and inspect succeed, reply with a short summary of the delivered plugin.",
);

/// Deterministic order for plugin.development bindings: CREATE_ACTIONS order
/// first, then every other authorized Action by its Action suffix.
fn dev_binding_key(binding: &nomifun_agent_runtime::AgentToolBinding) -> (usize, &str) {
    let position = nomifun_plugin_development::CREATE_ACTIONS
        .iter()
        .position(|candidate| *candidate == binding.action_id.as_ref())
        .unwrap_or(usize::MAX);
    let suffix = binding
        .action_id
        .as_ref()
        .strip_prefix("plugin.development/")
        .unwrap_or_else(|| binding.action_id.as_ref());
    (position, suffix)
}

/// The plugin.development bindings on this plan in the shared route order.
fn dev_bindings(plan: &AgentToolPlan) -> Vec<&nomifun_agent_runtime::AgentToolBinding> {
    let mut bindings: Vec<_> = plan
        .model_definitions()
        .iter()
        .filter_map(|definition| plan.binding(&definition.name))
        .filter(|binding| {
            binding.capability_id.as_ref() == nomifun_plugin_development::MODULE_ID
        })
        .collect();
    bindings.sort_by(|a, b| dev_binding_key(a).cmp(&dev_binding_key(b)));
    bindings
}

/// `action = \`model_name\`` for the given plugin.development bindings.
fn action_mapping(bindings: &[&nomifun_agent_runtime::AgentToolBinding]) -> String {
    bindings
        .iter()
        .map(|binding| {
            let action = binding
                .action_id
                .as_ref()
                .rsplit('/')
                .next()
                .unwrap_or_default();
            format!("{action} = `{}`", binding.model_name)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Managed drafts as compact JSON data with host-generated fields only
/// (newest first, at most 8 entries).
fn drafts_json(drafts: &[ConversationPluginDraft]) -> String {
    let entries: Vec<serde_json::Value> = drafts
        .iter()
        .take(8)
        .map(|draft| {
            let mut entry = serde_json::json!({
                "draft_id": draft.draft_id,
                "status": if draft.delivered { "delivered" } else { "in_progress" },
            });
            if let Some(plugin_id) = &draft.plugin_id {
                entry["plugin_id"] = serde_json::json!(plugin_id);
            }
            entry
        })
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_owned())
}

/// The plugin.development module is bound on this turn's plan.
pub(super) fn has_plugin_development(plan: &AgentToolPlan) -> bool {
    plan.contains_capability(nomifun_plugin_development::MODULE_ID)
}

/// The tool names this route preactivates plus the instruction to inject.
pub(super) struct PluginTurnRoute {
    pub preactivated: Vec<String>,
    pub instruction: String,
    /// Only an explicit plugin_delivery obligation; context mode never
    /// displaces other hints.
    pub obligation: bool,
}

/// Names the authorized plugin.development functions for a Plugin turn.
/// Delivery mode requires an explicit `plugin_delivery` requirement on the
/// accepted request; otherwise conversation-owned drafts produce neutral
/// context only.
pub(super) fn route(
    plan: &AgentToolPlan,
    requirement: Option<&nomifun_api_types::PluginDeliveryRequirement>,
    drafts: &[ConversationPluginDraft],
) -> Option<PluginTurnRoute> {
    let obligation = requirement.is_some();
    if !obligation && drafts.is_empty() {
        return None;
    }
    let mut bindings = dev_bindings(plan);
    // Context mode keeps only the authoring workflow ready; management
    // Actions stay under the normal presentation policy so an unrelated
    // follow-up does not carry the whole module.
    if !obligation {
        bindings.retain(|binding| {
            nomifun_plugin_development::CREATE_ACTIONS.contains(&binding.action_id.as_ref())
        });
    }
    if bindings.is_empty() {
        return None;
    }
    let mapping = action_mapping(&bindings);
    let mut preactivated: Vec<String> = bindings
        .iter()
        .map(|binding| binding.model_name.clone())
        .collect();
    // Installed Plugin Actions owned by this conversation are bound on the
    // plan; preactivate them so a follow-up can call the delivered tool
    // without a discovery round-trip. Other plugins' bindings stay deferred.
    let mut seen: BTreeSet<String> = preactivated.iter().cloned().collect();
    let mut plugin_ids: Vec<&str> = Vec::new();
    for draft in drafts {
        if let Some(plugin_id) = draft.plugin_id.as_deref()
            && !plugin_ids.contains(&plugin_id)
        {
            plugin_ids.push(plugin_id);
        }
    }
    for plugin_id in plugin_ids {
        let prefix = format!("{PLUGIN_ACTION_ID_PREFIX}{plugin_id}/");
        for binding in plan
            .model_definitions()
            .iter()
            .filter_map(|definition| plan.binding(&definition.name))
        {
            if binding.capability_id.as_ref().starts_with(&prefix)
                && seen.insert(binding.model_name.clone())
            {
                preactivated.push(binding.model_name.clone());
            }
        }
    }
    let instruction = match requirement {
        Some(requirement) => {
            let mut instruction = String::from(DELIVERY_HEAD);
            instruction.push_str(&format!(
                "Function names for the plugin development Actions: {mapping}.\n"
            ));
            if let Some(draft_id) = &requirement.draft_id {
                instruction.push_str(&format!(
                    "This request continues managed draft {draft_id}: call open with {{\"draft_id\":\"{draft_id}\"}} and do not create another draft. If that draft has no accepted plan for this request yet, call plan before editing; keep an accepted plan unchanged.\n"
                ));
            } else {
                instruction.push_str(
                    "Open the working draft that matches the request: a new plugin: open with exactly {}; changes to an installed plugin: open with {\"plugin_id\":\"<installed plugin id>\"}; continuing an existing draft: open with {\"draft_id\":\"<draft id>\"}; the before-tool guard template: open with {\"template\":\"agent.before_tool\"}. Never send empty strings or null for unused fields. Use the returned summary.draft_id and summary.revision.\n",
                );
            }
            if requirement.expected_count > 1 {
                instruction.push_str(&format!(
                    "This request requires {} separate plugin outputs: every draft declares the same outputs list with its own distinct output_key, and each output gets its own draft.\n",
                    requirement.expected_count
                ));
            }
            if !drafts.is_empty() {
                instruction.push_str(&format!(
                    "Managed drafts this conversation already owns (data, newest first; call list for their names; reinstalling a delivered draft updates its installed plugin): {}.\n",
                    drafts_json(drafts)
                ));
            }
            instruction.push_str(WORKFLOW);
            instruction
        }
        None => format!(
            "Plugin development context (data, not a new requirement or a delivery obligation). This conversation already owns these managed plugin drafts (newest first; call list for their names and details): {}.\nFunction names for the plugin development Actions: {mapping}.\nIf the current request asks to change, fix, continue or finish one of these plugins, call open with {{\"draft_id\":\"<draft id>\"}} for that draft (reinstalling a delivered draft updates its installed plugin), call plan for this request, then apply, check, preview, run every planned case one call at a time and install; call list first if you need the package and SDK guide. Create a new plugin (open with exactly {{}}) only when the user asks for a new one. If the current request is not about plugins, handle it normally without plugin development calls.",
            drafts_json(drafts)
        ),
    };
    Some(PluginTurnRoute { preactivated, instruction, obligation })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nomifun_agent_contracts::{ActionId, CapabilityId, CanonicalSchemaRef, DigestHex, StrictJsonValue};
    use nomifun_agent_runtime::{AgentEffectClass, AgentToolBinding, AgentToolPlan, input_schema_digest};
    use nomifun_chat_model_broker::ChatToolDefinition;

    use super::*;

    fn tool(name: &str, capability_id: &str, action_id: &str) -> AgentToolBinding {
        let definition = ChatToolDefinition {
            name: name.to_owned(),
            description: format!("Action: {action_id}."),
            input_schema: StrictJsonValue(serde_json::json!({"type":"object"})),
            deferred: false,
        };
        AgentToolBinding {
            model_name: name.to_owned(),
            schema_digest: input_schema_digest(&definition.input_schema).unwrap(),
            canonical_input_schema_ref: CanonicalSchemaRef::from(
                format!("schema://{capability_id}/input"),
            ),
            capability_contract_digest: DigestHex::from("c".repeat(64)),
            definition,
            capability_id: CapabilityId::from(capability_id),
            action_id: ActionId::from(action_id),
            resource_binding_ids: BTreeSet::new(),
            effect_class: AgentEffectClass::ManagedEffect,
            parallel_safe: false,
        }
    }

    fn plugin_dev_tool(name: &str, action_id: &str) -> AgentToolBinding {
        tool(name, nomifun_plugin_development::MODULE_ID, action_id)
    }

    fn plugin_tool(name: &str, plugin_id: &str, action_id: &str) -> AgentToolBinding {
        let stable = format!("{PLUGIN_ACTION_ID_PREFIX}{plugin_id}/{action_id}");
        let mut binding = tool(name, &stable, &stable);
        binding.definition.description = format!("Action: {stable}.");
        binding
    }

    fn plugin_dev_plan() -> AgentToolPlan {
        AgentToolPlan::new(nomifun_plugin_development::CREATE_ACTIONS.iter().map(|action| {
            let name = action.rsplit('/').next().unwrap_or_default();
            plugin_dev_tool(&format!("dev_{name}"), action)
        }))
        .unwrap()
    }

    fn draft(draft_id: &str, plugin_id: Option<&str>, delivered: bool) -> ConversationPluginDraft {
        ConversationPluginDraft {
            draft_id: draft_id.into(),
            plugin_id: plugin_id.map(str::to_owned),
            delivered,
        }
    }

    fn requirement(draft_id: Option<&str>, expected_count: u8) -> nomifun_api_types::PluginDeliveryRequirement {
        nomifun_api_types::PluginDeliveryRequirement {
            draft_id: draft_id.map(str::to_owned),
            expected_count,
        }
    }

    #[test]
    fn no_route_without_plugin_development_or_context() {
        let plan = AgentToolPlan::new([tool("read", "workspace.files", "workspace.files/read")]).unwrap();
        assert!(route(&plan, Some(&requirement(None, 1)), &[]).is_none());
        let plan = plugin_dev_plan();
        assert!(route(&plan, None, &[]).is_none());
    }

    #[test]
    fn delivery_mode_documents_every_open_form() {
        let plan = AgentToolPlan::new(
            nomifun_plugin_development::CREATE_ACTIONS
                .iter()
                .map(|action| {
                    let name = action.rsplit('/').next().unwrap_or_default();
                    plugin_dev_tool(&format!("dev_{name}"), action)
                })
                .chain(std::iter::once(plugin_dev_tool(
                    "dev_delete",
                    "plugin.development/delete",
                ))),
        )
        .unwrap();
        let route = route(&plan, Some(&requirement(None, 1)), &[]).unwrap();
        assert!(route.obligation);
        assert!(route.instruction.contains("host-verified"));
        assert!(route.instruction.contains("open with exactly {}"));
        assert!(route.instruction.contains(r#"{"plugin_id":"<installed plugin id>"}"#));
        assert!(route.instruction.contains(r#"{"draft_id":"<draft id>"}"#));
        assert!(route.instruction.contains(r#"{"template":"agent.before_tool"}"#));
        assert!(!route.instruction.contains("Do not pass draft_id"));
        assert!(route.instruction.contains("cannot complete until install succeeds"));
        assert!(route.instruction.contains("Workflow:"));
        for action in ["list", "open", "plan", "apply", "check", "preview", "test_ui", "test_action", "install", "inspect"] {
            assert!(route.instruction.contains(&format!("{action} = `dev_{action}`")), "{action}");
        }
        // The mapping and preactivation follow CREATE_ACTIONS order, with
        // every other authorized Action after all create Actions.
        let list = route.instruction.find("list = `").unwrap();
        let open = route.instruction.find("open = `").unwrap();
        let install = route.instruction.find("install = `").unwrap();
        assert!(list < open && open < install, "{}", route.instruction);
        let delete = route.instruction.find("delete = `dev_delete`").unwrap();
        let inspect = route.instruction.find("inspect = `dev_inspect`").unwrap();
        assert!(inspect < delete, "non-create Actions sort after the create Actions");
        assert_eq!(route.preactivated.last().unwrap(), "dev_delete");
    }

    #[test]
    fn delivery_mode_with_draft_id_targets_that_draft() {
        let plan = plugin_dev_plan();
        let route = route(&plan, Some(&requirement(Some("draft-42"), 1)), &[]).unwrap();
        assert!(route.instruction.contains("continues managed draft draft-42"));
        assert!(route.instruction.contains(r#"{"draft_id":"draft-42"}"#));
        assert!(route.instruction.contains("do not create another draft"));
        assert!(!route.instruction.contains("a new plugin: open with exactly {}"));
        assert!(!route.instruction.contains("\"template\":\"agent.before_tool\""));
        assert!(!route.instruction.contains("Do not pass draft_id"));
    }

    #[test]
    fn delivery_mode_lists_existing_drafts_as_data() {
        let plan = plugin_dev_plan();
        let drafts = vec![
            draft("d-new", Some("plug-1"), false),
            draft("d-old", None, true),
        ];
        let route = route(&plan, Some(&requirement(None, 1)), &drafts).unwrap();
        assert!(route.instruction.contains("Managed drafts this conversation already owns"));
        assert!(route.instruction.contains(r#""draft_id":"d-new""#));
        assert!(route.instruction.contains(r#""plugin_id":"plug-1""#));
        assert!(route.instruction.contains(r#""status":"in_progress""#));
        assert!(route.instruction.contains(r#""status":"delivered""#));
        let new = route.instruction.find("d-new").unwrap();
        let old = route.instruction.find("d-old").unwrap();
        assert!(new < old, "drafts are listed newest first");
        // plugin_id is omitted entirely for drafts that target no installed plugin.
        assert!(route.instruction.contains(r#"{"draft_id":"d-old","status":"delivered"}"#));
    }

    #[test]
    fn multi_output_requirement_adds_the_outputs_clause() {
        let plan = plugin_dev_plan();
        let route = route(&plan, Some(&requirement(None, 2)), &[]).unwrap();
        assert!(route.instruction.contains("requires 2 separate plugin outputs"));
        assert!(route.instruction.contains("distinct output_key"));
    }

    #[test]
    fn context_mode_is_neutral_context_not_an_obligation() {
        let plan = AgentToolPlan::new(
            nomifun_plugin_development::CREATE_ACTIONS
                .iter()
                .map(|action| {
                    let name = action.rsplit('/').next().unwrap_or_default();
                    plugin_dev_tool(&format!("dev_{name}"), action)
                })
                .chain(std::iter::once(plugin_dev_tool(
                    "dev_delete",
                    "plugin.development/delete",
                ))),
        )
        .unwrap();
        let drafts = vec![draft("d-1", Some("plug-1"), false)];
        let route = route(&plan, None, &drafts).unwrap();
        assert!(!route.obligation);
        assert!(route.instruction.contains("Plugin development context"));
        assert!(route.instruction.contains("not a new requirement or a delivery obligation"));
        assert!(!route.instruction.contains("cannot complete until install succeeds"));
        assert!(route.instruction.contains(r#""draft_id":"d-1""#));
        assert!(route.instruction.contains(r#"{"draft_id":"<draft id>"}"#));
        assert!(route.instruction.contains("not about plugins, handle it normally"));
        // Only the authoring workflow is readied in context mode; management
        // Actions stay under the normal presentation policy.
        for action in nomifun_plugin_development::CREATE_ACTIONS {
            let name = action.rsplit('/').next().unwrap_or_default();
            assert!(route.preactivated.contains(&format!("dev_{name}")), "{action}");
        }
        assert!(!route.preactivated.contains(&"dev_delete".to_owned()));
        assert!(!route.instruction.contains("delete = `"));
        assert!(route.instruction.contains("call list for their names"));
    }

    #[test]
    fn drafts_context_carries_only_host_generated_fields() {
        let parsed: serde_json::Value =
            serde_json::from_str(&drafts_json(&[draft("d-1", Some("plug-1"), false)])).unwrap();
        assert_eq!(parsed, serde_json::json!([{
            "draft_id": "d-1", "status": "in_progress", "plugin_id": "plug-1"
        }]));
    }

    #[test]
    fn preactivation_orders_dev_actions_then_this_conversations_plugin_tools() {
        let plan = AgentToolPlan::new([
            plugin_dev_tool("dev_install", "plugin.development/install"),
            plugin_dev_tool("dev_open", "plugin.development/open"),
            plugin_dev_tool("dev_list", "plugin.development/list"),
            plugin_tool("plug_b_run", "plug-b", "run"),
            plugin_tool("plug_a_run", "plug-a", "run"),
        ])
        .unwrap();
        let before = serde_json::to_string(&plan.model_definitions()).unwrap();
        let drafts = vec![draft("d-1", Some("plug-a"), true)];
        let route = route(&plan, Some(&requirement(None, 1)), &drafts).unwrap();
        assert_eq!(
            route.preactivated,
            vec!["dev_list", "dev_open", "dev_install", "plug_a_run"]
        );
        let after = serde_json::to_string(&plan.model_definitions()).unwrap();
        assert_eq!(before, after, "the frozen plan is untouched");
    }

    #[test]
    fn instruction_stays_within_the_route_budget() {
        let plan = plugin_dev_plan();
        let drafts: Vec<ConversationPluginDraft> = (0..32)
            .map(|index| {
                draft(
                    &format!("draft-{index}"),
                    Some("plug-x"),
                    index % 2 == 0,
                )
            })
            .collect();
        for requirement in [
            None,
            Some(requirement(None, 2)),
            Some(requirement(Some("draft-0"), 1)),
        ] {
            let route = route(&plan, requirement.as_ref(), &drafts).unwrap();
            assert!(
                route.instruction.len() <= 8 * 1024,
                "{} bytes exceeds the route budget",
                route.instruction.len()
            );
        }
    }
}
