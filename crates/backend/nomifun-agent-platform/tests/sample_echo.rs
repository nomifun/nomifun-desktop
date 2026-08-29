use std::path::PathBuf;

use nomifun_agent_platform::{SampleEchoGateConfig, run_sample_echo_gate};

#[tokio::test]
async fn compiled_sample_echo_runs_the_final_stack_and_fault_gate() {
    let directory = tempfile::tempdir().expect("temporary C6 root");
    let working_root = directory.path().join("sample-echo");
    std::fs::create_dir_all(&working_root).expect("sample.echo fixture parent");
    let report = run_sample_echo_gate(SampleEchoGateConfig {
        working_root,
        node_executable: find_node().expect("Node.js is required for the recorded stdio fixture"),
    })
    .await
    .expect("sample.echo final-stack gate");

    assert!(report.package_materialized);
    assert!(report.capability_materialized);
    assert!(report.skill_materialized);
    assert!(report.mcp_materialized);
    assert!(report.config_validated);

    assert_eq!(report.clean_revision_action, "reuse_current_revision");
    assert_eq!(
        report.dirty_revision_action,
        "save_ordinary_visible_revision"
    );
    assert_eq!(report.clean_revision, 1);
    assert_eq!(report.dirty_revision, 2);
    assert!(report.clean_session.persistent_session);
    assert!(report.dirty_session.persistent_session);
    assert!(report.clean_session.tombstone_committed);
    assert!(report.dirty_session.tombstone_committed);
    assert_eq!(report.clean_session.effect_success_count, 1);
    assert_eq!(report.dirty_session.effect_success_count, 2);
    assert_eq!(report.clean_session.dispose_rpc, "acked");
    assert_eq!(report.dirty_session.dispose_rpc, "timed_out");

    assert_eq!(report.broker_recorded_transport_calls, 2);
    assert_eq!(report.first_echo, "echo:hello");
    assert_eq!(report.restart_echo, "echo:after-restart");
    assert!(report.plugin_state_cas_conflict);
    assert!(report.plugin_state_survived_restart);
    assert!(report.plugin_state_survived_session_delete);

    assert!(!report.faults.save_failure_created_revision);
    assert!(!report.faults.save_failure_created_session);
    assert!(!report.faults.materialization_failure_published_generation);
    assert!(report.faults.panic_effect_became_uncertain);
    assert!(!report.faults.panic_retried_effect);
    assert!(report.faults.dispose_timeout_forced_tree_cleanup);
}

#[test]
fn sample_echo_has_no_hidden_revision_session_or_effect_shortcut() {
    let source = include_str!("../src/sample_echo.rs");
    for forbidden in [
        "TestPreset",
        "TestSession",
        "hidden_revision",
        "ephemeral_session",
        "MockEffect",
        "mock_effect",
        "DraftSnapshot",
    ] {
        assert!(
            !source.contains(forbidden),
            "sample.echo reintroduced forbidden surface {forbidden}"
        );
    }
}

fn find_node() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("NODE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in node_names() {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate.canonicalize().ok().or(Some(candidate));
            }
        }
    }
    None
}

fn node_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["node.exe", "node.cmd"]
    } else {
        &["node"]
    }
}
