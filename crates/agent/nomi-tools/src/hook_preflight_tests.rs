//! Native preflight admission must not perform the target operation.
use std::sync::{Arc, RwLock};

use nomi_config::file_cache::FileCacheConfig;
use nomi_process_runtime::{CapabilityPolicy, ProcessSupervisor, SandboxPolicy, SupervisorConfig};
use serde_json::json;

use crate::{
    Tool, ToolExecutionContext, apply_patch::ApplyPatchTool, bash::BashTool, edit::EditTool,
    exec_command::ExecCommandTool, file_cache::{FileStateCache, file_mtime_ms}, glob::GlobTool,
    grep::GrepTool, process_store::ProcessStore, read::ReadTool, update_plan::UpdatePlanTool,
    write::WriteTool, write_stdin::WriteStdinTool,
};

fn context() -> ToolExecutionContext {
    ToolExecutionContext::from_scoped_tool_call("native-preflight-tests", "call")
}

fn cache() -> Arc<RwLock<FileStateCache>> {
    Arc::new(RwLock::new(FileStateCache::new(&FileCacheConfig {
        max_entries: 10,
        max_size_bytes: 1024 * 1024,
        enabled: true,
    })))
}

#[tokio::test]
async fn hook_preflight_write_checks_root_and_does_not_create_directories() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let tool = WriteTool::new(None)
        .with_cwd(Some(root.path().to_path_buf()))
        .with_write_root(Some(root.path().to_path_buf()));
    tool.preflight_hook(&json!({"file_path":"new/文件.txt", "content":"secret"}), &context())
        .await.unwrap();
    assert!(!root.path().join("new").exists());
    let input = json!({"file_path":outside.path().join("escape.txt"), "content":"secret"});
    let rejected = tool.preflight_hook(&input, &context()).await.unwrap_err();
    assert!(rejected.contains("outside the allowed write root"));
    assert_eq!(tool.execute(input).await.content, rejected);
}

#[tokio::test]
async fn hook_preflight_cache_guards_reject_unread_and_stale_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.txt");
    std::fs::write(&path, "original").unwrap();
    let cache = cache();
    let write = WriteTool::new(Some(cache.clone())).with_cwd(Some(root.path().to_path_buf()));
    let edit = EditTool::new(Some(cache.clone())).with_cwd(Some(root.path().to_path_buf()));
    let patch = ApplyPatchTool::new(Some(cache.clone())).with_cwd(Some(root.path().to_path_buf()));
    let write_args = json!({"file_path":"notes.txt", "content":"replacement"});
    let edit_args = json!({"file_path":"notes.txt", "old_string":"original", "new_string":"replacement"});
    let patch_args = json!({"files":[{"file_path":"notes.txt", "delete":true}]});
    for (tool, input) in [(&write as &dyn Tool, &write_args), (&edit, &edit_args), (&patch, &patch_args)] {
        assert!(tool.preflight_hook(input, &context()).await.unwrap_err().contains("must Read"));
    }
    let reader = ReadTool::new(Some(cache.clone()), Some(root.path().to_path_buf()));
    assert!(!reader.execute(json!({"file_path":"notes.txt"})).await.is_error);
    for (tool, input) in [(&write as &dyn Tool, &write_args), (&edit, &edit_args), (&patch, &patch_args)] {
        tool.preflight_hook(input, &context()).await.unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    let mut stale = cache.read().unwrap().peek(&path).unwrap().clone();
    stale.mtime_ms = file_mtime_ms(&path).unwrap().wrapping_add(1);
    cache.write().unwrap().insert(path.clone(), stale);
    assert!(edit.preflight_hook(&edit_args, &context()).await.unwrap_err().contains("modified externally"));
    assert!(patch.preflight_hook(&patch_args, &context()).await.unwrap_err().contains("changed on disk"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
}

#[tokio::test]
async fn hook_preflight_final_dispatch_rechecks_changed_authority() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("notes.txt");
    std::fs::write(&path, "original").unwrap();
    let cache = cache();
    let reader = ReadTool::new(Some(cache.clone()), Some(root.path().to_path_buf()));
    reader.execute(json!({"file_path":"notes.txt"})).await;
    let tool = WriteTool::new(Some(cache.clone())).with_cwd(Some(root.path().to_path_buf()));
    let input = json!({"file_path":"notes.txt", "content":"replacement"});
    tool.preflight_hook(&input, &context()).await.unwrap();
    cache.write().unwrap().clear();
    assert!(tool.execute(input).await.is_error);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
}

#[tokio::test]
async fn hook_preflight_patch_validates_whole_batch_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let tool = ApplyPatchTool::new(None)
        .with_cwd(Some(root.path().to_path_buf()))
        .with_write_root(Some(root.path().to_path_buf()));
    let input = json!({"files":[
        {"file_path":"new/first.txt", "content":"first"},
        {"file_path":outside.path().join("second.txt"), "content":"second"}
    ]});
    assert!(tool.preflight_hook(&input, &context()).await.is_err());
    assert!(!root.path().join("new").exists());
    assert!(!outside.path().join("second.txt").exists());
    let invalid = json!({"files":[{"file_path":"new.txt", "delete":true, "content":"bad"}]});
    let error = tool.preflight_hook(&invalid, &context()).await.unwrap_err();
    assert_eq!(tool.execute(invalid).await.content, error);
}

