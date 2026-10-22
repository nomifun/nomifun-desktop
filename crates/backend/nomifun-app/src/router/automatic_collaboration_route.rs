//! Policy-aware routing for explicit Agent collaboration requests.
//!
//! Every interaction entry uses the same frozen Action grants and typed
//! delegation policy. Ordinary work stays in the receiving Session; durable
//! collaboration is selected for an explicit request or actual task benefit.
//!
//! Intent hints never replace the frozen tool plan or force every model step
//! into another delegation. Native collaboration receipts, not tool choice,
//! record that an explicit request has been fulfilled.

use nomifun_agent_runtime::AgentToolPlan;
use nomifun_common::DelegationPolicy;

pub(super) const COLLABORATION_CAPABILITY_ID: &str = "agent.collaboration";
pub(super) const DELEGATE_ACTION_ID: &str = "agent/delegate";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExplicitCollaborationIntent {
    MultiAgent,
    Subagent,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CollaborationTurnRoute {
    pub(super) tool_plan: AgentToolPlan,
    pub(super) instructions: Vec<String>,
    pub(super) required_delegation: Option<String>,
}

fn contains_any(input: &str, values: &[&str]) -> bool {
    values.iter().any(|value| input.contains(value))
}

fn compact(input: &str) -> String {
    input
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '-' | '_'))
        .collect()
}

fn is_directly_negated(input: &str) -> bool {
    contains_any(
        input,
        &[
            "不要使用subagent",
            "别使用subagent",
            "不用subagent",
            "无需subagent",
            "禁止subagent",
            "关闭subagent",
            "不要用subagent",
            "不要使用多agent",
            "别用多agent",
            "不要启动多agent",
            "不要开启多agent",
            "不用多agent",
            "无需多agent",
            "不要使用多代理",
            "别用多代理",
            "不用多代理",
            "无需多代理",
            "donotusesubagent",
            "don'tusesubagent",
            "withoutsubagent",
            "donotusemultiagent",
            "don'tusemultiagent",
            "withoutmultiagent",
        ],
    )
}

fn is_explanatory_request(input: &str) -> bool {
    contains_any(
        input,
        &[
            "什么是多agent",
            "什么是多代理",
            "什么是subagent",
            "如何使用subagent",
            "怎么使用subagent",
            "howtousesubagent",
            "whatisasubagent",
            "whataresubagents",
        ],
    )
}

fn has_execution_verb(normalized: &str, compacted: &str) -> bool {
    let chinese = contains_any(
        compacted,
        &[
            "使用",
            "调用",
            "启动",
            "开启",
            "创建",
            "构建",
            "设计",
            "实现",
            "运行",
            "测试",
            "派发",
            "委派",
            "分工",
            "交给",
            "让子",
            "请用",
            "帮我用",
            "排查",
            "审查",
            "解决",
        ],
    );
    let words = normalized
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let english = words.iter().any(|word| {
        matches!(
            *word,
            "use"
                | "invoke"
                | "spawn"
                | "start"
                | "launch"
                | "create"
                | "build"
                | "design"
                | "run"
                | "test"
                | "delegate"
                | "parallelize"
        )
    }) || words
        .windows(2)
        .any(|pair| matches!(pair, ["review" | "investigate", "with"]));
    chinese || english
}

fn classify(input: &str) -> Option<ExplicitCollaborationIntent> {
    let request = super::automatic_turn_intent::request_text(input)?;
    let normalized = request.trim().to_lowercase();
    let compacted = compact(&normalized);
    if compacted.is_empty()
        || is_directly_negated(&compacted)
        || is_explanatory_request(&compacted)
        || !has_execution_verb(&normalized, &compacted)
    {
        return None;
    }

    if contains_any(
        &compacted,
        &[
            "multiagent",
            "agent集群",
            "agent团队",
            "多agent",
            "多代理",
            "代理集群",
            "代理团队",
            "多智能体",
            "智能体集群",
            "agentteam",
            "multipleagents",
            "parallelagents",
        ],
    ) {
        return Some(ExplicitCollaborationIntent::MultiAgent);
    }
    contains_any(
        &compacted,
        &["subagent", "子agent", "子代理", "子智能体"],
    )
    .then_some(ExplicitCollaborationIntent::Subagent)
}

