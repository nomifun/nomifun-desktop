// TC-AX-01: Multi-feature collaboration test (LOCAL, no LLM).
//
// Exercises memory + compression + file cache + tool description all at once.

use nomi_agent::compact::micro::{CLEARED_TOOL_RESULT, microcompact};
use nomi_agent::context::{SystemPromptCache, build_system_prompt};
use nomi_config::compact::CompactConfig;
use nomi_config::file_cache::FileCacheConfig;
use nomi_tools::Tool;
use nomi_tools::file_cache::FileStateCache;
use nomi_tools::read::ReadTool;
use nomi_types::message::{ContentBlock, Message, Role};
use serde_json::json;
use std::sync::{Arc, RwLock};

#[tokio::test]
async fn tc_ax_01_multi_feature_collaboration() {
    // ── Step 1: Setup memory directory with MEMORY.md and an entry ──
    let tmp = tempfile::TempDir::new().unwrap();
    let mem_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&mem_dir).unwrap();
    std::fs::write(
        mem_dir.join("MEMORY.md"),
        "- [Preferences](prefs.md) \u{2014} user prefers dark theme\n",
    )
    .unwrap();
    std::fs::write(
        mem_dir.join("prefs.md"),
        "The user prefers dark theme and compact layout.\n",
    )
    .unwrap();

    // ── Step 2: Build system prompt with memory ──
    let system_prompt = build_system_prompt(
        &mut SystemPromptCache::new(),
        None,
        "/tmp",
        "test-model",
        &[],
        None,
        Some(&mem_dir),
        false, // plan_mode_active = false
        false,
        false, // browser_enabled
    );

    // Assert: system prompt contains memory content
    assert!(
        system_prompt.contains("auto memory"),
        "system prompt should contain memory system display name"
    );
    assert!(
        system_prompt.contains("prefs.md"),
        "system prompt should reference the memory entry file"
    );

    // Assert: system prompt contains tool guidance
    assert!(
        system_prompt.contains("# Using your tools"),
        "system prompt should contain tool usage guidance heading"
    );

    // ── Step 3: ReadTool dedup via FileStateCache ──
    let cache_config = FileCacheConfig {
        max_entries: 100,
        max_size_bytes: 25 * 1024 * 1024,
        enabled: true,
    };
    let cache = Arc::new(RwLock::new(FileStateCache::new(&cache_config)));
    let read_tool = ReadTool::new(Some(Arc::clone(&cache)), None);

    let test_file = tmp.path().join("test_read.txt");
    std::fs::write(&test_file, "line one\nline two\nline three\n").unwrap();

    let input = json!({ "file_path": test_file.to_str().unwrap() });

    // First read: full content
    let r1 = read_tool.execute(input.clone()).await;
    assert!(!r1.is_error, "first read should succeed");
    assert!(
        r1.content.contains("line one"),
        "first read should return file content"
    );

    // Second read: dedup stub (file unchanged)
    let r2 = read_tool.execute(input).await;
    assert!(!r2.is_error, "second read should succeed");
    assert!(
        r2.content.contains("unchanged since last read"),
        "second read should return dedup stub, got: {}",
        r2.content
    );

    // ── Step 4: Microcompact clears old tool results ──
    let mut messages = Vec::new();
    for i in 0..8 {
        let id = format!("t{i}");
        messages.push(Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: id.clone(),
                name: "Read".to_string(),
                input: json!({}),
                extra: None,
            }],
        ));
        messages.push(Message::new(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: id,
                content: format!("file-content-{i}"),
                is_error: false,
                images: Vec::new(),
            }],
        ));
    }

    let compact_config = CompactConfig {
        micro_keep_recent: 3,
        ..CompactConfig::default()
    };
    let result = microcompact(&mut messages, &compact_config);

    // Assert: microcompact cleared some old tool results
    assert!(
        result.cleared_count > 0,
        "microcompact should clear old tool results, got cleared_count={}",
        result.cleared_count
    );

    // Verify cleared results contain the placeholder
    let cleared_results: Vec<_> = messages
        .iter()
        .flat_map(|m| &m.content)
        .filter(|b| matches!(b, ContentBlock::ToolResult { content, .. } if content == CLEARED_TOOL_RESULT))
        .collect();
    assert_eq!(
        cleared_results.len(),
        result.cleared_count,
        "number of cleared placeholders should match cleared_count"
    );
}
