//! Sidecar backup-model caller. Resolves the effective bypass provider/model
//! (per-watch override → the session's own model), then runs a one-shot
//! completion and parses the strict-JSON decision (with one retry).
//!
//! The provider call is behind the `Completer` trait so the supervisor tests can
//! inject canned responses without a live provider; the production impl wraps
//! `nomifun_ai_agent::{resolve_provider_config, one_shot_completion}`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::{one_shot_completion, resolve_provider_config, user_message};
use nomifun_api_types::{BypassModelRef, DecisionStrategy};
use nomifun_common::ProviderId;
use nomifun_model_invoke::ModelInvokeService;

use crate::prompt::{SIDECAR_SYSTEM, SidecarDecision, build_open_question_prompt, build_user_prompt, parse_decision};
use crate::signal::StallClass;

const SIDECAR_MAX_TOKENS: u32 = 1024;

/// The provider call seam. Production wraps the real provider; tests inject.
#[async_trait]
pub trait Completer: Send + Sync {
    /// Run a system+user completion against `provider_id`/`model`. Returns the
    /// assembled text, or `Err(())` on any provider failure (→ rule fallback).
    async fn complete(&self, provider_id: &str, model: &str, system: &str, user: &str) -> Result<String, ()>;
}

/// Production completer: provider row → nomi Config → one-shot completion.
pub struct LiveCompleter {
    pub model_invoke: Arc<ModelInvokeService>,
    pub workspace: PathBuf,
}

#[async_trait]
impl Completer for LiveCompleter {
    async fn complete(&self, provider_id: &str, model: &str, system: &str, user: &str) -> Result<String, ()> {
        ProviderId::parse(provider_id).map_err(|error| {
            tracing::warn!(provider_id, error = %error, "IDMM sidecar rejected a non-canonical provider id");
        })?;
        let cfg = resolve_provider_config(
            self.model_invoke.as_ref(),
            provider_id,
            model,
            &self.workspace,
        )
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "IDMM sidecar provider config resolution failed");
        })?;
        one_shot_completion(&cfg, system, vec![user_message(user)], SIDECAR_MAX_TOKENS)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "IDMM sidecar completion failed");
            })
    }
}

/// Outcome of a sidecar decision attempt.
#[derive(Debug, Clone)]
pub struct SidecarOutcome {
    /// The parsed decision, if the model produced valid JSON.
    pub decision: Option<SidecarDecision>,
    /// True if the provider call itself failed (vs. produced unparseable text).
    pub provider_failed: bool,
    /// The `(provider_id, model)` the sidecar resolved and used (or attempted,
    /// on a provider failure). `None` only when no backup was resolvable at all.
    /// Lets the caller record the audit `bypass_model` without re-resolving.
    pub resolved: Option<(String, String)>,
}

/// An open-question answer request (D6): the question text + its char cap. When
/// present, [`SidecarClient::decide`] uses the free-text answer prompt instead
/// of the option/permission prompt.
pub struct OpenQuestionAsk<'a> {
    pub question: &'a str,
    pub max_answer_chars: u32,
}

/// Resolves the bypass model and runs sidecar decisions.
pub struct SidecarClient {
    completer: Arc<dyn Completer>,
}

impl SidecarClient {
    pub fn new(completer: Arc<dyn Completer>) -> Self {
        Self { completer }
    }

    /// Resolve effective `(provider_id, model)` from the watch's `bypass_model`.
    /// An empty `model` means "the provider's default".
    ///
    /// This used to fall through to three global `client_preferences` defaults
    /// (backup provider / backup model / steering prompt). That tier is gone: a
    /// watch either names its own bypass model or borrows the supervised
    /// session's, and a watch with no policy text falls back to the built-in
    /// conservative one in [`build_user_prompt`]. Both remaining sources are
    /// visible where the watch is configured, so there is no longer a setting
    /// that silently changes how every session decides.
    pub fn resolve_backup(&self, bypass: &BypassModelRef) -> Option<(String, String)> {
        let provider_id = ProviderId::parse(bypass.provider_id.as_deref()?)
            .ok()?
            .into_string();
        let model = match &bypass.model {
            Some(m) if !m.is_empty() => m.clone(),
            _ => String::new(),
        };
        Some((provider_id, model))
    }

    /// Whether a backup provider is resolvable for this watch's bypass model —
    /// used by validation + the `sidecar_provider_resolved` state flag.
    pub fn backup_resolvable(&self, bypass: &BypassModelRef) -> bool {
        self.resolve_backup(bypass).is_some()
    }

