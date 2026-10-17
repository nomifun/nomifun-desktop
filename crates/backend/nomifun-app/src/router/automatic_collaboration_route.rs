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
const ROBOT_INTERACTION_GUIDANCE: &str = "Physical Companion interaction policy: use visible device tools immediately for a clear supported movement; do not plan, narrate, probe unrelated state, or delegate that simple action. For a device-action-only request, finish silently after every requested action has a successful receipt; do not add greetings, acknowledgments, recaps, praise, follow-up questions or unrelated suggestions. Use only an actually advertised silent-finish control under its declared contract, and never stop before all requested actions. Speak for a question, an explicitly requested verbal receipt, a real failure, blocker, uncertain result or necessary clarification. Accepted dispatch does not prove completed movement. Ordinary companionship stays conversational and follows the user's requested depth. A delivered desktop-task result needs only a concise answer to the original question, without repeating the task process.";

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

/// The physical receptionist fills capability gaps through its own Companion's
/// frozen desktop role. A one-question lookup is a valid directed task; the
/// generic fan-out/complexity advice would incorrectly discourage it here.
/// The caller has already proved the canonical physical role and narrow grants.
pub(super) fn route_robot(
    input: &str,
    policy: DelegationPolicy,
    tool_plan: AgentToolPlan,
) -> CollaborationTurnRoute {
    if policy == DelegationPolicy::Disabled {
        let mut result = route(input, policy, tool_plan);
        result.instructions.insert(0, ROBOT_INTERACTION_GUIDANCE.into());
        return result;
    }
    let Some(tool_name) = tool_plan
        .model_name_for_action(COLLABORATION_CAPABILITY_ID, DELEGATE_ACTION_ID)
        .map(str::to_owned)
    else {
        return CollaborationTurnRoute {
            tool_plan, tool_choice: ChatToolChoice::Auto,
            instructions: vec![ROBOT_INTERACTION_GUIDANCE.into()], forced: false,
        };
    };
    let forced = classify(input).is_some();
    let instruction = format!(
        "For live information (including today's weather in Beijing), research, files, applications or any platform task missing from this Agent, invoke `{tool_name}` with `strategy=planned`, `target_role=desktop` and the complete `goal`, even for a short indivisible question, unless the current user forbids delegation. Respect the user's explicit limits on using subagents; if no permitted local tool can complete the task, state that concrete limitation briefly. This creates a real subtask for the SAME Companion's saved desktop Agent/model/authority, without a planning-model round. Do not refuse just because this physical role lacks the local tool, and do not invent current facts. Use authorized ToolSearch for a deferred schema. Deep research, elaborate creative motion design and autonomous multi-step planning belong in that desktop task; include the user's goal, known device abilities and constraints, and request a concise result or motion plan. The worker cannot move this robot through delegation; apply any returned plan only within the original accepted request and current device grants. On a successful delegation receipt, end this turn without narration or acknowledgment: never poll or claim the work has finished. The runtime handles this accepted handoff without requiring a public reply. The host returns the recorded result asynchronously to this source; summarize the answer concisely. Report actual unavailability or a failed handoff briefly, rather than promising a result.{}",
        if forced { " The current user explicitly requested collaboration: invoke this directed task now, preserving the requested deliverable. Use the directed parameters above rather than generic parallel tasks." } else { "" },
    );
    CollaborationTurnRoute {
        tool_plan: if forced {
            tool_plan.for_action(COLLABORATION_CAPABILITY_ID, DELEGATE_ACTION_ID)
        } else { tool_plan },
        tool_choice: if forced { ChatToolChoice::Specific { name: tool_name } } else { ChatToolChoice::Auto },
        instructions: vec![ROBOT_INTERACTION_GUIDANCE.into(), instruction],
        forced,
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
    fn robot_capability_gaps_use_directed_tasks_even_for_one_short_question() {
        for input in ["北京今天的天气怎么样", "站起来再转身", "陪我聊聊今天的心情", "设计一段有创意的舞蹈"] {
            let original = plan();
            let result = route_robot(input, DelegationPolicy::Automatic, original.clone());
            assert_eq!(result.tool_plan, original, "semantic choice must retain every granted tool");
            assert_eq!(result.tool_choice, ChatToolChoice::Auto, "no keyword dispatch or fabricated movement");
            assert!(!result.forced);
            let guidance = result.instructions.join("\n");
            assert!(guidance.contains("`plugin_delegate_hash`"));
            assert!(guidance.contains("target_role=desktop") && guidance.contains("short indivisible question"));
            assert!(guidance.contains("without a planning-model round") && guidance.contains("never poll"));
            assert!(guidance.contains("end this turn without narration or acknowledgment"));
            assert!(!guidance.contains("at most a brief acknowledgment"));
            assert!(guidance.contains("finish silently after every requested action has a successful receipt"));
            assert!(guidance.contains("explicitly requested verbal receipt") && guidance.contains("necessary clarification"));
            assert!(!guidance.contains("finish_without_reply"), "the Runtime describes a control only while it is actually exposed");
            assert!(!guidance.contains("strategy=parallel"));
        }
    }

    #[test]
    fn robot_explicit_subagent_request_uses_the_directed_contract() {
        let result = route_robot("请使用 subagent 查询北京今天的天气", DelegationPolicy::Automatic, plan());
        assert!(result.forced);
        assert_eq!(result.tool_choice, ChatToolChoice::Specific { name: "plugin_delegate_hash".into() });
        assert_eq!(result.tool_plan.len(), 1);
        let guidance = result.instructions.join("\n");
        assert!(guidance.contains("strategy=planned") && guidance.contains("target_role=desktop"));
        assert!(!guidance.contains("strategy=parallel"));
    }

    #[test]
    fn robot_guidance_never_resurrects_a_missing_or_disabled_delegate() {
        let result = route_robot("北京今天的天气怎么样", DelegationPolicy::Disabled, plan());
        assert!(!result.tool_plan.contains_capability(COLLABORATION_CAPABILITY_ID));
        assert!(!result.instructions.join("\n").contains("target_role=desktop"));
        assert!(result.instructions.join("\n").contains("collaboration is disabled"));
        let empty = route_robot("继续播报结果", DelegationPolicy::Automatic, AgentToolPlan::default());
        assert!(empty.tool_plan.is_empty());
        assert_eq!(empty.instructions, vec![ROBOT_INTERACTION_GUIDANCE]);
        assert!(!empty.instructions.join("\n").contains("plugin_delegate_hash"));
        let declined = route_robot("不要使用 subagent 查询天气", DelegationPolicy::Automatic, plan());
        assert!(!declined.forced);
        assert_eq!(declined.tool_choice, ChatToolChoice::Auto);
        assert!(declined.instructions.join("\n").contains("unless the current user forbids delegation"));
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
        let background = serde_json::json!({
            "task_brief":"Use subagents to build the whole product",
            "step_spec":"Implement the HTML game",
        }).to_string();
        let routed = route(&background, DelegationPolicy::Automatic, plan());
        assert!(!routed.forced);
        assert_eq!(routed.tool_plan.len(), 2);
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
