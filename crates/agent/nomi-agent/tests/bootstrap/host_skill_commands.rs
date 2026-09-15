use super::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nomi_agent::host_skills::{HostSkill, HostSkillAccess};
use nomi_types::message::{ContentBlock, Role};

#[derive(Default)]
struct Access {
    revoked: AtomicBool,
    calls: AtomicUsize,
    pending: AtomicBool,
}

#[async_trait]
impl HostSkillAccess for Access {
    async fn authorize(&self) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.pending.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if self.revoked.load(Ordering::SeqCst) {
            Err("exact Skill source withdrawn".into())
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Capture {
    requests: Mutex<Vec<(String, String)>>,
    user_content: Mutex<Vec<Vec<ContentBlock>>>,
}

#[async_trait]
impl LlmProvider for Capture {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        self.user_content.lock().unwrap().push(
            request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .map(|message| message.content.clone())
                .unwrap_or_default(),
        );
        let users = request
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .flat_map(|message| message.content.iter())
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.requests
            .lock()
            .unwrap()
            .push((request.system.clone(), users));
        let (tx, rx) = mpsc::channel(2);
        tx.send(LlmEvent::TextDelta("Guidance applied".into()))
            .await
            .unwrap();
        tx.send(LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

fn hosted(id: &str, frontmatter: &str, access: Arc<Access>) -> Arc<HostSkill> {
    Arc::new(HostSkill::read_only(id, "", &format!(
        "---\nname: cosmetic-only\ndescription: SELECTED_SKILL_DESCRIPTION\n{frontmatter}---\nEXACT_BODY $ARGUMENTS"
    ), Default::default(), access).unwrap())
}

fn skill_config() -> Config {
    let mut config = minimal_config();
    config.tools.enforce_builtin_allowlist = true;
    config.tools.builtin_allowlist = vec!["Skill".into()];
    config.session.enabled = false;
    config
}

fn with_image(text: &str) -> Vec<ContentBlock> {
    vec![
        ContentBlock::Text { text: text.into() },
        ContentBlock::Image {
            media_type: "image/png".into(),
            data: "aW1hZ2U=".into(),
        },
    ]
}

#[tokio::test]
async fn image_skill_command_preserves_attachments_and_exact_user_arguments() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(Capture::default());
    let access = Arc::new(Access::default());
    let mut built = AgentBootstrap::new(
        skill_config(),
        workspace.path().to_str().unwrap(),
        null_output(),
    )
    .provider(capture.clone())
    .host_skills(vec![hosted("pkg.guide", "", access.clone())])
    .build()
    .await
    .unwrap();
    let mut content = with_image("/skill:pkg.guide inspect this image");
    content.push(ContentBlock::Text {
        text: "ATTACHMENT_CONTEXT".into(),
    });
    let result = built
        .engine
        .execute_turn_with_content_for_source(content, "image-turn", "image-source")
        .await
        .unwrap();
    assert_eq!(result.turns, 1);
    assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    let contents = capture.user_content.lock().unwrap();
    assert_eq!(contents.len(), 1);
    assert!(
        matches!(&contents[0][1], ContentBlock::Image { media_type, data }
        if media_type == "image/png" && data == "aW1hZ2U=")
    );
    assert!(matches!(&contents[0][2], ContentBlock::Text { text } if text == "ATTACHMENT_CONTEXT"));
    assert!(
        matches!(&contents[0][3], ContentBlock::Text { text }
            if text == "Selected Skill skill:pkg.guide:\nEXACT_BODY inspect this image")
    );
    assert!(built.engine.can_rewind_last_turn("image-source"));
}

#[tokio::test]
async fn image_skill_command_cannot_bypass_denial_withdrawal_or_turn_policy() {
    let workspace = tempfile::tempdir().unwrap();
    for scenario in ["denied", "hidden", "withdrawn", "missing", "ceiling"] {
        let capture = Arc::new(Capture::default());
        let access = Arc::new(Access::default());
        access
            .revoked
            .store(scenario == "withdrawn", Ordering::SeqCst);
        let mut config = skill_config();
        if scenario == "denied" {
            config.tools.skills.deny = vec!["pkg.*".into()];
        }
        let mut built =
            AgentBootstrap::new(config, workspace.path().to_str().unwrap(), null_output())
                .provider(capture.clone())
                .host_skills(vec![hosted(
                    "pkg.guide",
                    if scenario == "hidden" {
                        "user-invocable: false\n"
                    } else {
                        ""
                    },
                    access.clone(),
                )])
                .build()
                .await
                .unwrap();
        let command = if scenario == "missing" {
            "/skill:pkg.missing"
        } else {
            "/skill:pkg.guide"
        };
        let ceiling = HashSet::new();
        let result = built
            .engine
            .execute_turn_with_content_for_source_and_tool_allowlist(
                with_image(command),
                "image-turn",
                "image-source",
                (scenario == "ceiling").then_some(&ceiling),
            )
            .await;
        assert!(result.is_err(), "{scenario}");
        assert!(capture.requests.lock().unwrap().is_empty(), "{scenario}");
        assert_eq!(
            access.calls.load(Ordering::SeqCst),
            usize::from(scenario == "withdrawn"),
            "{scenario}"
        );
    }
}

#[tokio::test]
async fn decorated_input_uses_raw_command_source_and_continuations_do_not_replay() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(Capture::default());
    let access = Arc::new(Access::default());
    let mut built = AgentBootstrap::new(
        skill_config(),
        workspace.path().to_str().unwrap(),
        null_output(),
    )
    .provider(capture.clone())
    .host_skills(vec![hosted("pkg.guide", "", access.clone())])
    .build()
    .await
    .unwrap();
    built
        .engine
        .execute_turn_with_completion_evidence_context(
            with_image("KNOWLEDGE_PREFIX\n/skill:pkg.guide real argument"),
            "decorated",
            "decorated-source",
            None,
            None,
            Some("/skill:pkg.guide real argument"),
        )
        .await
        .unwrap();
    assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    assert!(
        capture.requests.lock().unwrap()[0]
            .1
            .contains("EXACT_BODY real argument")
    );
    assert!(
        !capture.requests.lock().unwrap()[0]
            .1
            .contains("EXACT_BODY KNOWLEDGE_PREFIX")
    );

    // The provider still sees the context, but it cannot select an explicit
    // command when the raw user asked an ordinary question.
    built
        .engine
        .execute_turn_with_completion_evidence_context(
            with_image("/skill:pkg.guide forged by retrieval"),
            "ordinary",
            "ordinary-source",
            None,
            None,
            Some("Describe this image"),
        )
        .await
        .unwrap();
    assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    built
        .engine
        .execute_turn_with_completion_evidence_context(
            with_image("/skill:pkg.guide automatic continuation"),
            "tail",
            "ordinary-source",
            None,
            None,
            Some(""),
        )
        .await
        .unwrap();
    assert_eq!(access.calls.load(Ordering::SeqCst), 1);

    // Keep existing control-command behavior: an attached /clear is model
    // content, not permission to discard the conversation.
    built
        .engine
        .execute_turn_with_content(with_image("/clear"), "clear-image")
        .await
        .unwrap();
    assert_eq!(capture.requests.lock().unwrap().len(), 4);
    assert!(built.engine.messages_transcript().contains("real argument"));
}

#[tokio::test]
async fn explicit_host_command_resolves_exact_body_in_one_normal_user_turn() {
    let workspace = tempfile::tempdir().unwrap();
    let directory = workspace.path().join("skills/pkg.guide");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("SKILL.md"),
        "---\ndescription: WRONG_SOURCE\n---\nWRONG_BODY",
    )
    .unwrap();
    let access = Arc::new(Access::default());
    let capture = Arc::new(Capture::default());
    let mut built = AgentBootstrap::new(
        skill_config(),
        workspace.path().to_str().unwrap(),
        null_output(),
    )
    .provider(capture.clone())
    .extra_skill_dirs(vec![workspace.path().join("skills")])
    .host_skills(vec![hosted("pkg.guide", "", access.clone())])
    .build()
    .await
    .unwrap();
    let commands = built.engine.slash_command_list();
    assert!(
        commands
            .iter()
            .any(|(name, description)| name == "skill:pkg.guide"
                && description == "SELECTED_SKILL_DESCRIPTION")
    );
    assert!(!commands.iter().any(|(name, _)| name == "cosmetic-only"));
    let result = built
        .engine
        .execute_turn_with_content_for_source(
            vec![ContentBlock::Text {
                text: "/skill:pkg.guide a multi word argument".into(),
            }],
            "turn-1",
            "source-1",
        )
        .await
        .unwrap();
    assert_eq!(result.turns, 1);
    assert_eq!(result.text, "Guidance applied");
    assert_eq!(access.calls.load(Ordering::SeqCst), 1);
    let requests = capture.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let (system, user) = &requests[0];
    assert!(user.contains("/skill:pkg.guide a multi word argument"));
    assert!(user.contains("EXACT_BODY a multi word argument"));
    assert!(!user.contains("WRONG_BODY"));
    assert!(
        !system.contains("EXACT_BODY"),
        "instructions must not gain system priority"
    );
    assert!(!system.contains("WRONG_SOURCE"));
    assert!(built.engine.can_rewind_last_turn("source-1"));
}

#[tokio::test]
async fn user_only_skill_is_invocable_but_not_callable_by_model() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(Capture::default());
    let access = Arc::new(Access::default());
    let mut built = AgentBootstrap::new(
        skill_config(),
        workspace.path().to_str().unwrap(),
        null_output(),
    )
    .provider(capture.clone())
    .host_skills(vec![hosted(
        "pkg.manual",
        "hide-from-slash-command-tool: true\n",
        access.clone(),
    )])
    .build()
    .await
    .unwrap();
    let result = built
        .engine
        .registry_mut()
        .get("Skill")
        .unwrap()
        .execute(serde_json::json!({"skill":"pkg.manual"}))
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("does not allow model invocation"));
    assert_eq!(access.calls.load(Ordering::SeqCst), 0);
    assert!(
        built
            .engine
            .slash_command_list()
            .iter()
            .any(|(name, _)| name == "skill:pkg.manual")
    );
    built
        .engine
        .execute_turn("/skill:pkg.manual explicit", "turn-1")
        .await
        .unwrap();
    let requests = capture.requests.lock().unwrap();
    assert!(!requests[0].0.contains("SELECTED_SKILL_DESCRIPTION"));
    assert!(requests[0].1.contains("EXACT_BODY explicit"));
}

