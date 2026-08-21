//! A model reporting spec-driven work as delivered must have looked at the spec
//! since it started editing. The observed failure read README/QA_TASK in the
//! first few messages, then wrote code for 57 more without re-reading either,
//! invented its own CLI interface, wrote tests around the inventions, and
//! truthfully reported that those tests passed — `bun test` really did exit 0,
//! while an independent contract verifier scored 0/10.

use super::{
    CompletionAdjudication, CompletionEvidenceContext, CompletionEvidenceMode,
    SPEC_RECHECK_NUDGE, apply_terminal_effect_evidence, completion_adjudication,
    durable_targets_for_tool,
    metadata_refers_to_open_file, unbacked_completion_claim,
};
use nomi_types::message::{ContentBlock, Message, Role};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nomi_config::config::{CliArgs, Config};
use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_types::llm::{LlmEvent, LlmRequest};

use crate::output::null_sink::NullSink;
use crate::output::OutputSink;
use crate::session::{AcceptedTurnRoot, EditableTurnCheckpoint, SessionManager};

fn read(path: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: "r".to_string(),
        name: "Read".to_string(),
        input: serde_json::json!({ "file_path": path }),
        extra: None,
    }
}

fn write(path: &str) -> ContentBlock {
    ContentBlock::ToolUse {
        id: "w".to_string(),
        name: "Write".to_string(),
        input: serde_json::json!({ "file_path": path, "content": "x" }),
        extra: None,
    }
}

fn round(block: ContentBlock) -> Message {
    Message::new(Role::Assistant, vec![block])
}

/// The exact reported shape: spec read early, then implementation only.
fn the_bad_case() -> Vec<Message> {
    vec![
        round(read("workspace/README.md")),
        round(read("workspace/QA_TASK.md")),
        round(write("workspace/src/csv.ts")),
        round(write("workspace/tests/cli.test.ts")),
    ]
}

/// The observed final summary, verbatim in spirit: truthful about its own tests,
/// wrong about the contract.
const FALSE_GREEN: &str = "交付总结：bun test 20 pass / 0 fail，现有测试全部保留且通过，\
项目当前处于可交付状态。";

#[test]
fn the_reported_false_green_delivery_is_gated() {
    assert_eq!(
        unbacked_completion_claim(FALSE_GREEN, &the_bad_case()),
        Some(SPEC_RECHECK_NUDGE)
    );
}