fn policy_instruction(policy: DelegationPolicy, tool_name: &str) -> String {
    let base = format!(
        "Agent collaboration is available through the advertised function `{tool_name}` whose canonical Action is `agent/delegate`. Treat an explicit current-user request to use, start, build, design, test, or run multi-agent, multiple-agent, Agent-team, subagent, 多 Agent、多代理、Agent 集群、子 Agent, or 子代理 work as an operational request to invoke that Action. For independent work submit one `strategy=parallel` request with a `tasks` JSON array of bounded, non-overlapping tasks. The parent combines child results after joining them. For a goal needing a dependency plan use `strategy=planned` and `goal`. A successful receipt proves acceptance, not task completion. Once the requested work has been accepted, do not delegate it again with fresh call IDs. Continue useful independent work with the remaining selected tools, join the exact accepted children through the native collaboration controls, then integrate and verify their results before completing the original task. Do not replace a native wait with polling, repeated delegation, or speculative completion. For an ordinary short or indivisible request, answer directly."
    );
    match policy {
        DelegationPolicy::Automatic => base,
        DelegationPolicy::PreferParallel => format!(
            "{base}\n\nThis Session prefers parallel delegation. Deliberately check every substantive request for independent workstreams and prefer `agent/delegate` when parallel execution materially improves speed, coverage, or focused context. Do not manufacture duplicate tasks merely to create a multi-Agent execution."
        ),
        DelegationPolicy::Disabled => unreachable!("disabled policy has a separate instruction"),
    }
}

fn explicit_instruction(intent: ExplicitCollaborationIntent, tool_name: &str) -> String {
    match intent {
        ExplicitCollaborationIntent::MultiAgent => format!(
            "The current accepted user input explicitly requests a multi-Agent execution. You MUST invoke `{tool_name}` (canonical Action `agent/delegate`) once for the requested work, unless its native receipt already records acceptance. Preserve the user's deliverable. Use `strategy=parallel` with at least two bounded, non-overlapping tasks when the work can fan out; otherwise use `strategy=planned` with the full goal. Acceptance satisfies this invocation requirement: continue the original task and join its children instead of issuing the same delegation again."
        ),
        ExplicitCollaborationIntent::Subagent => format!(
            "The current accepted user input explicitly requests subagent execution. You MUST invoke `{tool_name}` (canonical Action `agent/delegate`) once for the requested work, unless its native receipt already records acceptance. Preserve the requested work in bounded tasks; use `strategy=parallel` for explicit fan-out and `strategy=planned` when the execution planner should decompose the goal. Acceptance satisfies this invocation requirement: keep the parent's tools available for independent work and final integration, and join the exact children without repeating the delegation."
        ),
    }
}

/// Apply the frozen conversation delegation policy to the already-admitted
/// model tool plan. Disabled policy can only narrow the plan; positive intent
/// supplies instructions without taking away the parent's remaining tools.
pub(super) fn route(
    input: &str,
    policy: DelegationPolicy,
    tool_plan: AgentToolPlan,
) -> CollaborationTurnRoute {
    if policy == DelegationPolicy::Disabled
        && tool_plan.contains_capability(COLLABORATION_CAPABILITY_ID)
    {
        return CollaborationTurnRoute {
            tool_plan: tool_plan.without_capability(COLLABORATION_CAPABILITY_ID),
            instructions: vec![
                "Agent collaboration is disabled for this Session. Do not start or claim to have started subagents, multiple Agents, or an AgentExecution. If the user explicitly requests them, state that collaboration must be enabled in a new Session."
                    .to_owned(),
            ],
            required_delegation: None,
        };
    }

    let Some(tool_name) = tool_plan
        .model_name_for_action(COLLABORATION_CAPABILITY_ID, DELEGATE_ACTION_ID)
        .map(str::to_owned)
    else {
        return CollaborationTurnRoute {
            tool_plan,
            instructions: Vec::new(),
            required_delegation: None,
        };
    };

    let intent = classify(input);
    let mut instructions = vec![policy_instruction(policy, &tool_name)];
    if let Some(intent) = intent {
        instructions.push(explicit_instruction(intent, &tool_name));
    }
    CollaborationTurnRoute { tool_plan, instructions, required_delegation:intent.map(|_|tool_name) }
}