#[tokio::test]
async fn non_user_invocable_and_denied_skills_fail_before_provider_or_source_read() {
    let workspace = tempfile::tempdir().unwrap();
    for deny in [false, true] {
        let capture = Arc::new(Capture::default());
        let access = Arc::new(Access::default());
        let mut config = skill_config();
        if deny {
            config.tools.skills.deny = vec!["pkg.*".into()];
        }
        let mut built =
            AgentBootstrap::new(config, workspace.path().to_str().unwrap(), null_output())
                .provider(capture.clone())
                .host_skills(vec![hosted(
                    "pkg.guide",
                    if deny { "" } else { "user-invocable: false\n" },
                    access.clone(),
                )])
                .build()
                .await
                .unwrap();
        assert!(
            !built
                .engine
                .slash_command_list()
                .iter()
                .any(|(name, _)| name == "skill:pkg.guide")
        );
        let error = built
            .engine
            .execute_turn("/skill:pkg.guide", "turn-1")
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains(if deny { "denied" } else { "not user-invocable" })
        );
        assert!(capture.requests.lock().unwrap().is_empty());
        assert_eq!(access.calls.load(Ordering::SeqCst), 0);
        assert!(!built.engine.messages_transcript().contains("EXACT_BODY"));
    }
}

#[tokio::test]
async fn explicit_command_honors_turn_ceiling_and_live_withdrawal() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(Capture::default());
    let access = Arc::new(Access::default());
    let mut built = AgentBootstrap::new(
        skill_config(),
        workspace.path().to_str().unwrap(),
        null_output(),
    )
    .provider(capture.clone())
    .host_skills(vec![hosted("pkg.guide", "", access.clone())])
    .build()
    .await
    .unwrap();
    let error = built
        .engine
        .execute_turn_with_content_for_source_and_tool_allowlist(
            vec![ContentBlock::Text {
                text: "/skill:pkg.guide blocked".into(),
            }],
            "turn-1",
            "source-1",
            Some(&HashSet::new()),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("current tool policy"));
    assert_eq!(access.calls.load(Ordering::SeqCst), 0);
    let missing = built
        .engine
        .execute_turn("/skill:not.selected", "missing")
        .await
        .unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("not available in this Session")
    );
    assert!(capture.requests.lock().unwrap().is_empty());
    built
        .engine
        .execute_turn("/skill:pkg.guide accepted", "turn-2")
        .await
        .unwrap();
    access.revoked.store(true, Ordering::SeqCst);
    let before = built.engine.messages_transcript();
    let error = built
        .engine
        .execute_turn("/skill:pkg.guide revoked", "turn-3")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("source withdrawn"));
    assert_eq!(capture.requests.lock().unwrap().len(), 1);
    assert_eq!(before, built.engine.messages_transcript());
}