#[test]
fn re_reading_the_spec_after_editing_satisfies_the_gate() {
    // The remedy the nudge asks for. Doing it must end the turn normally,
    // otherwise the gate would loop forever.
    let mut messages = the_bad_case();
    messages.push(round(read("workspace/README.md")));
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn an_english_completion_claim_is_gated_too() {
    for claim in [
        "All tests pass. The project is deliverable.",
        "20 pass / 0 fail and the work is ready to ship.",
    ] {
        assert!(
            unbacked_completion_claim(claim, &the_bad_case()).is_some(),
            "must gate: {claim}"
        );
    }
}

#[test]
fn an_ordinary_answer_is_never_gated() {
    for answer in [
        "Here is a summary of what the CSV parser does.",
        "The contract requires --db; I have not implemented it yet.",
        "I could not get the tests to run; the runner is missing.",
        "",
    ] {
        assert_eq!(
            unbacked_completion_claim(answer, &the_bad_case()),
            None,
            "must not gate: {answer}"
        );
    }
}

#[test]
fn a_turn_with_no_spec_to_check_is_never_gated() {
    // Ad-hoc work with no contract has nothing to re-read, so the gate must not
    // fire and demand a file that does not exist.
    let messages = vec![round(write("src/main.rs"))];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn a_read_only_turn_is_never_gated() {
    // Nothing was changed, so there is no implementation to have drifted.
    let messages = vec![round(read("README.md")), round(read("src/cli.ts"))];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn a_spec_read_only_after_the_last_write_still_counts() {
    // Order is what matters, not how many times the spec was read.
    let messages = vec![
        round(write("src/cli.ts")),
        round(read("docs/REQUIREMENTS.md")),
    ];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn non_spec_markdown_does_not_count_as_a_contract() {
    // Editing a changelog is not consulting a contract.
    let messages = vec![
        round(read("CHANGELOG.md")),
        round(write("src/cli.ts")),
    ];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn a_batch_read_of_the_spec_is_recognized() {
    // Read accepts file_paths for several files at once.
    let messages = vec![
        round(write("src/cli.ts")),
        round(ContentBlock::ToolUse {
            id: "r".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({ "file_paths": ["src/store.ts", "README.md"] }),
            extra: None,
        }),
    ];
    assert_eq!(unbacked_completion_claim(FALSE_GREEN, &messages), None);
}

#[test]
fn the_nudge_names_the_concrete_remedy() {
    // A gate that only objects would just make the model rephrase the claim.
    assert!(SPEC_RECHECK_NUDGE.contains("Re-read the spec"), "{SPEC_RECHECK_NUDGE}");
    assert!(
        SPEC_RECHECK_NUDGE.contains("each required behavior"),
        "{SPEC_RECHECK_NUDGE}"
    );
}

fn requirement(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_owned(),
    }]
}

struct FalseCompletionProvider;

#[async_trait::async_trait]
impl LlmProvider for FalseCompletionProvider {
    async fn stream(
        &self,
        _request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(LlmEvent::TextDelta("Created miniapp.html.".to_owned()))
            .unwrap();
        tx.try_send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::EndTurn,
            usage: Default::default(),
        })
        .unwrap();
        Ok(rx)
    }
}

struct RaceTailFalseCompletionProvider {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl LlmProvider for RaceTailFalseCompletionProvider {
    async fn stream(
        &self,
        _request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let text = if call == 0 {
            "The current status is available."
        } else {
            "Created miniapp.html."
        };
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(LlmEvent::TextDelta(text.to_owned())).unwrap();
        tx.try_send(LlmEvent::Done {
            stop_reason: nomi_types::message::StopReason::EndTurn,
            usage: Default::default(),
        })
        .unwrap();
        Ok(rx)
    }
}

#[derive(Default)]
struct DiscardRecordingSink {
    starts: AtomicUsize,
    discarded_attempts: Mutex<Vec<u32>>,
}

impl OutputSink for DiscardRecordingSink {
    fn emit_text_delta(&self, _text: &str, _msg_id: &str) {}
    fn emit_thinking(&self, _text: &str, _msg_id: &str) {}
    fn emit_tool_call(&self, _tool_use_id: &str, _name: &str, _input: &str) {}
    fn emit_tool_result(
        &self,
        _tool_use_id: &str,
        _name: &str,
        _is_error: bool,
        _content: &str,
    ) {
    }
    fn emit_stream_start(&self, _msg_id: &str) {
        self.starts.fetch_add(1, Ordering::SeqCst);
    }
    fn emit_output_discarded(&self, _msg_id: &str, restart_attempt: u32) {
        self.discarded_attempts
            .lock()
            .unwrap()
            .push(restart_attempt);
    }
    fn emit_stream_end(
        &self,
        _msg_id: &str,
        _turns: usize,
        _input_tokens: u64,
        _output_tokens: u64,
        _cache_creation_tokens: u64,
        _cache_read_tokens: u64,
    ) {
    }
    fn emit_error(&self, _msg: &str) {}
    fn emit_info(&self, _msg: &str) {}
}

struct FalseCompletionThenBreakSessionProvider {
    session_directory: std::path::PathBuf,
    broken: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl LlmProvider for FalseCompletionThenBreakSessionProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        if !self.broken.swap(true, std::sync::atomic::Ordering::SeqCst) {
            std::fs::remove_dir_all(&self.session_directory).unwrap();
            std::fs::write(&self.session_directory, "blocks rollback persistence").unwrap();
        }
        FalseCompletionProvider.stream(request).await
    }
}

fn persistent_test_config(workspace: &Path, session_directory: &Path) -> Config {
    let mut config = Config::resolve(&CliArgs {
        provider: Some("openai".to_owned()),
        api_key: Some("sk-test".to_owned()),
        base_url: None,
        model: Some("test-model".to_owned()),
        max_tokens: None,
        max_turns: Some(1),
        system_prompt: None,
        profile: None,
        auto_approve: true,
        project_dir: Some(workspace.to_path_buf()),
    })
    .unwrap();
    config.session.enabled = true;
    config.session.directory = session_directory.to_string_lossy().into_owned();
    config.compact.enabled = false;
    config
}

fn seeded_persistent_engine(
    workspace: &Path,
    session_directory: &Path,
    session_id: &str,
) -> (super::AgentEngine, Vec<Message>, EditableTurnCheckpoint) {
    seeded_persistent_engine_with_provider(
        workspace,
        session_directory,
        session_id,
        Arc::new(FalseCompletionProvider),
    )
}

fn seeded_persistent_engine_with_provider(
    workspace: &Path,
    session_directory: &Path,
    session_id: &str,
    provider: Arc<dyn LlmProvider>,
) -> (super::AgentEngine, Vec<Message>, EditableTurnCheckpoint) {
    let config = persistent_test_config(workspace, session_directory);
    let mut engine = super::AgentEngine::new_with_provider(
        provider,
        config,
        ToolRegistry::new(),
        Arc::new(NullSink),
        workspace.to_path_buf(),
    );
    engine
        .init_session("openai", &workspace.to_string_lossy(), Some(session_id))
        .unwrap();
    let prior_messages = vec![
        Message::now(
            Role::User,
            vec![ContentBlock::Text {
                text: "Earlier trusted question".to_owned(),
            }],
        ),
        Message::now(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "Earlier trusted answer".to_owned(),
            }],
        ),
    ];
    let mut prior_host_context = std::collections::BTreeMap::new();
    prior_host_context.insert("route".to_owned(), "trusted".to_owned());
    let prior_checkpoint = EditableTurnCheckpoint {
        source_message_id: "prior-source".to_owned(),
        start_len: 0,
        prior_host_context: prior_host_context.clone(),
    };
    engine.messages = prior_messages.clone();
    engine.editable_turn = Some(prior_checkpoint.clone());
    engine.host_context = prior_host_context;
    engine.try_save_session().unwrap();
    (engine, prior_messages, prior_checkpoint)
}

