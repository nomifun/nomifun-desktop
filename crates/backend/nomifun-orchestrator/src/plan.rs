//! 主管规划 (PlanProducer): turn a goal + a fleet member snapshot into an
//! executable task DAG ([`PlannedDag`]).
//!
//! [`LlmPlanProducer`] does one structured one-shot LLM call against a "lead"
//! model: it builds a planning prompt, asks the model for a strict-JSON
//! `{"tasks":[...]}` object, and parses it via [`parse_plan`].
//!
//! [`parse_plan`] is the heart of testability and is **fail-soft**: it extracts
//! the first JSON object from the raw model text (stripping ```json fences and
//! surrounding prose), parses it into a [`PlannedDag`], and on ANY failure
//! (no JSON, bad shape, empty `tasks`) logs a `warn!` and returns a single-task
//! fallback DAG built from the goal — so the Run engine always has something
//! executable.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::{one_shot_completion, resolve_provider_config, user_message};
use nomifun_api_types::{FleetMember, PlannedDag, PlannedTask};
use nomifun_common::{AppError, ProviderWithModel};
use nomifun_db::IProviderRepository;
use nomifun_db::models::Provider;

/// How many tokens the planner may use for its one-shot DAG completion.
const PLAN_MAX_TOKENS: u32 = 4096;

/// Max length of the fallback task title derived from the goal.
const FALLBACK_TITLE_LEN: usize = 60;

/// Per-model user-authored descriptions, keyed by `(provider_id, model)`.
///
/// Built from the providers' `model_descriptions` JSON (Task 1) and threaded
/// into the planning prompt so the lead model can pick the best-matching model
/// per task. A missing key means "no description" (rendered as `-`).
type DescriptionMap = HashMap<(String, String), String>;


/// Produces a task DAG from a goal. The Run engine consumes the result.
#[async_trait]
pub trait PlanProducer: Send + Sync {
    /// 把目标拆成任务 DAG。`members` 是 fleet 成员快照(供按 index 分派)。
    async fn produce(&self, goal: &str, members: &[FleetMember]) -> Result<PlannedDag, AppError>;
}

/// Production planner: a single structured LLM call against a "lead" model
/// yields a [`PlannedDag`] JSON, parsed fail-soft via [`parse_plan`].
///
/// Holds the `provider_repo` plus the `encryption_key` and `workspace` that
/// [`resolve_provider_config`] needs to materialize a `Config` from the
/// `lead` provider row (mirrors `nomifun-idmm`'s `LiveCompleter`).
pub struct LlmPlanProducer {
    provider_repo: Arc<dyn IProviderRepository>,
    encryption_key: [u8; 32],
    workspace: PathBuf,
    lead: ProviderWithModel,
}

impl LlmPlanProducer {
    /// Build a planner against the `lead` model. `encryption_key` / `workspace`
    /// are required to resolve the provider config for the LLM call (the brief
    /// signature is `new(provider_repo, lead)`; these two are adapted in to
    /// satisfy `resolve_provider_config`, matching the IDMM sidecar pattern).
    pub fn new(
        provider_repo: Arc<dyn IProviderRepository>,
        encryption_key: [u8; 32],
        workspace: impl Into<PathBuf>,
        lead: ProviderWithModel,
    ) -> Self {
        Self {
            provider_repo,
            encryption_key,
            workspace: workspace.into(),
            lead,
        }
    }
}

/// Pick the planning "lead" model from the fleet members.
///
/// The app wires `LlmPlanProducer` with an EMPTY placeholder `lead`
/// (`provider_id:""`, `model:""`), which `resolve_provider_config` rejects
/// before `parse_plan`'s fail-soft can ever run — so every real run would stall
/// in `planning`. The real provider+model live on the fleet members, so derive
/// the lead from the FIRST member that carries BOTH a non-empty `provider_id`
/// AND a non-empty `model` (mirroring the Nomi-engine member contract in
/// `worker.rs`). If no member qualifies, fall back to the construction-time
/// `lead` override.
fn pick_lead(members: &[FleetMember], fallback: &ProviderWithModel) -> ProviderWithModel {
    for m in members {
        if let (Some(pid), Some(model)) = (m.provider_id.as_ref(), m.model.as_ref()) {
            if !pid.is_empty() && !model.is_empty() {
                return ProviderWithModel {
                    provider_id: pid.clone(),
                    model: model.clone(),
                    use_model: Some(model.clone()),
                };
            }
        }
    }
    fallback.clone()
}