#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nomifun_agent_contracts::{
        ActionId, CanonicalSchemaRef, CapabilityId, DigestHex, StrictJsonValue,
    };
    use nomifun_agent_runtime::{
        AgentEffectClass, AgentToolBinding, AgentToolPlan, input_schema_digest,
    };
    use nomifun_chat_model_broker::ChatToolDefinition;
    use serde_json::json;

    use super::*;

    fn binding(name: &str, capability: &str, action: &str) -> AgentToolBinding {
        let schema = StrictJsonValue(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        }));
        AgentToolBinding {
            model_name: name.to_owned(),
            definition: ChatToolDefinition {
                name: name.to_owned(),
                description: action.to_owned(),
                input_schema: schema.clone(),
                deferred: false,
            },
            schema_digest: input_schema_digest(&schema).unwrap(),
            canonical_input_schema_ref: CanonicalSchemaRef::from(format!(
                "schema://{action}/input"
            )),
            capability_contract_digest: DigestHex::from("a".repeat(64)),
            capability_id: CapabilityId::from(capability),
            action_id: ActionId::from(action),
            resource_binding_ids: BTreeSet::new(),
            effect_class: AgentEffectClass::ManagedEffect,
            parallel_safe: false,
        }
    }

    fn plan() -> AgentToolPlan {
        AgentToolPlan::new([
            binding(
                "plugin_delegate_hash",
                COLLABORATION_CAPABILITY_ID,
                DELEGATE_ACTION_ID,
            ),
            binding(
                "read_file",
                "workspace.files",
                "workspace.files/read",
            ),
        ])
        .unwrap()
    }

    #[test]
    fn physical_guidance_keeps_actions_and_results_under_the_shared_agent() {
        for input in ["北京今天的天气怎么样", "站起来再转身", "陪我聊聊今天的心情", "设计一段有创意的舞蹈"] {
            let original = plan();
            let result = route(input, DelegationPolicy::Automatic, original.clone());
            assert_eq!(result.tool_plan, original);
            assert!(result.required_delegation.is_none());
            assert!(result.instructions.join("\n").contains("ordinary short or indivisible request, answer directly"));
        }
    }

    #[test]
    fn explicit_requests_preserve_parent_tools_and_require_only_one_accepted_delegation() {
        for input in [
            "设计一个简单的多agent集群测试示例",
            "请使用 subagent 来完成这个工作",
            "不要只解释，请使用 subagent 实际执行",
            "启动多代理团队并行排查这些模块",
            "Use multiple agents to review these packages",
        ] {
            let routed = route(input, DelegationPolicy::Automatic, plan());
            assert_eq!(routed.tool_plan, plan(), "parent tools remain available: {input}");
            assert_eq!(routed.required_delegation.as_deref(),Some("plugin_delegate_hash"));
            let instructions = routed.instructions.join("\n");
            assert!(instructions.contains("MUST invoke"));
            assert!(instructions.contains("unless its native receipt already records acceptance"));
            assert!(instructions.contains("integrate and verify"));
            assert!(!instructions.contains("end the turn"));
        }
    }

    #[test]
    fn explanations_comparisons_and_negations_stay_on_the_normal_route() {
        for input in [
            "解释 multi-agent 集群是怎么工作的",
            "比较多 Agent 和单 Agent 的优缺点",
            "如何使用 subagent？",
            "不要使用 subagent，直接回答",
            "Where is the subagent implementation?",
        ] {
            let routed = route(input, DelegationPolicy::Automatic, plan());
            assert_eq!(routed.tool_plan.len(), 2, "{input}");
            assert!(routed.required_delegation.is_none());
        }
        let background = serde_json::json!({
            "task_brief":"Use subagents to build the whole product",
            "step_spec":"Implement the HTML game",
        }).to_string();
        let routed = route(&background, DelegationPolicy::Automatic, plan());
        assert_eq!(routed.tool_plan.len(), 2);
        assert!(routed.required_delegation.is_none(),"delegation mentioned only in the brief is source data");
    }

    #[test]
    fn policy_guidance_tracks_automatic_and_prefer_parallel() {
        let automatic = route("Review the release", DelegationPolicy::Automatic, plan());
        let preferred = route(
            "Review the release",
            DelegationPolicy::PreferParallel,
            plan(),
        );
        assert!(automatic.instructions.join("\n").contains("explicit current-user request"));
        assert!(!automatic.instructions.join("\n").contains("prefers parallel delegation"));
        assert!(preferred.instructions.join("\n").contains("prefers parallel delegation"));
    }

    #[test]
    fn disabled_policy_removes_every_collaboration_action() {
        let plan = AgentToolPlan::new([
            binding(
                "plugin_delegate_hash",
                COLLABORATION_CAPABILITY_ID,
                DELEGATE_ACTION_ID,
            ),
            binding(
                "plugin_fork_hash",
                COLLABORATION_CAPABILITY_ID,
                "agent/fork",
            ),
            binding(
                "read_file",
                "workspace.files",
                "workspace.files/read",
            ),
        ])
        .unwrap();
        let routed = route(
            "请使用 subagent 完成工作",
            DelegationPolicy::Disabled,
            plan,
        );
        assert_eq!(routed.tool_plan.len(), 1);
        assert!(routed
            .tool_plan
            .model_name_for_action(COLLABORATION_CAPABILITY_ID, DELEGATE_ACTION_ID)
            .is_none());
        assert!(routed.instructions.join("\n").contains("disabled"));

        let fork_only = AgentToolPlan::new([binding(
            "plugin_fork_hash",
            COLLABORATION_CAPABILITY_ID,
            "agent/fork",
        )])
        .unwrap();
        assert!(
            route("使用子代理", DelegationPolicy::Disabled, fork_only)
                .tool_plan
                .is_empty()
        );
    }

    #[test]
    fn absent_delegate_action_never_advertises_or_forces_collaboration() {
        let only_workspace = AgentToolPlan::new([binding(
            "read_file",
            "workspace.files",
            "workspace.files/read",
        )])
        .unwrap();
        let routed = route(
            "设计一个多 Agent 集群测试",
            DelegationPolicy::Automatic,
            only_workspace.clone(),
        );
        assert_eq!(routed.tool_plan, only_workspace);
        assert!(routed.instructions.is_empty());
    }
}