fn adjudicated(
    root: &Path,
    requirement_text: &str,
    answer: &str,
    supported: &[&str],
) -> bool {
    completion_adjudication(
        answer,
        &requirement(requirement_text),
        &supported.iter().map(|value| value.to_string()).collect::<Vec<_>>(),
        root,
        CompletionEvidenceMode::LocalFingerprint,
    )
    .is_some()
}

#[test]
fn explicit_current_file_commands_and_claims_enter_the_narrow_gate() {
    let root = tempfile::tempdir().unwrap();
    for (request, answer) in [
        ("Create miniapp.html.", "Created miniapp.html."),
        ("Can you create miniapp.html?", "I created miniapp.html."),
        ("可以帮我创建miniapp.html吗？", "已创建miniapp.html。"),
        ("Please fix `src/lib.rs`.", "Fixed `src/lib.rs`."),
        ("Create [my app.html](https://example.invalid).", "Created [my app.html](https://example.invalid)."),
        ("Create .env.", "Created .env."),
        ("Create Dockerfile.", "Created Dockerfile."),
        ("先创建a.html。", "已创建a.html。"),
        ("Create miniapp.html.", "Created \"miniapp.html\"."),
        ("Create miniapp.html.", "miniapp.html is ready."),
        ("Create miniapp.html.", "miniapp.html is now ready."),
        (
            "Create miniapp.html.",
            "miniapp.html now contains the implementation.",
        ),
        ("Create miniapp.html.", "Task complete — see miniapp.html."),
        (
            "Create miniapp.html.",
            "I created miniapp.html, but couldn't run the tests.",
        ),
        (
            "Create miniapp.html.",
            "Created miniapp.html, but deleted temp.txt.",
        ),
        (
            "Create miniapp.html.",
            "已创建miniapp.html，但未生成预览图。",
        ),
        (
            "Create miniapp.html.",
            "Created miniapp.html — would you like anything else?",
        ),
        (
            "Create miniapp.html.",
            "Created miniapp.html successfully.",
        ),
        (
            "Create miniapp.html.",
            "I created miniapp.html as requested.",
        ),
        (
            "Create miniapp.html.",
            "miniapp.html has been created successfully.",
        ),
        (
            "Create miniapp.html.",
            "已创建miniapp.html，已按要求完成。",
        ),
    ] {
        assert!(
            adjudicated(root.path(), request, answer, &[]),
            "expected hard evidence gate: request={request:?}, answer={answer:?}"
        );
    }
}