#[async_trait]
impl PlanProducer for LlmPlanProducer {
    async fn produce(&self, goal: &str, members: &[FleetMember]) -> Result<PlannedDag, AppError> {
        // Derive the lead from the fleet members; `self.lead` is the
        // construction-time override/fallback only (the app wires it empty).
        let lead = pick_lead(members, &self.lead);

        // The model to plan with: prefer the explicit use_model alias, else model.
        let model = lead.use_model.as_deref().unwrap_or(&lead.model);

        let cfg = resolve_provider_config(
            &self.provider_repo,
            &self.encryption_key,
            &lead.provider_id,
            model,
            self.workspace.as_path(),
        )
        .await?;

        // Build the (provider_id, model) → description map so the prompt can
        // surface each member's user-authored model description. Fetch every
        // provider once via `list()` (cheaper than N `find_by_id` calls and the
        // member set is small), then decode each provider's `model_descriptions`
        // JSON fail-soft. A repo error here MUST NOT fail the plan — descriptions
        // are an optimization, so degrade to an empty map (all `desc=-`).
        let descriptions = match self.provider_repo.list().await {
            Ok(providers) => build_description_map(&providers, members),
            Err(err) => {
                tracing::warn!(error = %err, "failed to list providers for plan descriptions; planning without them");
                DescriptionMap::new()
            }
        };

        let user = build_plan_user_prompt(goal, members, &descriptions);
        let raw = one_shot_completion(&cfg, PLAN_SYSTEM, vec![user_message(user)], PLAN_MAX_TOKENS).await?;

        // parse_plan is fail-soft: a bad/empty reply degrades to a single-task DAG
        // rather than erroring out, so the engine always has an executable plan.
        Ok(parse_plan(&raw, goal))
    }
}

/// System prompt: instruct the model to output ONLY a strict-JSON task DAG.
const PLAN_SYSTEM: &str = "You are a planning supervisor for a multi-agent fleet. \
Decompose the user's GOAL into an executable task DAG and output ONLY a single JSON object — \
no prose, no explanation, no markdown fences. \
The JSON object MUST have exactly this shape:\n\
{\"tasks\":[{\"title\":string,\"spec\":string,\"task_profile\":{\"kind\":string,\"needs_vision\":bool,\"needs_long_context\":bool,\"needs_high_reasoning\":bool,\"bulk\":bool}?,\"depends_on\":[int],\"member_index\":int?,\"rationale\":string?}]}\n\
Rules:\n\
- \"depends_on\" lists the 0-based indices of EARLIER tasks (smaller index) this task depends on; the graph MUST be acyclic.\n\
- \"member_index\" is the 0-based index into the provided MEMBERS list, if you want to pre-assign the task to a member; omit it to let the engine route automatically.\n\
- Each member row carries a \"desc\" column: the user-authored description of that member's model. PREFER the member whose \"desc\" best matches the task and set \"member_index\" accordingly; \"desc=-\" means no description is available.\n\
- \"task_profile\", \"member_index\" and \"rationale\" are optional.\n\
- \"title\" is a short imperative label; \"spec\" is the full instruction the worker agent will execute.\n\
- Keep the plan minimal but complete: one task if the goal is atomic, several with dependencies if it must be staged.\n\
Output the JSON object and nothing else.";

