//! Policy-aware routing for explicit Agent collaboration requests.
//!
//! The retired Runtime appended persistent delegation guidance to every
//! eligible desktop Session. During the unified Runtime cutover that guidance
//! was deleted while the typed `delegation_policy` kept flowing unused into
//! [`AgentRuntimeBuildOptions`](nomifun_ai_agent::types::AgentRuntimeBuildOptions).
//! This module restores that contract against the canonical Capability Action
//! instead of a legacy MCP tool name.
//!
//! Explicit routing is deliberately conservative. A positive match requires
//! both collaboration vocabulary and a current execution verb. Explanations,
//! comparisons, and direct negations stay on the ordinary model route.

use nomifun_agent_runtime::AgentToolPlan;
use nomifun_chat_model_broker::ChatToolChoice;
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
    pub(super) tool_choice: ChatToolChoice,
    pub(super) instructions: Vec<String>,
    pub(super) forced: bool,
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
    let normalized = input.trim().to_lowercase();
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
        "Agent collaboration is available through the advertised function `{tool_name}` whose canonical Action is `agent/delegate`. Treat an explicit current-user request to use, start, build, design, test, or run multi-agent, multiple-agent, Agent-team, subagent, 多 Agent、多代理、Agent 集群、子 Agent, or 子代理 work as an operational request to invoke that Action, not as a request to merely describe sample code. For independent work call it exactly once with `strategy=parallel`, a `tasks` JSON array of non-overlapping tasks, and `synthesize=true` when one downstream Agent should combine every result. For a complex goal whose dependency graph should be planned, call it once with `strategy=planned` and `goal`. A successful receipt transfers completion to the durable AgentExecution: end the turn without polling or claiming that delegated work is already complete. For an ordinary short or indivisible request, answer directly."
    );
    match policy {
        DelegationPolicy::Automatic => base,
        DelegationPolicy::PreferParallel => format!(
            "{base}\n\nThis Session prefers parallel delegation. Deliberately check every substantive request for independent workstreams and prefer `agent/delegate` when parallel execution materially improves speed, coverage, or focused context. Do not manufacture duplicate tasks merely to create a multi-Agent execution."
        ),
        DelegationPolicy::Disabled => unreachable!("disabled policy has a separate instruction"),
    }
}

fn forced_instruction(intent: ExplicitCollaborationIntent, tool_name: &str) -> String {
    match intent {
        ExplicitCollaborationIntent::MultiAgent => format!(
            "The current accepted user input explicitly requests a multi-Agent execution. You MUST invoke `{tool_name}` (canonical Action `agent/delegate`) exactly once in this model step instead of returning only a design, example, or explanation. Preserve the user's requested deliverable. Use `strategy=parallel` with at least two bounded, non-overlapping tasks when the work can fan out, normally with `synthesize=true`; otherwise use `strategy=planned` with the full goal so the execution planner can create a dependency DAG."
        ),
        ExplicitCollaborationIntent::Subagent => format!(
            "The current accepted user input explicitly requests subagent execution. You MUST invoke `{tool_name}` (canonical Action `agent/delegate`) exactly once in this model step instead of claiming that a subagent was started or returning only instructions. Preserve the requested work in one or more bounded tasks; use `strategy=parallel` for explicit task fan-out and `strategy=planned` when the execution planner should decompose the goal."
        ),
    }
}

/// Apply the frozen conversation delegation policy to the already-admitted
/// model tool plan. This can only narrow the plan. An explicit request also
/// selects the exact generated function name so provider behavior is not left
/// to probabilistic tool choice.
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
            tool_choice: ChatToolChoice::Auto,
            instructions: vec![
                "Agent collaboration is disabled for this Session. Do not start or claim to have started subagents, multiple Agents, or an AgentExecution. If the user explicitly requests them, state that collaboration must be enabled in a new Session."
                    .to_owned(),
            ],
            forced: false,
        };
    }

    let Some(tool_name) = tool_plan
        .model_name_for_action(COLLABORATION_CAPABILITY_ID, DELEGATE_ACTION_ID)
        .map(str::to_owned)
    else {
        return CollaborationTurnRoute {
            tool_plan,
            tool_choice: ChatToolChoice::Auto,
            instructions: Vec::new(),
            forced: false,
        };
    };

    let intent = classify(input);
    let mut instructions = vec![policy_instruction(policy, &tool_name)];
    if let Some(intent) = intent {
        instructions.push(forced_instruction(intent, &tool_name));
        CollaborationTurnRoute {
            tool_plan: tool_plan.for_action(COLLABORATION_CAPABILITY_ID, DELEGATE_ACTION_ID),
            tool_choice: ChatToolChoice::Specific { name: tool_name },
            instructions,
            forced: true,
        }
    } else {
        CollaborationTurnRoute {
            tool_plan,
            tool_choice: ChatToolChoice::Auto,
            instructions,
            forced: false,
        }
    }
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
    fn routes_the_reported_chinese_multi_agent_and_subagent_requests() {
        for input in [
            "设计一个简单的多agent集群测试示例",
            "请使用 subagent 来完成这个工作",
            "不要只解释，请使用 subagent 实际执行",
            "启动多代理团队并行排查这些模块",
            "Use multiple agents to review these packages",
        ] {
            let routed = route(input, DelegationPolicy::Automatic, plan());
            assert!(routed.forced, "{input}");
            assert_eq!(routed.tool_plan.len(), 1, "{input}");
            assert!(matches!(
                routed.tool_choice,
                ChatToolChoice::Specific { ref name } if name == "plugin_delegate_hash"
            ));
            assert!(routed.instructions.join("\n").contains("MUST invoke"));
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
            assert!(!routed.forced, "{input}");
            assert_eq!(routed.tool_plan.len(), 2, "{input}");
            assert_eq!(routed.tool_choice, ChatToolChoice::Auto);
        }
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
        assert!(!routed.forced);
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
        assert_eq!(routed.tool_choice, ChatToolChoice::Auto);
        assert!(routed.instructions.is_empty());
        assert!(!routed.forced);
    }
}