#[test]
fn ascii_case_folding_preserves_unicode_span_boundaries() {
    let root = tempfile::tempdir().unwrap();
    assert!(!adjudicated(
        root.path(),
        "Create İİİİİ miniapp.html.",
        "Created İİİİİ miniapp.html.",
        &[],
    ));
}

#[test]
fn exact_generic_completion_claims_bind_every_required_target() {
    let root = tempfile::tempdir().unwrap();
    let request = "Create a.html. Create b.html.";
    assert!(adjudicated(root.path(), request, "Done.", &[]));
    assert!(
        adjudicated(root.path(), request, "Completed.", &["a.html"]),
        "one durable target cannot back a whole-task completion claim"
    );
    assert!(!adjudicated(
        root.path(),
        request,
        "已完成。",
        &["a.html", "b.html"],
    ));
}

#[test]
fn read_only_historical_quoted_and_hypothetical_prose_never_enters_the_gate() {
    let root = tempfile::tempdir().unwrap();
    for (request, answer) in [
        ("Explain how to create miniapp.html; do not modify files.", "Created miniapp.html."),
        ("Review miniapp.html and report whether it is fixed.", "miniapp.html is not fixed."),
        ("Show code that would create miniapp.html.", "This would create miniapp.html."),
        ("Generate a report about src/lib.rs.", "Generated a report about src/lib.rs."),
        ("生成一份关于src/lib.rs的分析。", "已生成一份关于src/lib.rs的分析。"),
        ("请确认是否创建miniapp.html。", "已创建miniapp.html。"),
        ("请比较创建miniapp.html与修改src/lib.rs的区别。", "已创建miniapp.html。"),
        ("请不要创建miniapp.html。", "已创建miniapp.html。"),
        ("麻烦你不要修改src/lib.rs。", "已修改src/lib.rs。"),
        ("The prior task said:\n> Create miniapp.html\nWhy?", "Created miniapp.html."),
        ("Update Node to v1.2.3.", "Updated v1.2.3."),
        ("Create a link to https://example.com in README.md.", "Created example.com."),
        ("Explain 3.14.", "Updated 3.14."),
    ] {
        assert!(
            !adjudicated(root.path(), request, answer, &[]),
            "ordinary prose must not hard fail: request={request:?}, answer={answer:?}"
        );
    }
}

#[test]
fn negative_modal_historical_and_unsaved_answers_are_not_positive_claims() {
    let root = tempfile::tempdir().unwrap();
    let request = "Create miniapp.html.";
    for answer in [
        "I did not create miniapp.html.",
        "miniapp.html is still missing.",
        "I could not create miniapp.html: permission denied.",
        "I will create miniapp.html next.",
        "The log says \"Created miniapp.html\", but the file is missing.",
        "Previously, I created miniapp.html.",
        "I created miniapp.html in a previous run.",
        "'Created miniapp.html'",
        "`Created miniapp.html`",
        "Created miniapp.html was the model's final answer.",
        "Created miniapp.html draft, but did not save it.",
        "已创建miniapp.html草稿，但未保存、未落盘。",
        "上一轮已创建miniapp.html。",
    ] {
        assert!(
            !adjudicated(root.path(), request, answer, &[]),
            "honest/history/modal text must not be a current claim: {answer:?}"
        );
    }
}

#[test]
fn action_target_binding_does_not_assign_source_paths_to_mutation() {
    let root = tempfile::tempdir().unwrap();
    let request = "Read docs/spec.md and then create miniapp.html.";
    assert!(adjudicated(
        root.path(),
        request,
        "Created miniapp.html.",
        &[]
    ));
    assert!(!adjudicated(
        root.path(),
        request,
        "Created miniapp.html.",
        &["miniapp.html"]
    ));
    assert!(adjudicated(
        root.path(),
        "Fix src/lib.rs using docs/spec.md.",
        "Fixed src/lib.rs.",
        &["tests/lib.rs"]
    ));
}

#[test]
fn later_directives_cancel_and_can_readd_the_exact_target() {
    let root = tempfile::tempdir().unwrap();
    assert!(!adjudicated(
        root.path(),
        "Create miniapp.html.\nPlease do not create miniapp.html.",
        "Created miniapp.html.",
        &[]
    ));
    assert!(adjudicated(
        root.path(),
        "Create miniapp.html.\nPlease do not create miniapp.html.\nPlease create miniapp.html.",
        "Created miniapp.html.",
        &[]
    ));
}