/// Build the `(provider_id, model) → description` map for the prompt.
///
/// For each distinct `provider_id` referenced by a member, decode that
/// provider's `model_descriptions` JSON (`{model_id: description}`) and record
/// the description for every `(provider_id, model)` a member actually uses.
///
/// **Fail-soft on every axis** — descriptions are an optimization, never a hard
/// dependency:
/// - a provider with no row, `model_descriptions == None`, or the Task-1 default
///   `"{}"` contributes nothing;
/// - a malformed `model_descriptions` JSON is skipped (no entries) with a warn,
///   not propagated as an error;
/// - a blank/whitespace-only description is dropped (treated as "no description").
fn build_description_map(providers: &[Provider], members: &[FleetMember]) -> DescriptionMap {
    // Index providers by id for O(1) lookup as we walk the members.
    let by_id: HashMap<&str, &Provider> = providers.iter().map(|p| (p.id.as_str(), p)).collect();

    // Decode each referenced provider's model_descriptions once, fail-soft.
    let mut decoded: HashMap<&str, HashMap<String, String>> = HashMap::new();
    let mut out = DescriptionMap::new();

    for m in members {
        let (Some(pid), Some(model)) = (m.provider_id.as_deref(), m.model.as_deref()) else {
            continue;
        };
        if pid.is_empty() || model.is_empty() {
            continue;
        }

        // Lazily decode this provider's descriptions JSON the first time we see it.
        let table = decoded.entry(pid).or_insert_with(|| {
            let Some(provider) = by_id.get(pid) else {
                return HashMap::new();
            };
            let raw = provider.model_descriptions.as_deref().unwrap_or("{}");
            match serde_json::from_str::<HashMap<String, String>>(raw) {
                Ok(map) => map,
                Err(err) => {
                    tracing::warn!(
                        provider_id = pid,
                        error = %err,
                        "provider model_descriptions is not a JSON object; ignoring"
                    );
                    HashMap::new()
                }
            }
        });

        if let Some(desc) = table.get(model) {
            let trimmed = desc.trim();
            if !trimmed.is_empty() {
                out.insert((pid.to_string(), model.to_string()), trimmed.to_string());
            }
        }
    }
    out
}

