//! Generic one-shot engine turn with a construction-time tool whitelist.
//!
//! 底座选型（Task 3 Step 1 探索结论）：复用本 crate 既有的 provider 直连路径
//! （`resolve_provider_config` + `nomi_providers::create_provider`，与
//! `one_shot_completion`/IDMM sidecar/companion learner 同族），在其上补一个
//! 最小 tool-loop，而不是复用完整 nomi 引擎会话（`nomi_agent::session`）。
//! 理由：完整引擎的会话构造会注册内建 OS/文件/浏览器工具、技能与 MCP 面，
//! "再剔除"属于运行时钳制（fail-open 风险正是本设计要消灭的）；而 provider
//! 直连路径发给模型的工具表 **只能** 来自 `OneShotTurnRequest::tools`——安全
//! 边界由构造保证：未传入的工具在注册表中根本不存在，也没有任何 handler 可被
//! 调用。该入口不含任何客服（cs）概念，可被任意域复用。
//!
//! 无状态：每回合新建请求、跑完丢弃；不触碰 AgentRuntimeRegistry / workspace
//! lease / 会话持久化。

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use nomi_providers::{LlmProvider, create_provider};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason};
use nomi_types::tool::ToolDef;
use nomifun_common::AppError;
use nomifun_model_invoke::ModelInvokeService;

use crate::factory::provider_config::resolve_provider_config;

/// Max tokens for a one-shot reply.
const ONE_SHOT_MAX_TOKENS: u32 = 4096;
/// Upper bound on tool rounds inside one turn, so a looping model cannot spin
/// forever below the wall-clock timeout.
const MAX_TOOL_ROUNDS: usize = 8;

/// A single whitelisted tool for a one-shot turn. The handler is the ONLY
/// executable surface: the engine never resolves tool names against any other
/// registry.
pub struct OneShotTool {
    pub name: String, pub description: String,
    pub input_schema: serde_json::Value,
    pub handler: Arc<dyn Fn(serde_json::Value) -> futures::future::BoxFuture<'static, Result<String, String>> + Send + Sync>,
}

/// One stateless engine turn: provider + prompt + window + whitelisted tools.
pub struct OneShotTurnRequest {
    pub provider: nomifun_common::ProviderWithModel,
    pub system_prompt: String,
    pub history: Vec<(String /*"user"|"assistant"*/, String)>,
    pub user_text: String,
    pub tools: Vec<OneShotTool>,
    pub timeout_secs: u64,
}

/// Dependencies needed to resolve provider credentials into a live LLM
/// client. Fixed after exploration (Task 3 Step 1): the same triple the
/// crate's other standalone completion surfaces use
/// ([`crate::knowledge_completer::LiveKnowledgeCompleter`], companion
/// learner).
pub struct OneShotDeps {
    pub model_invoke: Arc<ModelInvokeService>,
    /// Directory handed to `Config::resolve` as project dir. The one-shot
    /// engine registers no filesystem tools, so nothing is read or written
    /// here; it only anchors provider config resolution.
    pub workspace: PathBuf,
}

/// Run one isolated engine turn and return the final reply text.
///
/// The session is built fresh from the request: no skills, no MCP, no
/// filesystem tools, no workspace mount — the tool table contains exactly
/// `req.tools`. The whole turn (every model round and tool execution) races
/// `req.timeout_secs`; on expiry the turn is abandoned and
/// `AppError::Internal("one-shot turn timed out")` is returned.
pub async fn run_one_shot_turn(services: &OneShotDeps, req: OneShotTurnRequest) -> Result<String, AppError> {
    let config = resolve_provider_config(
        services.model_invoke.as_ref(),
        &req.provider.provider_id,
        &req.provider.model,
        &services.workspace,
    )
    .await?;
    let provider: Arc<dyn LlmProvider> = create_provider(&config);
    run_one_shot_turn_with_provider(provider, req).await
}

/// Provider-injected core of [`run_one_shot_turn`] (tests stub the LLM here).
pub(crate) async fn run_one_shot_turn_with_provider(
    provider: Arc<dyn LlmProvider>,
    req: OneShotTurnRequest,
) -> Result<String, AppError> {
    let timeout = std::time::Duration::from_secs(req.timeout_secs);
    match tokio::time::timeout(timeout, tool_loop(provider, req)).await {
        Ok(result) => result,
        Err(_) => Err(AppError::Internal("one-shot turn timed out".into())),
    }
}