#[test]
fn atomic_tool_receipts_preserve_exact_paths_and_filter_deletes() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("src").join("lib.rs");
    let write = durable_targets_for_tool(
        "Write",
        &serde_json::json!({ "file_path": nested }),
        root.path(),
        CompletionEvidenceMode::LocalFingerprint,
    );
    assert_eq!(write, vec!["src/lib.rs"]);

    let patch = durable_targets_for_tool(
        "ApplyPatch",
        &serde_json::json!({
            "files": [
                { "file_path": "src/lib.rs", "edits": [] },
                { "file_path": "tests/lib.rs", "delete": true }
            ]
        }),
        root.path(),
        CompletionEvidenceMode::LocalFingerprint,
    );
    assert_eq!(patch, vec!["src/lib.rs"]);
}

#[test]
fn fingerprint_identity_binds_the_open_handle_to_the_terminal_path() {
    let root = tempfile::tempdir().unwrap();
    let expected_path = root.path().join("expected.txt");
    let replacement_path = root.path().join("replacement.txt");
    std::fs::write(&expected_path, "same-size").unwrap();
    std::fs::write(&replacement_path, "different").unwrap();
    let open = std::fs::File::open(&expected_path).unwrap();

    assert!(metadata_refers_to_open_file(&open, &expected_path));
    assert!(!metadata_refers_to_open_file(&open, &replacement_path));
}

#[tokio::test]
async fn final_fingerprint_requires_both_a_real_delta_and_this_turns_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut context = CompletionEvidenceContext::new(requirement("Create miniapp.html."));
    context
        .ensure_target_baselines(root.path(), CompletionEvidenceMode::LocalFingerprint)
        .await;
    std::fs::write(root.path().join("miniapp.html"), "hello").unwrap();

    let without_mutation = context
        .supported_targets(
            "Created miniapp.html.",
            root.path(),
            CompletionEvidenceMode::LocalFingerprint,
            &[],
        )
        .await;
    assert!(without_mutation.is_empty());

    context.successful_mutation_observed = true;
    let with_mutation = context
        .supported_targets(
            "Created miniapp.html.",
            root.path(),
            CompletionEvidenceMode::LocalFingerprint,
            &[],
        )
        .await;
    assert_eq!(with_mutation, vec!["miniapp.html"]);

    std::fs::remove_file(root.path().join("miniapp.html")).unwrap();
    let deleted_again = context
        .supported_targets(
            "Created miniapp.html.",
            root.path(),
            CompletionEvidenceMode::LocalFingerprint,
            &["miniapp.html".to_owned()],
        )
        .await;
    assert!(deleted_again.is_empty(), "a stale receipt cannot prove final presence");
}

#[tokio::test]
async fn resource_limited_files_need_a_known_absent_baseline_for_exact_creation() {
    let preexisting_root = tempfile::tempdir().unwrap();
    let preexisting_path = preexisting_root.path().join("large.html");
    let preexisting = std::fs::File::create(&preexisting_path).unwrap();
    preexisting
        .set_len(super::MAX_COMPLETION_EVIDENCE_FILE_BYTES + 1)
        .unwrap();
    drop(preexisting);
    let mut preexisting_context =
        CompletionEvidenceContext::new(requirement("Create large.html."));
    preexisting_context
        .ensure_target_baselines(
            preexisting_root.path(),
            CompletionEvidenceMode::LocalFingerprint,
        )
        .await;
    let unsupported = preexisting_context
        .supported_targets(
            "Created large.html.",
            preexisting_root.path(),
            CompletionEvidenceMode::LocalFingerprint,
            &["large.html".to_owned()],
        )
        .await;
    assert!(
        unsupported.is_empty(),
        "an exact receipt cannot prove that a preexisting unhashable file changed"
    );

    let absent_root = tempfile::tempdir().unwrap();
    let mut absent_context = CompletionEvidenceContext::new(requirement("Create large.html."));
    absent_context
        .ensure_target_baselines(
            absent_root.path(),
            CompletionEvidenceMode::LocalFingerprint,
        )
        .await;
    let created = std::fs::File::create(absent_root.path().join("large.html")).unwrap();
    created
        .set_len(super::MAX_COMPLETION_EVIDENCE_FILE_BYTES + 1)
        .unwrap();
    drop(created);
    let supported = absent_context
        .supported_targets(
            "Created large.html.",
            absent_root.path(),
            CompletionEvidenceMode::LocalFingerprint,
            &["large.html".to_owned()],
        )
        .await;
    assert_eq!(supported, ["large.html"]);
}