/// Build the user message: the goal plus a compact member roster.
fn build_plan_user_prompt(
    goal: &str,
    members: &[FleetMember],
    descriptions: &DescriptionMap,
) -> String {
    let mut out = String::new();
    out.push_str("GOAL:\n");
    out.push_str(goal);
    out.push_str("\n\nMEMBERS (index, agent_id, role_hint, strengths, desc):\n");
    if members.is_empty() {
        out.push_str("(none — plan without pre-assigning member_index)\n");
    } else {
        for (i, m) in members.iter().enumerate() {
            let role = m.role_hint.as_deref().unwrap_or("-");
            let strengths = m
                .capability_profile
                .as_ref()
                .map(|p| p.strengths.join("/"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "-".to_string());
            // Look up the member's model description; missing → the "-" sentinel.
            let desc = match (m.provider_id.as_deref(), m.model.as_deref()) {
                (Some(pid), Some(model)) => descriptions
                    .get(&(pid.to_string(), model.to_string()))
                    .map(String::as_str)
                    .unwrap_or("-"),
                _ => "-",
            };
            out.push_str(&format!(
                "{i}. {} | role={role} | strengths={strengths} | desc={desc}\n",
                m.agent_id
            ));
        }
    }
    out.push_str("\nReturn ONLY the JSON task DAG.");
    out
}

/// Parse the raw model text into a [`PlannedDag`], **fail-soft**.
///
/// Strips ```json/``` fences and surrounding prose, locates the first balanced
/// `{...}` JSON object, and deserializes it. On ANY failure — no JSON object,
/// malformed JSON, wrong shape, or an empty `tasks` array — logs a `warn!` and
/// returns a single-task fallback DAG derived from `goal` (so the engine always
/// has an executable plan).
pub fn parse_plan(raw: &str, goal: &str) -> PlannedDag {
    match extract_json_object(raw).and_then(|json| serde_json::from_str::<PlannedDag>(&json).ok()) {
        Some(dag) if !dag.tasks.is_empty() => dag,
        Some(_) => {
            tracing::warn!("planner output parsed but tasks were empty; using fallback DAG");
            fallback_dag(goal)
        }
        None => {
            tracing::warn!(
                raw_len = raw.len(),
                "planner output unparseable (no valid JSON task DAG); using fallback DAG"
            );
            fallback_dag(goal)
        }
    }
}

/// Extract the first balanced top-level `{...}` substring from `raw`,
/// after stripping any markdown code fences. Returns `None` if no balanced
/// object is found. Quote/escape aware so braces inside strings don't confuse
/// the brace counter.
fn extract_json_object(raw: &str) -> Option<String> {
    // Strip code fences first; the model is told not to use them, but be robust.
    let cleaned = raw.replace("```json", "").replace("```JSON", "").replace("```", "");

    let bytes = cleaned.as_bytes();
    let start = cleaned.find('{')?;

    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(cleaned[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Single-task fallback DAG: the whole goal as one task assigned to member 0.
fn fallback_dag(goal: &str) -> PlannedDag {
    PlannedDag {
        tasks: vec![PlannedTask {
            title: truncate_title(goal),
            spec: goal.to_string(),
            task_profile: None,
            depends_on: vec![],
            member_index: Some(0),
            rationale: Some("fallback: planner output unparseable".to_string()),
        }],
    }
}

/// Truncate the goal into a short title (~`FALLBACK_TITLE_LEN` chars),
/// respecting char boundaries (CJK-safe).
fn truncate_title(goal: &str) -> String {
    let trimmed = goal.trim();
    if trimmed.chars().count() <= FALLBACK_TITLE_LEN {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(FALLBACK_TITLE_LEN).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::TaskProfile;

    /// Fixed 2-task DAG mock proving the [`PlanProducer`] trait shape. Reused by
    /// the Run engine (Task 6) to drive the scheduler without a live LLM.
    struct MockPlanProducer;

    #[async_trait]
    impl PlanProducer for MockPlanProducer {
        async fn produce(&self, _goal: &str, _members: &[FleetMember]) -> Result<PlannedDag, AppError> {
            Ok(PlannedDag {
                tasks: vec![
                    PlannedTask {
                        title: "Gather".to_string(),
                        spec: "collect sources".to_string(),
                        task_profile: None,
                        depends_on: vec![],
                        member_index: Some(0),
                        rationale: None,
                    },
                    PlannedTask {
                        title: "Synthesize".to_string(),
                        spec: "write the report".to_string(),
                        task_profile: None,
                        depends_on: vec![0],
                        member_index: Some(1),
                        rationale: None,
                    },
                ],
            })
        }
    }

    #[tokio::test]
    async fn mock_plan_producer_returns_fixed_two_task_dag() {
        let planner: Arc<dyn PlanProducer> = Arc::new(MockPlanProducer);
        let dag = planner.produce("anything", &[]).await.expect("mock never errors");

        assert_eq!(dag.tasks.len(), 2);
        assert_eq!(dag.tasks[0].title, "Gather");
        assert!(dag.tasks[0].depends_on.is_empty());
        assert_eq!(dag.tasks[1].title, "Synthesize");
        assert_eq!(dag.tasks[1].depends_on, vec![0]);
        assert_eq!(dag.tasks[1].member_index, Some(1));
    }

    #[test]
    fn parse_plan_accepts_bare_valid_json() {
        let raw = r#"{"tasks":[
            {"title":"Research","spec":"find sources","depends_on":[],"member_index":0},
            {"title":"Write","spec":"synthesize","depends_on":[0],"member_index":1,
             "task_profile":{"kind":"writing","needs_vision":false,"needs_long_context":true,"needs_high_reasoning":true,"bulk":false}}
        ]}"#;
        let dag = parse_plan(raw, "Research and write a report");

        assert_eq!(dag.tasks.len(), 2);
        assert_eq!(dag.tasks[0].title, "Research");
        assert_eq!(dag.tasks[0].member_index, Some(0));
        assert_eq!(dag.tasks[1].depends_on, vec![0]);
        let profile: &TaskProfile = dag.tasks[1].task_profile.as_ref().expect("profile decoded");
        assert_eq!(profile.kind, "writing");
        assert!(profile.needs_long_context);
        assert!(profile.needs_high_reasoning);
        assert!(!profile.bulk);
    }

    #[test]
    fn parse_plan_strips_json_fences() {
        let raw = "```json\n{\"tasks\":[{\"title\":\"One\",\"spec\":\"do it\",\"depends_on\":[]}]}\n```";
        let dag = parse_plan(raw, "goal");
        assert_eq!(dag.tasks.len(), 1);
        assert_eq!(dag.tasks[0].title, "One");
    }

    #[test]
    fn parse_plan_extracts_json_wrapped_in_prose() {
        let raw = "Sure! Here is the plan you asked for:\n\n\
            {\"tasks\":[{\"title\":\"Alpha\",\"spec\":\"step\",\"depends_on\":[]}]}\n\n\
            Let me know if you'd like changes.";
        let dag = parse_plan(raw, "goal");
        assert_eq!(dag.tasks.len(), 1);
        assert_eq!(dag.tasks[0].title, "Alpha");
    }

    #[test]
    fn parse_plan_handles_braces_inside_strings() {
        // A literal "}" inside a string value must not prematurely close the object.
        let raw = r#"{"tasks":[{"title":"Use {braces}","spec":"emit a } char","depends_on":[]}]}"#;
        let dag = parse_plan(raw, "goal");
        assert_eq!(dag.tasks.len(), 1);
        assert_eq!(dag.tasks[0].title, "Use {braces}");
        assert_eq!(dag.tasks[0].spec, "emit a } char");
    }

    #[test]
    fn parse_plan_falls_back_on_garbage() {
        let dag = parse_plan("I'm sorry, I cannot help with that.", "Build a rocket");
        assert_eq!(dag.tasks.len(), 1);
        assert_eq!(dag.tasks[0].title, "Build a rocket");
        assert_eq!(dag.tasks[0].spec, "Build a rocket");
        assert_eq!(dag.tasks[0].member_index, Some(0));
        assert!(dag.tasks[0].depends_on.is_empty());
        assert_eq!(
            dag.tasks[0].rationale.as_deref(),
            Some("fallback: planner output unparseable")
        );
    }

    #[test]
    fn parse_plan_falls_back_on_empty_tasks() {
        let dag = parse_plan(r#"{"tasks":[]}"#, "Some goal");
        assert_eq!(dag.tasks.len(), 1, "empty tasks must degrade to fallback");
        assert_eq!(dag.tasks[0].title, "Some goal");
    }

    #[test]
    fn parse_plan_falls_back_on_malformed_json() {
        // Unterminated object → no balanced match → fallback.
        let dag = parse_plan(r#"{"tasks":[{"title":"x" "#, "Goal text");
        assert_eq!(dag.tasks.len(), 1);
        assert_eq!(dag.tasks[0].spec, "Goal text");
    }

    #[test]
    fn parse_plan_truncates_long_goal_title() {
        let goal = "x".repeat(200);
        let dag = parse_plan("not json", &goal);
        // 60 chars + ellipsis.
        assert_eq!(dag.tasks[0].title.chars().count(), FALLBACK_TITLE_LEN + 1);
        assert!(dag.tasks[0].title.ends_with('…'));
        // spec keeps the full goal.
        assert_eq!(dag.tasks[0].spec, goal);
    }

    #[test]
    fn truncate_title_is_cjk_safe() {
        let goal = "目标".repeat(50); // 100 CJK chars
        let title = truncate_title(&goal);
        assert_eq!(title.chars().count(), FALLBACK_TITLE_LEN + 1);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn build_plan_user_prompt_includes_goal_and_members() {
        let member = FleetMember {
            id: "fm_1".to_string(),
            agent_id: "agent_research".to_string(),
            provider_id: None,
            model: None,
            role_hint: Some("researcher".to_string()),
            capability_profile: Some(nomifun_api_types::CapabilityProfile {
                strengths: vec!["search".to_string(), "synthesis".to_string()],
                modalities: vec!["text".to_string()],
                tools: true,
                reasoning: "high".to_string(),
                cost_tier: "premium".to_string(),
                speed_tier: "medium".to_string(),
            }),
            constraints: None,
            sort_order: 0,
            description: None,
            system_prompt: None,
            enabled_skills: Vec::new(),
            disabled_builtin_skills: Vec::new(),
        };
        let prompt = build_plan_user_prompt("Research X", &[member], &DescriptionMap::new());
        assert!(prompt.contains("Research X"));
        assert!(prompt.contains("0. agent_research"));
        assert!(prompt.contains("role=researcher"));
        assert!(prompt.contains("search/synthesis"));
        // No description available for this member → desc column is the "-" sentinel.
        assert!(prompt.contains("desc=-"), "missing-description members get desc=-: {prompt}");
    }

    #[test]
    fn build_plan_user_prompt_handles_no_members() {
        let prompt = build_plan_user_prompt("Solo goal", &[], &DescriptionMap::new());
        assert!(prompt.contains("Solo goal"));
        assert!(prompt.contains("none"));
    }

    // (a) build_plan_user_prompt surfaces a member's model description in the
    // desc= column when the (provider_id, model) → description map carries one,
    // so the planner can read it and pick the best-matching model.
    #[test]
    fn build_plan_user_prompt_includes_model_description() {
        let member = member_with(Some("prov_x"), Some("model-x"));
        let mut descriptions = DescriptionMap::new();
        descriptions.insert(
            ("prov_x".to_string(), "model-x".to_string()),
            "擅长前端与可视化".to_string(),
        );
        let prompt = build_plan_user_prompt("Build a UI", &[member], &descriptions);
        assert!(
            prompt.contains("desc=擅长前端与可视化"),
            "description must surface in the desc= column: {prompt}"
        );
    }

    /// Build a minimal `FleetMember` carrying the given provider/model.
    fn member_with(provider_id: Option<&str>, model: Option<&str>) -> FleetMember {
        FleetMember {
            id: "fm".to_string(),
            agent_id: "agent".to_string(),
            provider_id: provider_id.map(str::to_string),
            model: model.map(str::to_string),
            role_hint: None,
            capability_profile: None,
            constraints: None,
            sort_order: 0,
            description: None,
            system_prompt: None,
            enabled_skills: Vec::new(),
            disabled_builtin_skills: Vec::new(),
        }
    }

    #[test]
    fn pick_lead_picks_first_member_with_provider_and_model() {
        let fallback = ProviderWithModel {
            provider_id: String::new(),
            model: String::new(),
            use_model: None,
        };
        // members[0] lacks a model; members[1] is fully specified → pick [1].
        let members = vec![
            member_with(Some("prov_a"), None),
            member_with(Some("prov_b"), Some("model_b")),
        ];
        let lead = pick_lead(&members, &fallback);
        assert_eq!(lead.provider_id, "prov_b");
        assert_eq!(lead.model, "model_b");
        assert_eq!(lead.use_model.as_deref(), Some("model_b"));
    }

    #[test]
    fn pick_lead_skips_empty_string_provider() {
        let fallback = ProviderWithModel {
            provider_id: String::new(),
            model: String::new(),
            use_model: None,
        };
        // members[0] has an EMPTY provider_id → skipped; members[1] qualifies.
        let members = vec![
            member_with(Some(""), Some("model_x")),
            member_with(Some("prov_real"), Some("model_real")),
        ];
        let lead = pick_lead(&members, &fallback);
        assert_eq!(lead.provider_id, "prov_real");
        assert_eq!(lead.model, "model_real");
        assert_eq!(lead.use_model.as_deref(), Some("model_real"));
    }

    #[test]
    fn pick_lead_falls_back_when_no_member_qualifies() {
        let fallback = ProviderWithModel {
            provider_id: "fallback_prov".to_string(),
            model: "fallback_model".to_string(),
            use_model: Some("fallback_use".to_string()),
        };
        // No member carries both provider+model → return the fallback as-is.
        let members = vec![member_with(None, Some("m")), member_with(Some("p"), None), member_with(Some(""), Some(""))];
        let lead = pick_lead(&members, &fallback);
        assert_eq!(lead.provider_id, "fallback_prov");
        assert_eq!(lead.model, "fallback_model");
        assert_eq!(lead.use_model.as_deref(), Some("fallback_use"));
    }

    /// Build a minimal `Provider` row carrying the given `model_descriptions`
    /// JSON (the only field `build_description_map` reads, besides `id`).
    fn provider_with_descriptions(id: &str, model_descriptions: Option<&str>) -> Provider {
        Provider {
            id: id.to_string(),
            platform: "openai".to_string(),
            name: "p".to_string(),
            base_url: String::new(),
            api_key_encrypted: String::new(),
            models: "[]".to_string(),
            enabled: true,
            capabilities: "[]".to_string(),
            context_limit: None,
            model_protocols: None,
            model_descriptions: model_descriptions.map(str::to_string),
            model_enabled: None,
            model_health: None,
            bedrock_config: None,
            is_full_url: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    // build_description_map decodes each provider's model_descriptions JSON and
    // keys the result by (provider_id, model) for the members that reference it.
    #[test]
    fn build_description_map_keys_by_provider_and_model() {
        let providers = vec![provider_with_descriptions(
            "prov_a",
            Some(r#"{"model-a":"擅长前端","model-b":"擅长后端"}"#),
        )];
        let members = vec![
            member_with(Some("prov_a"), Some("model-a")),
            member_with(Some("prov_a"), Some("model-b")),
        ];
        let map = build_description_map(&providers, &members);
        assert_eq!(
            map.get(&("prov_a".to_string(), "model-a".to_string())).map(String::as_str),
            Some("擅长前端")
        );
        assert_eq!(
            map.get(&("prov_a".to_string(), "model-b".to_string())).map(String::as_str),
            Some("擅长后端")
        );
    }

    // An unset model_descriptions (Task 1 stores the default as `Some("{}")`) and
    // an absent model entry both yield "no description" (no map entry) — not an error.
    #[test]
    fn build_description_map_treats_empty_object_as_no_description() {
        let providers = vec![
            provider_with_descriptions("prov_empty", Some("{}")),
            provider_with_descriptions("prov_partial", Some(r#"{"other-model":"x"}"#)),
        ];
        let members = vec![
            member_with(Some("prov_empty"), Some("model-a")),
            member_with(Some("prov_partial"), Some("model-a")),
        ];
        let map = build_description_map(&providers, &members);
        assert!(map.is_empty(), "no member matched a description entry: {map:?}");
    }

    // A blank description string is dropped (treated as "no description"), and a
    // malformed model_descriptions JSON is fail-soft (no entries, no panic/error).
    #[test]
    fn build_description_map_is_fail_soft_on_bad_json_and_blank() {
        let providers = vec![
            provider_with_descriptions("prov_bad", Some("not json at all")),
            provider_with_descriptions("prov_blank", Some(r#"{"model-a":"   "}"#)),
            provider_with_descriptions("prov_none", None),
        ];
        let members = vec![
            member_with(Some("prov_bad"), Some("model-a")),
            member_with(Some("prov_blank"), Some("model-a")),
            member_with(Some("prov_none"), Some("model-a")),
        ];
        let map = build_description_map(&providers, &members);
        assert!(map.is_empty(), "bad/blank/absent descriptions yield no entries: {map:?}");
    }
}
