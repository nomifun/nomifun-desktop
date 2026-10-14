//! Test-only definitions built from the same owner schema and typed guide as
//! the platform. This is not a Runtime catalogue or a production resolver.
use nomifun_chat_model_broker::ChatToolDefinition;

pub(crate) fn tool_definition(capability_id: &str, action_id: &str) -> ChatToolDefinition {
    let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
    let capability = registration.metadata.manifest.payload.contributions.capabilities.iter()
        .find(|capability| capability.id.as_ref() == capability_id).unwrap();
    let reference = &capability.contributions.actions.iter()
        .find(|action| action.action_id.as_ref() == action_id).unwrap().input_schema;
    let canonical = nomifun_agent_domain_wave2::resolve_action_schema(capability_id, reference).unwrap();
    let guide = nomifun_agent_contracts::tool_model_guide::tool_model_guide(capability_id, action_id).unwrap();
    let rendered = guide.render(&canonical, std::env::consts::OS).unwrap();
    ChatToolDefinition {
        name: nomifun_agent_contracts::tool_presentation::platform_tool_name(capability_id, action_id),
        description: rendered.description,
        input_schema: rendered.input_schema,
        deferred: false,
    }
}