#[tokio::test]
async fn missing_pre_effect_baseline_never_turns_path_presence_into_change_proof() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("miniapp.html"), "original").unwrap();
    let mut context = CompletionEvidenceContext::new(requirement("Create miniapp.html."));
    context
        .unfingerprinted_targets
        .insert("miniapp.html".to_owned());
    let supported = context
        .supported_targets(
            "Created miniapp.html.",
            root.path(),
            CompletionEvidenceMode::LocalFingerprint,
            &["miniapp.html".to_owned()],
        )
        .await;
    assert!(
        supported.is_empty(),
        "a late/preflight-failed target can have been written and restored"
    );
}

#[test]
fn failed_or_opaque_mutation_invalidates_every_older_terminal_receipt() {
    let mut context = CompletionEvidenceContext::default();
    let mut ledger = crate::round::RoundLedger::default();
    apply_terminal_effect_evidence(
        &mut context,
        &mut ledger,
        true,
        false,
        vec!["miniapp.html".to_owned()],
        Vec::new(),
    );
    assert_eq!(context.terminal_exact_receipts, ["miniapp.html"]);

    apply_terminal_effect_evidence(
        &mut context,
        &mut ledger,
        true,
        true,
        Vec::new(),
        Vec::new(),
    );
    assert!(context.terminal_exact_receipts.is_empty());
    assert!(context.prior_durable_effect_targets.is_empty());
    assert!(ledger.durable_effect_targets.is_empty());
}

#[cfg(windows)]
#[test]
fn windows_absolute_targets_accept_canonical_root_casing_without_collapsing_suffix() {
    let root = tempfile::tempdir().unwrap();
    let absolute = root.path().join("MissingDir").join("Foo.rs");
    let mut differently_cased = absolute.to_string_lossy().to_string();
    differently_cased.replace_range(0..1, &differently_cased[..1].to_ascii_lowercase());

    let targets = durable_targets_for_tool(
        "Write",
        &serde_json::json!({ "file_path": differently_cased }),
        root.path(),
        CompletionEvidenceMode::LocalFingerprint,
    );
    assert_eq!(targets, ["MissingDir/Foo.rs"]);
}

#[tokio::test]
async fn remote_mode_uses_case_sensitive_exact_atomic_receipts_only() {
    let root = tempfile::tempdir().unwrap();
    let mut context = CompletionEvidenceContext::new(requirement("Create /Home/Foo.rs."));
    context.successful_mutation_observed = true;
    let wrong_case = context
        .supported_targets(
            "Created /Home/Foo.rs.",
            root.path(),
            CompletionEvidenceMode::RemoteExactReceipts,
            &["remote-posix:/home/foo.rs".to_owned()],
        )
        .await;
    assert!(wrong_case.is_empty());
    let exact = context
        .supported_targets(
            "Created /Home/Foo.rs.",
            root.path(),
            CompletionEvidenceMode::RemoteExactReceipts,
            &["remote-posix:/Home/Foo.rs".to_owned()],
        )
        .await;
    assert_eq!(exact, vec!["remote-posix:/Home/Foo.rs"]);
}

#[test]
fn typed_issue_exposes_the_exact_missing_target() {
    let root = tempfile::tempdir().unwrap();
    let issue = completion_adjudication(
        "Created src/lib.rs.",
        &requirement("Fix src/lib.rs."),
        &["tests/lib.rs".to_owned()],
        root.path(),
        CompletionEvidenceMode::LocalFingerprint,
    )
    .expect("same leaf in another directory is not evidence");
    assert!(matches!(
        issue,
        CompletionAdjudication::UnbackedStateChangeClaim { ref target }
            if target == "src/lib.rs"
    ));
}