async fn tool_loop(
    provider: Arc<dyn LlmProvider>,
    req: OneShotTurnRequest,
) -> Result<String, AppError> {
    // The tool defs sent to the model and the handler table are derived from
    // the SAME whitelist; there is no other tool source in this code path.
    let tool_defs: Vec<ToolDef> = req
        .tools
        .iter()
        .map(|tool| ToolDef {
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
            deferred: false,
        })
        .collect();

    let mut messages: Vec<Message> = Vec::with_capacity(req.history.len() + 1);
    for (role, text) in &req.history {
        let role = if role == "assistant" { Role::Assistant } else { Role::User };
        messages.push(Message::new(role, vec![ContentBlock::Text { text: text.clone() }]));
    }
    messages.push(Message::new(
        Role::User,
        vec![ContentBlock::Text { text: req.user_text.clone() }],
    ));

    for _round in 0..MAX_TOOL_ROUNDS {
        let request = LlmRequest {
            model: req.provider.model.clone(),
            system: req.system_prompt.clone(),
            messages: messages.clone(),
            tools: tool_defs.clone(),
            max_tokens: Some(ONE_SHOT_MAX_TOKENS),
            thinking: None,
            reasoning_effort: None,
            retain_provider_round: false,
        };
        let mut rx = provider
            .stream(&request)
            .await
            .map_err(|error| AppError::BadGateway(format!("LLM provider error: {error}")))?;

        let mut text = String::new();
        let mut thinking = String::new();
        let mut thinking_signature: Option<String> = None;
        let mut tool_uses: Vec<(String, String, serde_json::Value, Option<serde_json::Value>)> =
            Vec::new();
        let mut terminal: Option<StopReason> = None;
        let mut saw_truncated_tool_use = false;
        while let Some(event) = rx.recv().await {
            if terminal.is_some() {
                return Err(AppError::BadGateway(
                    "LLM stream protocol violation: event emitted after terminal Done".into(),
                ));
            }
            match event {
                LlmEvent::TextDelta(delta) => text.push_str(&delta),
                LlmEvent::ThinkingDelta(delta) => thinking.push_str(&delta),
                LlmEvent::ThinkingSignature(signature) => {
                    thinking_signature = Some(signature);
                }
                LlmEvent::ToolUse { id, name, input, extra } => {
                    tool_uses.push((id, name, input, extra));
                }
                LlmEvent::ToolUseTruncated { .. } => {
                    // This is pass-level evidence, never an executable call.
                    // Keep consuming only to validate the provider terminal;
                    // the adjudication below fails regardless of draft text.
                    saw_truncated_tool_use = true;
                }
                LlmEvent::Done { stop_reason, .. } => {
                    terminal = Some(stop_reason);
                }
                LlmEvent::Error(message) => {
                    return Err(AppError::BadGateway(format!("LLM stream error: {message}")));
                }
                LlmEvent::ProviderRoundId(_) => {
                    return Err(AppError::BadGateway(
                        "LLM stream protocol violation: provider round id was emitted for a non-retainable one-shot request"
                            .into(),
                    ));
                }
                LlmEvent::ToolUseDelta { .. } => {}
            }
        }

        let stop_reason = terminal.ok_or_else(|| {
            AppError::BadGateway(
                "LLM stream ended without a terminal Done event".into(),
            )
        })?;
        if saw_truncated_tool_use {
            return Err(AppError::BadGateway(
                "LLM output was truncated while generating a one-shot tool call".into(),
            ));
        }
        match stop_reason {
            StopReason::EndTurn => {
                if !tool_uses.is_empty() {
                    return Err(AppError::BadGateway(
                        "LLM stream protocol violation: EndTurn contained tool calls".into(),
                    ));
                }
                return finish(text);
            }
            StopReason::ToolUse => {
                if tool_uses.is_empty() {
                    return Err(AppError::BadGateway(
                        "LLM stream protocol violation: ToolUse contained no complete tool calls"
                            .into(),
                    ));
                }
            }
            StopReason::MaxTokens => {
                return Err(AppError::BadGateway(
                    "LLM output was truncated before the one-shot turn completed".into(),
                ));
            }
            StopReason::Refusal => {
                return Err(AppError::BadGateway(
                    "LLM refused the one-shot turn".into(),
                ));
            }
            StopReason::MaxTurns => {
                return Err(AppError::BadGateway(
                    "LLM stream protocol violation: provider emitted engine-only MaxTurns"
                        .into(),
                ));
            }
        }

        // Assistant message replaying the model's tool calls (and any text).
        let mut assistant_blocks: Vec<ContentBlock> = Vec::new();
        if !thinking.is_empty() || thinking_signature.is_some() {
            assistant_blocks.push(ContentBlock::Thinking {
                thinking,
                signature: thinking_signature,
            });
        }
        if !text.is_empty() {
            assistant_blocks.push(ContentBlock::Text { text: text.clone() });
        }
        for (id, name, input, extra) in &tool_uses {
            assistant_blocks.push(ContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
                extra: extra.clone(),
            });
        }
        messages.push(Message::new(Role::Assistant, assistant_blocks));

        // Execute strictly against the whitelist: an unknown name gets an
        // error result and NEVER reaches any other execution surface.
        let mut result_blocks: Vec<ContentBlock> = Vec::new();
        for (id, name, input, _extra) in tool_uses {
            let outcome = match req.tools.iter().find(|tool| tool.name == name) {
                Some(tool) => (tool.handler)(input).await,
                None => Err(format!("tool '{name}' is not available in this session")),
            };
            let (content, is_error) = match outcome {
                Ok(content) => (content, false),
                Err(message) => (message, true),
            };
            result_blocks.push(ContentBlock::ToolResult {
                tool_use_id: id,
                content,
                is_error,
                images: Vec::new(),
            });
        }
        messages.push(Message::new(Role::User, result_blocks));
    }

    Err(AppError::BadGateway(format!(
        "one-shot turn exceeded {MAX_TOOL_ROUNDS} tool rounds without a final answer"
    )))
}