#[tokio::test]
async fn hook_preflight_reads_searches_and_plan_only_validate_inputs() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache();
    let reader = ReadTool::new(Some(cache.clone()), Some(root.path().to_path_buf()));
    // A nonexistent body can be admitted without attempting to read it.
    reader.preflight_hook(&json!({"file_path":"missing.txt"}), &context()).await.unwrap();
    assert!(cache.read().unwrap().is_empty());
    assert!(reader.preflight_hook(&json!({"file_path":"a", "file_paths":["b"]}), &context()).await.is_err());
    let glob = GlobTool::new(root.path().to_path_buf());
    glob.preflight_hook(&json!({"pattern":"**/*.rs"}), &context()).await.unwrap();
    assert!(glob.preflight_hook(&json!({"pattern":"["}), &context()).await.is_err());
    let grep = GrepTool::new(root.path().to_path_buf());
    grep.preflight_hook(&json!({"pattern":"needle"}), &context()).await.unwrap();
    assert!(grep.preflight_hook(&json!({"pattern":"needle", "context_lines":-1}), &context()).await.is_err());
    let plan = UpdatePlanTool::new();
    plan.preflight_hook(&json!({"plan":[{"step":"verify", "status":"pending"}]}), &context()).await.unwrap();
    assert!(plan.preflight_hook(&json!({"plan":[]}), &context()).await.is_err());
}

#[tokio::test]
async fn hook_preflight_commands_reuse_cwd_admission_without_starting_processes() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let store = Arc::new(ProcessStore::new());
    let capability = CapabilityPolicy::local_owner(root.path().to_path_buf());
    let bash = BashTool::new(supervisor.clone(), root.path().to_path_buf(), capability.clone());
    let exec = ExecCommandTool::new(supervisor.clone(), store.clone(), root.path().to_path_buf(), capability);
    bash.preflight_hook(&json!({"command":"echo secret > marker"}), &context()).await.unwrap();
    exec.preflight_hook(&json!({"cmd":"echo secret > marker"}), &context()).await.unwrap();
    exec.preflight_hook(&json!({"script":"echo secret > marker", "language":"shell", "timeout":1000}), &context()).await.unwrap();
    assert!(!root.path().join("marker").exists());
    assert!(store.is_empty());
    assert!(bash.preflight_hook(&json!({"command":"echo secret", "workdir":outside.path()}), &context()).await.is_err());
    assert!(exec.preflight_hook(&json!({"cmd":"echo secret", "workdir":root.path().join("missing")}), &context()).await.is_err());
    let denied = BashTool::new(supervisor.clone(), root.path().to_path_buf(), CapabilityPolicy {
        cwd_roots: vec![root.path().to_path_buf()], sandbox: SandboxPolicy::DenySpawn,
    });
    assert!(denied.preflight_hook(&json!({"command":"echo secret"}), &context()).await.unwrap_err().contains("sandbox policy"));
    assert!(exec.preflight_hook(&json!({"script":"print('secret')", "language":"python", "timeout":1000}), &context()).await.is_err());
    assert!(supervisor.shutdown().await.sessions.is_empty());
}

#[tokio::test]
async fn hook_preflight_stdin_rejects_unknown_session_without_installing_binding() {
    let supervisor = ProcessSupervisor::new(SupervisorConfig::default());
    let store = Arc::new(ProcessStore::new());
    let tool = WriteStdinTool::new(supervisor.clone(), store.clone());
    assert!(tool.preflight_hook(&json!({"session_id":1,"chars":"secret"}), &context()).await.unwrap_err().contains("unknown or finished"));
    assert!(store.is_empty());
    assert!(supervisor.shutdown().await.sessions.is_empty());
}