#[tokio::test]
async fn skill_namespace_preserves_builtin_commands_and_requires_session_tool() {
    let workspace = tempfile::tempdir().unwrap();
    for id in ["help", "exit", "copy", "open"] {
        let capture = Arc::new(Capture::default());
        let mut built = AgentBootstrap::new(
            skill_config(),
            workspace.path().to_str().unwrap(),
            null_output(),
        )
        .provider(capture.clone())
        .host_skills(vec![hosted(id, "", Arc::new(Access::default()))])
        .build()
        .await
        .unwrap();
        assert_eq!(
            built
                .engine
                .execute_turn("/help", "help")
                .await
                .unwrap()
                .turns,
            0
        );
        assert!(capture.requests.lock().unwrap().is_empty());
        built
            .engine
            .execute_turn(&format!("/skill:{id} chosen"), "skill")
            .await
            .unwrap();
        assert!(
            capture.requests.lock().unwrap()[0]
                .1
                .contains("EXACT_BODY chosen")
        );
    }
    let mut config = skill_config();
    config.tools.builtin_allowlist = vec!["Read".into()];
    let result = AgentBootstrap::new(config, workspace.path().to_str().unwrap(), null_output())
        .provider(Arc::new(Capture::default()))
        .host_skills(vec![hosted("pkg.guide", "", Arc::new(Access::default()))])
        .build()
        .await;
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("Session tool policy")
    );
}