fn finish(text: String) -> Result<String, AppError> {
    if text.trim().is_empty() {
        Err(AppError::BadGateway(
            "LLM stream ended without producing a response".into(),
        ))
    } else {
        Ok(text)
    }
}

/// Convenience constructor for a boxed async tool handler.
pub fn one_shot_handler<F, Fut>(
    handler: F,
) -> Arc<dyn Fn(serde_json::Value) -> BoxFuture<'static, Result<String, String>> + Send + Sync>
where
    F: Fn(serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    Arc::new(move |input| Box::pin(handler(input)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomi_providers::ProviderError;
    use nomi_types::message::TokenUsage;
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    fn request(tools: Vec<OneShotTool>, timeout_secs: u64) -> OneShotTurnRequest {
        OneShotTurnRequest {
            provider: nomifun_common::ProviderWithModel {
                provider_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
                model: "test-model".into(),
                use_model: None,
            },
            system_prompt: "you are a test".into(),
            history: vec![("user".into(), "earlier".into()), ("assistant".into(), "ok".into())],
            user_text: "hello".into(),
            tools,
            timeout_secs,
        }
    }

    fn tool(name: &str, calls: Arc<Mutex<Vec<serde_json::Value>>>) -> OneShotTool {
        OneShotTool {
            name: name.into(),
            description: format!("test tool {name}"),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } }
            }),
            handler: one_shot_handler(move |input| {
                let calls = Arc::clone(&calls);
                async move {
                    calls.lock().unwrap().push(input);
                    Ok("tool says 42".to_owned())
                }
            }),
        }
    }

    /// Scripted fake provider: each `stream` call pops the next script entry;
    /// every observed request (tool names + messages) is recorded.
    struct ScriptedProvider {
        script: Mutex<Vec<Vec<LlmEvent>>>,
        seen_tool_names: Mutex<Vec<Vec<String>>>,
        seen_messages: Mutex<Vec<Vec<Message>>>,
    }

    impl ScriptedProvider {
        fn new(script: Vec<Vec<LlmEvent>>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(script),
                seen_tool_names: Mutex::new(Vec::new()),
                seen_messages: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn stream(
            &self,
            request: &LlmRequest,
        ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
            self.seen_tool_names
                .lock()
                .unwrap()
                .push(request.tools.iter().map(|tool| tool.name.clone()).collect());
            self.seen_messages.lock().unwrap().push(request.messages.clone());
            let mut script = self.script.lock().unwrap();
            if script.is_empty() {
                return Err(ProviderError::Connection("script exhausted".into()));
            }
            let events = script.remove(0);
            let (tx, rx) = mpsc::channel(events.len().max(1));
            tokio::spawn(async move {
                for event in events {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
            Ok(rx)
        }
    }

    fn done(stop_reason: StopReason) -> LlmEvent {
        LlmEvent::Done { stop_reason, usage: TokenUsage::default() }
    }

    /// 安全不变量：发给模型的工具注册面恰等于传入集合——每一轮都如此。
    #[tokio::test]
    async fn tool_registry_is_exactly_the_passed_whitelist() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = ScriptedProvider::new(vec![
            vec![
                LlmEvent::ThinkingDelta("opaque reasoning preview".into()),
                LlmEvent::ThinkingSignature("responses:v1:encrypted".into()),
                LlmEvent::ToolUse {
                    id: "call_1".into(),
                    name: "knowledge_search".into(),
                    input: serde_json::json!({"query": "退货"}),
                    extra: None,
                },
                done(StopReason::ToolUse),
            ],
            vec![LlmEvent::TextDelta("final answer".into()), done(StopReason::EndTurn)],
        ]);
        let tools = vec![
            tool("knowledge_search", Arc::clone(&calls)),
            tool("knowledge_read", Arc::clone(&calls)),
            tool("cs_notes_search", Arc::clone(&calls)),
        ];

        let text = run_one_shot_turn_with_provider(provider.clone(), request(tools, 30))
            .await
            .unwrap();
        assert_eq!(text, "final answer");

        let seen = provider.seen_tool_names.lock().unwrap();
        assert_eq!(seen.len(), 2, "two model rounds");
        for round in seen.iter() {
            assert_eq!(
                round,
                &vec![
                    "knowledge_search".to_owned(),
                    "knowledge_read".to_owned(),
                    "cs_notes_search".to_owned()
                ],
                "tool registry must be exactly the whitelist on every round"
            );
        }

        // The handler ran with the model's input and its result was fed back.
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[serde_json::json!({"query": "退货"})]
        );
        let rounds = provider.seen_messages.lock().unwrap();
        let followup = &rounds[1];
        let assistant = followup
            .iter()
            .rev()
            .find(|message| message.role == Role::Assistant)
            .expect("the tool round is replayed");
        assert!(matches!(
            &assistant.content[0],
            ContentBlock::Thinking { thinking, signature }
                if thinking == "opaque reasoning preview"
                    && signature.as_deref() == Some("responses:v1:encrypted")
        ), "unexpected tool-round assistant content: {:?}", assistant.content);
        let last = followup.last().unwrap();
        assert!(matches!(
            &last.content[0],
            ContentBlock::ToolResult { content, is_error: false, .. } if content == "tool says 42"
        ));
    }

    /// 空白名单 ⇒ 模型看到零工具（构造级隔离，无任何默认注册）。
    #[tokio::test]
    async fn empty_whitelist_registers_zero_tools() {
        let provider = ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta("plain".into()),
            done(StopReason::EndTurn),
        ]]);
        let text = run_one_shot_turn_with_provider(provider.clone(), request(vec![], 30))
            .await
            .unwrap();
        assert_eq!(text, "plain");
        assert_eq!(provider.seen_tool_names.lock().unwrap()[0], Vec::<String>::new());
    }

    #[tokio::test]
    async fn only_a_well_formed_end_turn_is_deliverable() {
        let cases = vec![
            (
                "max tokens with draft text",
                vec![LlmEvent::TextDelta("draft".into()), done(StopReason::MaxTokens)],
                "truncated",
            ),
            (
                "refusal with visible text",
                vec![LlmEvent::TextDelta("cannot comply".into()), done(StopReason::Refusal)],
                "refused",
            ),
            (
                "provider-only max turns",
                vec![LlmEvent::TextDelta("draft".into()), done(StopReason::MaxTurns)],
                "MaxTurns",
            ),
            (
                "ToolUse without a complete call",
                vec![done(StopReason::ToolUse)],
                "no complete tool calls",
            ),
            (
                "text without Done",
                vec![LlmEvent::TextDelta("unterminated".into())],
                "without a terminal Done",
            ),
            (
                "event after Done",
                vec![
                    LlmEvent::TextDelta("committed?".into()),
                    done(StopReason::EndTurn),
                    LlmEvent::TextDelta("poison".into()),
                ],
                "after terminal Done",
            ),
            (
                "truncated tool arguments",
                vec![
                    LlmEvent::ToolUseTruncated {
                        id: "call_cut".into(),
                        name: "knowledge_search".into(),
                        argument_bytes: 21,
                    },
                    done(StopReason::MaxTokens),
                ],
                "truncated while generating a one-shot tool call",
            ),
            (
                "EndTurn carrying a complete tool call",
                vec![
                    LlmEvent::ToolUse {
                        id: "call_wrong_terminal".into(),
                        name: "knowledge_search".into(),
                        input: serde_json::json!({"query": "x"}),
                        extra: None,
                    },
                    done(StopReason::EndTurn),
                ],
                "EndTurn contained tool calls",
            ),
        ];

        for (label, events, expected) in cases {
            let provider = ScriptedProvider::new(vec![events]);
            let error = run_one_shot_turn_with_provider(provider, request(vec![], 30))
                .await
                .expect_err(label);
            assert!(
                matches!(&error, AppError::BadGateway(message) if message.contains(expected)),
                "{label}: {error:?}"
            );
        }
    }

    /// 模型索要白名单之外的工具 ⇒ 错误 ToolResult，绝不触达任何执行面。
    #[tokio::test]
    async fn unknown_tool_name_fails_closed() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let provider = ScriptedProvider::new(vec![
            vec![
                LlmEvent::ToolUse {
                    id: "call_1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"cmd": "rm -rf /"}),
                    extra: None,
                },
                done(StopReason::ToolUse),
            ],
            vec![LlmEvent::TextDelta("sorry".into()), done(StopReason::EndTurn)],
        ]);
        let tools = vec![tool("knowledge_search", Arc::clone(&calls))];
        let text = run_one_shot_turn_with_provider(provider.clone(), request(tools, 30))
            .await
            .unwrap();
        assert_eq!(text, "sorry");
        assert!(calls.lock().unwrap().is_empty(), "no whitelisted handler may run");
        let rounds = provider.seen_messages.lock().unwrap();
        let followup = rounds[1].last().unwrap();
        assert!(matches!(
            &followup.content[0],
            ContentBlock::ToolResult { is_error: true, content, .. }
                if content.contains("not available")
        ));
    }

    /// 超时路径：整回合被 `timeout_secs` 硬包裹。
    #[tokio::test]
    async fn timeout_returns_internal_error() {
        struct StallingProvider;
        #[async_trait::async_trait]
        impl LlmProvider for StallingProvider {
            async fn stream(
                &self,
                _request: &LlmRequest,
            ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
                let (tx, rx) = mpsc::channel(1);
                tokio::spawn(async move {
                    // Keep the sender alive forever without sending.
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                    drop(tx);
                });
                Ok(rx)
            }
        }

        let error = run_one_shot_turn_with_provider(Arc::new(StallingProvider), request(vec![], 1))
            .await
            .unwrap_err();
        assert!(matches!(
            &error,
            AppError::Internal(message) if message == "one-shot turn timed out"
        ));
    }

    /// 流错误映射为 BadGateway。
    #[tokio::test]
    async fn stream_error_maps_to_bad_gateway() {
        let provider =
            ScriptedProvider::new(vec![vec![LlmEvent::Error("rate limited".into())]]);
        let error = run_one_shot_turn_with_provider(provider, request(vec![], 30))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::BadGateway(_)));
    }

    /// history 角色映射与末尾 user_text 的窗口组装。
    #[tokio::test]
    async fn history_and_user_text_form_the_window() {
        let provider = ScriptedProvider::new(vec![vec![
            LlmEvent::TextDelta("ok".into()),
            done(StopReason::EndTurn),
        ]]);
        run_one_shot_turn_with_provider(provider.clone(), request(vec![], 30))
            .await
            .unwrap();
        let rounds = provider.seen_messages.lock().unwrap();
        let window = &rounds[0];
        assert_eq!(window.len(), 3);
        assert_eq!(window[0].role, Role::User);
        assert_eq!(window[1].role, Role::Assistant);
        assert_eq!(window[2].role, Role::User);
        assert!(matches!(
            &window[2].content[0],
            ContentBlock::Text { text } if text == "hello"
        ));
    }
}