    /// Run one sidecar decision pass.
    ///
    /// `bypass` is the active watch's bypass-model selection. `strategy` drives
    /// the prompt's policy block (tendency / freeform / never-destructive).
    /// `fallback` is the supervised session's own `(provider_id, model)` — used
    /// when the watch names no bypass model, so the model tier works
    /// out-of-the-box on a plain desktop chat (the session's own model becomes
    /// the bypass model). `open_question`, when `Some`, switches to the free-text
    /// answer prompt (D6).
    #[allow(clippy::too_many_arguments)]
    pub async fn decide(
        &self,
        bypass: &BypassModelRef,
        strategy: &DecisionStrategy,
        class: StallClass,
        detail: &str,
        context: &str,
        fallback: Option<(String, String)>,
        open_question: Option<OpenQuestionAsk<'_>>,
    ) -> SidecarOutcome {
        let resolved = match self.resolve_backup(bypass) {
            Some(pm) => Some(pm),
            None => fallback.filter(|(provider_id, _)| ProviderId::parse(provider_id).is_ok()),
        };
        let Some((provider_id, model)) = resolved else {
            return SidecarOutcome {
                decision: None,
                provider_failed: true,
                resolved: None,
            };
        };
        let used = (provider_id.clone(), model.clone());
        let user = match &open_question {
            Some(oq) => build_open_question_prompt(strategy, oq.question, context, oq.max_answer_chars),
            None => build_user_prompt(strategy, class, detail, context),
        };

        // First attempt.
        let raw = match self
            .completer
            .complete(&provider_id, &model, SIDECAR_SYSTEM, &user)
            .await
        {
            Ok(r) => r,
            Err(()) => {
                return SidecarOutcome {
                    decision: None,
                    provider_failed: true,
                    resolved: Some(used),
                };
            }
        };
        if let Some(d) = parse_decision(&raw) {
            return SidecarOutcome {
                decision: Some(d),
                provider_failed: false,
                resolved: Some(used),
            };
        }

        // One retry, nudging for strict JSON.
        let retry_user = format!("{user}\n\nReturn ONLY the JSON object, nothing else.");
        match self
            .completer
            .complete(&provider_id, &model, SIDECAR_SYSTEM, &retry_user)
            .await
        {
            Ok(r2) => SidecarOutcome {
                decision: parse_decision(&r2),
                provider_failed: false,
                resolved: Some(used),
            },
            Err(()) => SidecarOutcome {
                decision: None,
                provider_failed: true,
                resolved: Some(used),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::{BypassModelRef, DecisionStrategy};
    use std::sync::Mutex;

    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const WATCH_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const SESSION_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000003";

    // ── Mock completer: scripted responses ──
    struct ScriptedCompleter {
        responses: Mutex<Vec<Result<String, ()>>>,
        calls: Mutex<u32>,
    }
    impl ScriptedCompleter {
        fn new(responses: Vec<Result<String, ()>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                calls: Mutex::new(0),
            }
        }
    }

    struct CapturingCompleter {
        last_target: Mutex<Option<(String, String)>>,
    }

    #[async_trait]
    impl Completer for CapturingCompleter {
        async fn complete(&self, p: &str, m: &str, _s: &str, _user: &str) -> Result<String, ()> {
            *self.last_target.lock().unwrap() = Some((p.to_string(), m.to_string()));
            Ok(r#"{"action":"retry","confidence":0.9}"#.into())
        }
    }

    #[async_trait]
    impl Completer for ScriptedCompleter {
        async fn complete(&self, _p: &str, _m: &str, _s: &str, _u: &str) -> Result<String, ()> {
            *self.calls.lock().unwrap() += 1;
            let mut r = self.responses.lock().unwrap();
            if r.is_empty() { Err(()) } else { r.remove(0) }
        }
    }

    fn bypass() -> BypassModelRef {
        BypassModelRef {
            provider_id: Some(PROVIDER_ID.into()),
            model: Some("m1".into()),
        }
    }

    fn strat() -> DecisionStrategy {
        DecisionStrategy::default()
    }

    #[test]
    fn resolve_backup_reads_the_watch_and_nothing_else() {
        let client = SidecarClient::new(Arc::new(ScriptedCompleter::new(vec![])));

        let watch = BypassModelRef {
            provider_id: Some(WATCH_PROVIDER_ID.into()),
            model: Some("watch_model".into()),
        };
        assert_eq!(
            client.resolve_backup(&watch),
            Some((WATCH_PROVIDER_ID.into(), "watch_model".into()))
        );

        // A provider with no model means "the provider's default", not "look
        // somewhere else for a model".
        let provider_only = BypassModelRef {
            provider_id: Some(WATCH_PROVIDER_ID.into()),
            model: None,
        };
        assert_eq!(
            client.resolve_backup(&provider_only),
            Some((WATCH_PROVIDER_ID.into(), String::new()))
        );
    }

    #[test]
    fn resolve_backup_none_when_the_watch_names_no_provider() {
        // There is no global-default tier to fall through to: an unset watch
        // resolves to nothing, and `decide` then borrows the session's model.
        let client = SidecarClient::new(Arc::new(ScriptedCompleter::new(vec![])));
        assert!(client.resolve_backup(&BypassModelRef::default()).is_none());
        assert!(!client.backup_resolvable(&BypassModelRef::default()));
    }

    #[tokio::test]
    async fn an_unset_watch_borrows_the_supervised_session_model() {
        let comp = Arc::new(CapturingCompleter {
            last_target: Mutex::new(None),
        });
        let client = SidecarClient::new(comp.clone());

        let out = client
            .decide(
                &BypassModelRef::default(),
                &strat(),
                StallClass::Decision,
                "pick an option",
                "ctx",
                Some((SESSION_PROVIDER_ID.into(), "session_model".into())),
                None,
            )
            .await;

        assert!(!out.provider_failed);
        assert_eq!(
            *comp.last_target.lock().unwrap(),
            Some((SESSION_PROVIDER_ID.into(), "session_model".into()))
        );
    }

    #[tokio::test]
    async fn an_unset_watch_with_no_session_model_fails_the_provider_call() {
        let comp = Arc::new(ScriptedCompleter::new(vec![]));
        let client = SidecarClient::new(comp.clone());
        let out = client
            .decide(
                &BypassModelRef::default(),
                &strat(),
                StallClass::Decision,
                "pick an option",
                "ctx",
                None,
                None,
            )
            .await;
        assert!(out.provider_failed);
        assert!(out.resolved.is_none());
        assert_eq!(*comp.calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn sidecar_returns_parsed_decision() {
        let comp = Arc::new(ScriptedCompleter::new(vec![Ok(
            r#"{"action":"retry","confidence":0.9,"reason":"transient"}"#.into(),
        )]));
        let client = SidecarClient::new(comp);
        let out = client
            .decide(&bypass(), &strat(), StallClass::ProviderError, "500", "ctx", None, None)
            .await;
        assert!(!out.provider_failed);
        assert_eq!(out.decision.unwrap().action, "retry");
    }

    #[tokio::test]
    async fn sidecar_retries_once_on_garbage_then_parses() {
        let comp = Arc::new(ScriptedCompleter::new(vec![
            Ok("sorry, I cannot".into()),
            Ok(r#"{"action":"send_text","text":"continue"}"#.into()),
        ]));
        let client = SidecarClient::new(comp.clone());
        let out = client
            .decide(&bypass(), &strat(), StallClass::Idle, "idle", "ctx", None, None)
            .await;
        assert!(!out.provider_failed);
        assert_eq!(out.decision.unwrap().action, "send_text");
        assert_eq!(*comp.calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn sidecar_garbage_twice_yields_no_decision() {
        let comp = Arc::new(ScriptedCompleter::new(vec![Ok("nope".into()), Ok("still nope".into())]));
        let client = SidecarClient::new(comp);
        let out = client
            .decide(&bypass(), &strat(), StallClass::Idle, "idle", "ctx", None, None)
            .await;
        assert!(!out.provider_failed);
        assert!(out.decision.is_none());
    }

    #[tokio::test]
    async fn sidecar_provider_error_sets_provider_failed() {
        let comp = Arc::new(ScriptedCompleter::new(vec![Err(())]));
        let client = SidecarClient::new(comp);
        let out = client
            .decide(&bypass(), &strat(), StallClass::ProviderError, "500", "ctx", None, None)
            .await;
        assert!(out.provider_failed);
        assert!(out.decision.is_none());
    }

    #[tokio::test]
    async fn sidecar_open_question_returns_answer_text() {
        // D6: an open-question ask uses the free-text prompt and the model
        // replies with answer_text.
        let comp = Arc::new(ScriptedCompleter::new(vec![Ok(
            r#"{"action":"answer_text","text":"用 LRU + 30 分钟 TTL","confidence":0.8}"#.into(),
        )]));
        let client = SidecarClient::new(comp);
        let out = client
            .decide(
                &bypass(),
                &strat(),
                StallClass::OpenQuestion,
                "open question: 缓存怎么设计",
                "ctx",
                None,
                Some(OpenQuestionAsk {
                    question: "你希望缓存怎么设计？",
                    max_answer_chars: 600,
                }),
            )
            .await;
        assert!(!out.provider_failed);
        let d = out.decision.unwrap();
        assert_eq!(d.action, "answer_text");
        assert_eq!(d.text, "用 LRU + 30 分钟 TTL");
    }
}