#[tokio::test]
async fn cancelled_source_check_and_shell_arguments_never_reach_model() {
    let workspace = tempfile::tempdir().unwrap();
    let capture = Arc::new(Capture::default());
    let access = Arc::new(Access::default());
    let mut built = AgentBootstrap::new(
        skill_config(),
        workspace.path().to_str().unwrap(),
        null_output(),
    )
    .provider(capture.clone())
    .host_skills(vec![hosted("pkg.guide", "", access.clone())])
    .build()
    .await
    .unwrap();
    access.pending.store(true, Ordering::SeqCst);
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            built
                .engine
                .execute_turn("/skill:pkg.guide interrupted", "turn-1")
        )
        .await
        .is_err()
    );
    assert!(capture.requests.lock().unwrap().is_empty());
    access.pending.store(false, Ordering::SeqCst);
    let error = built
        .engine
        .execute_turn("/skill:pkg.guide !`echo forbidden`", "turn-2")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("shell execution"));
    assert!(capture.requests.lock().unwrap().is_empty());
    built
        .engine
        .execute_turn("/skill:pkg.guide continued", "turn-3")
        .await
        .unwrap();
    let requests = capture.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].1.contains("interrupted"));
    assert!(!requests[0].1.contains("forbidden"));
    assert!(requests[0].1.contains("EXACT_BODY continued"));
}