#[tokio::test]
async fn typed_verdict_restores_and_persists_the_exact_seeded_turn_root() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let (mut engine, prior_messages, prior_checkpoint) =
        seeded_persistent_engine(workspace.path(), sessions.path(), "rollback-seeded");
    let mut context = CompletionEvidenceContext::new(requirement("Create miniapp.html."));

    let result = engine
        .execute_turn_with_completion_evidence_context(
            requirement("Create miniapp.html."),
            "attempt-msg",
            "new-source",
            None,
            Some(&mut context),
        )
        .await
        .unwrap();
    assert!(matches!(
        result.completion_adjudication,
        Some(CompletionAdjudication::UnbackedStateChangeClaim { .. })
    ));
    assert_eq!(
        serde_json::to_value(&engine.messages).unwrap(),
        serde_json::to_value(&prior_messages).unwrap()
    );
    assert_eq!(engine.editable_turn, Some(prior_checkpoint.clone()));
    assert_eq!(engine.host_context_value("route").as_deref(), Some("trusted"));

    let persisted = SessionManager::new(sessions.path().to_path_buf(), 20)
        .load("rollback-seeded")
        .unwrap();
    assert_eq!(
        serde_json::to_value(&persisted.messages).unwrap(),
        serde_json::to_value(&prior_messages).unwrap()
    );
    assert_eq!(persisted.editable_turn, Some(prior_checkpoint));
    assert_eq!(
        persisted.host_context.get("route").map(String::as_str),
        Some("trusted")
    );
}

#[tokio::test]
async fn race_tail_terminal_adjudication_discards_the_whole_accepted_turn() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let config = persistent_test_config(workspace.path(), sessions.path());
    let output = Arc::new(DiscardRecordingSink::default());
    let provider = Arc::new(RaceTailFalseCompletionProvider {
        calls: AtomicUsize::new(0),
    });
    let mut engine = super::AgentEngine::new_with_provider(
        provider,
        config,
        ToolRegistry::new(),
        output.clone(),
        workspace.path().to_path_buf(),
    );
    engine
        .init_session(
            "openai",
            &workspace.path().to_string_lossy(),
            Some("race-tail-full-discard"),
        )
        .unwrap();
    let mut context = CompletionEvidenceContext::new(requirement("Explain the current status."));

    let first = engine
        .execute_turn_with_completion_evidence_context(
            requirement("Explain the current status."),
            "attempt-a",
            "source-root",
            None,
            Some(&mut context),
        )
        .await
        .unwrap();
    assert!(first.completion_adjudication.is_none());

    let second_requirement = requirement("Create miniapp.html.");
    context.requirement.extend(second_requirement.clone());
    let second = engine
        .execute_turn_with_completion_evidence_context(
            second_requirement,
            "attempt-b",
            "source-root",
            None,
            Some(&mut context),
        )
        .await
        .unwrap();
    assert!(matches!(
        second.completion_adjudication,
        Some(CompletionAdjudication::UnbackedStateChangeClaim { .. })
    ));
    assert_eq!(output.starts.load(Ordering::SeqCst), 2);
    assert_eq!(
        output.discarded_attempts.lock().unwrap().as_slice(),
        [0],
        "a terminal A2 verdict restores the immutable accepted-turn stream checkpoint"
    );
    assert!(engine.messages.is_empty(), "both host passes rewind to the accepted root");
}

#[tokio::test]
async fn checked_root_persistence_failure_becomes_state_inconsistent() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let provider = Arc::new(FalseCompletionThenBreakSessionProvider {
        session_directory: sessions.path().to_path_buf(),
        broken: std::sync::atomic::AtomicBool::new(false),
    });
    let (mut engine, _, _) = seeded_persistent_engine_with_provider(
        workspace.path(),
        sessions.path(),
        "rollback-failure",
        provider,
    );
    let mut context = CompletionEvidenceContext::new(requirement("Create miniapp.html."));

    let result = engine
        .execute_turn_with_completion_evidence_context(
            requirement("Create miniapp.html."),
            "attempt-msg",
            "new-source",
            None,
            Some(&mut context),
        )
        .await
        .unwrap();
    assert!(matches!(
        result.completion_adjudication,
        Some(CompletionAdjudication::HistoryRollbackFailed { .. })
    ));
}

#[test]
fn explicit_context_clear_drops_every_recovery_root_before_reload() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let (mut engine, prior_messages, prior_checkpoint) =
        seeded_persistent_engine(workspace.path(), sessions.path(), "clear-roots");
    let root = AcceptedTurnRoot {
        source_message_id: "stale-source".to_owned(),
        messages: prior_messages,
        editable_turn: Some(prior_checkpoint),
        host_context: engine.host_context.clone(),
        activated_deferred_tools: Vec::new(),
    };
    let session = engine.current_session.as_mut().unwrap();
    session.accepted_turn_root = Some(root.clone());
    session.pending_host_terminal_root = Some(root);
    session.last_interrupted_turn_source = Some("stale-source".to_owned());
    engine.try_save_session().unwrap();

    engine.clear_context().unwrap();

    assert!(engine.messages.is_empty());
    let live = engine.current_session.as_ref().unwrap();
    assert!(live.accepted_turn_root.is_none());
    assert!(live.pending_host_terminal_root.is_none());
    assert!(live.last_interrupted_turn_source.is_none());
    let fresh = SessionManager::new(sessions.path().to_path_buf(), 20)
        .load("clear-roots")
        .unwrap();
    assert!(fresh.messages.is_empty());
    assert!(fresh.accepted_turn_root.is_none());
    assert!(fresh.pending_host_terminal_root.is_none());
    assert!(fresh.last_interrupted_turn_source.is_none());
}

#[test]
fn failed_context_clear_restores_live_and_persisted_history() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let (mut engine, prior_messages, prior_checkpoint) =
        seeded_persistent_engine(workspace.path(), sessions.path(), "clear-save-failure");
    engine
        .session_manager
        .as_ref()
        .unwrap()
        .fail_next_save_for_test();

    assert!(engine.clear_context().is_err());
    assert_eq!(
        serde_json::to_value(&engine.messages).unwrap(),
        serde_json::to_value(&prior_messages).unwrap()
    );
    assert_eq!(engine.editable_turn, Some(prior_checkpoint.clone()));
    let fresh = SessionManager::new(sessions.path().to_path_buf(), 20)
        .load("clear-save-failure")
        .unwrap();
    assert_eq!(
        serde_json::to_value(&fresh.messages).unwrap(),
        serde_json::to_value(&prior_messages).unwrap()
    );
    assert_eq!(fresh.editable_turn, Some(prior_checkpoint));
}

#[test]
fn failed_host_text_persistence_restores_live_and_persisted_history() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let (mut engine, prior_messages, prior_checkpoint) =
        seeded_persistent_engine(workspace.path(), sessions.path(), "host-save-failure");
    engine
        .session_manager
        .as_ref()
        .unwrap()
        .fail_next_save_for_test();

    assert!(
        engine
            .record_host_text_turn("generate a fox", "configure an image model", "host-source")
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(&engine.messages).unwrap(),
        serde_json::to_value(&prior_messages).unwrap()
    );
    assert_eq!(engine.editable_turn, Some(prior_checkpoint.clone()));
    let fresh = SessionManager::new(sessions.path().to_path_buf(), 20)
        .load("host-save-failure")
        .unwrap();
    assert_eq!(
        serde_json::to_value(&fresh.messages).unwrap(),
        serde_json::to_value(&prior_messages).unwrap()
    );
    assert_eq!(fresh.editable_turn, Some(prior_checkpoint));
}

#[test]
fn failed_turn_rewind_restores_live_and_persisted_history() {
    let workspace = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let (mut engine, prior_messages, prior_checkpoint) =
        seeded_persistent_engine(workspace.path(), sessions.path(), "rewind-save-failure");
    engine
        .session_manager
        .as_ref()
        .unwrap()
        .fail_next_save_for_test();

    assert!(engine.rewind_last_turn("prior-source").is_err());
    assert_eq!(
        serde_json::to_value(&engine.messages).unwrap(),
        serde_json::to_value(&prior_messages).unwrap()
    );
    assert_eq!(engine.editable_turn, Some(prior_checkpoint.clone()));
    let fresh = SessionManager::new(sessions.path().to_path_buf(), 20)
        .load("rewind-save-failure")
        .unwrap();
    assert_eq!(
        serde_json::to_value(&fresh.messages).unwrap(),
        serde_json::to_value(&prior_messages).unwrap()
    );
    assert_eq!(fresh.editable_turn, Some(prior_checkpoint));
}
