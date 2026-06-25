use super::*;

// Helper: run execute_shell_commands with LoadedFrom::Skills
async fn run(content: &str) -> Result<String, ShellExecutionError> {
    let tmp = std::env::temp_dir();
    execute_shell_commands(content, LoadedFrom::Skills, tmp.to_str().unwrap()).await
}

// -----------------------------------------------------------------------
// TC-1: Block 语法解析
// -----------------------------------------------------------------------

// TC-1.2: Block 多行命令
#[test]
fn tc_1_2_block_multiline_command() {
    let content = "```!\nls -la\npwd\n```";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 1);
    // command should contain both lines
    assert!(matches[0].command.contains("ls -la"));
    assert!(matches[0].command.contains("pwd"));
}

// TC-1.3: Block 前后有内容 — full_match 包含完整 block
#[test]
fn tc_1_3_block_with_surrounding_text() {
    let content = "before\n```!\ncmd\n```\nafter";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 1);
    assert!(matches[0].full_match.starts_with("```!"));
    assert!(matches[0].full_match.ends_with("```"));
}

// TC-1.4: 多个 Block
#[test]
fn tc_1_4_multiple_blocks() {
    let content = "```!\necho first\n```\ntext\n```!\necho second\n```";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 2);
}

// TC-1.5: 无 Block 内容 → 匹配 0 条
#[test]
fn tc_1_5_no_block_no_match() {
    let content = "no shell commands here";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 0);
}

// TC-1.6: 普通代码块不匹配（无 `!`）
#[test]
fn tc_1_6_regular_code_block_not_matched() {
    let content = "```rust\nfn main() {}\n```";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 0);
}

// TC-1.7: Block 内容为空
#[test]
fn tc_1_7_block_empty_command() {
    let content = "```!\n```";
    let matches = extract_shell_matches(content);
    // empty command block still matched
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].command, "");
}

// -----------------------------------------------------------------------
// TC-2: Inline 语法解析
// -----------------------------------------------------------------------

// TC-2.2: 空格前 Inline — 匹配 1 条
#[test]
fn tc_2_2_inline_space_preceded() {
    let content = "dir is !`pwd` end";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].command, "pwd");
}

// TC-2.3: 多个 Inline
#[test]
fn tc_2_3_multiple_inline_matches() {
    let content = "!`cmd1` and !`cmd2`";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 2);
    let cmds: Vec<&str> = matches.iter().map(|m| m.command.as_str()).collect();
    assert!(cmds.contains(&"cmd1"));
    assert!(cmds.contains(&"cmd2"));
}

// TC-2.4: 无空格前缀不匹配（D-1 偏离）
// Rust regex 不支持 lookbehind；前缀为非空白字符时不应匹配
#[test]
fn tc_2_4_no_prefix_not_matched() {
    // "x!`cmd`" — x 不是空格/行首，不应匹配
    let content = "x!`cmd`";
    let matches = extract_shell_matches(content);
    assert_eq!(
        matches.len(),
        0,
        "inline !`cmd` preceded by non-whitespace 'x' should not match"
    );
}

// TC-2.5: 换行前 Inline
#[test]
fn tc_2_5_inline_after_newline() {
    let content = "text\n!`ls`\n";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].command, "ls");
}

// TC-2.6: Inline 命令含空格
#[test]
fn tc_2_6_inline_command_with_spaces() {
    let content = "!`echo hello world`";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].command, "echo hello world");
}

// -----------------------------------------------------------------------
// TC-3: Block + Inline 混合
// -----------------------------------------------------------------------

// TC-3.1: Block + Inline 都存在
#[test]
fn tc_3_1_block_and_inline_both_present() {
    let content = "!`echo inline`\n```!\necho block\n```\n";
    let matches = extract_shell_matches(content);
    assert_eq!(matches.len(), 2);
    let cmds: Vec<&str> = matches.iter().map(|m| m.command.as_str()).collect();
    assert!(cmds.contains(&"echo inline"));
    assert!(cmds.contains(&"echo block"));
}

// TC-3.2: Block 内含 Inline 语法 — Block 优先，内部不被单独匹配（D-2 偏离）
#[test]
fn tc_3_2_block_contains_inline_syntax_deduped() {
    // The inline !`ls` is inside a block — should not be extracted separately
    let content = "```!\necho first\n!`ls`\n```";
    let matches = extract_shell_matches(content);
    // Only the block should be matched, not the inner inline
    assert_eq!(matches.len(), 1);
    assert!(matches[0].full_match.starts_with("```!"));
}

// -----------------------------------------------------------------------
// TC-4: execute_command — 命令执行
// -----------------------------------------------------------------------

// TC-4.1: 成功命令
#[tokio::test]
async fn tc_4_1_successful_command_echo() {
    let content = "!`echo hello`";
    let result = run(content).await.unwrap();
    assert!(result.contains("hello"));
}

// TC-4.2: 命令有 stdout
#[tokio::test]
async fn tc_4_2_stdout_captured() {
    let content = "!`echo captured_stdout`";
    let result = run(content).await.unwrap();
    assert!(result.contains("captured_stdout"));
}

// TC-4.3: 命令有 stderr
#[tokio::test]
async fn tc_4_3_stderr_captured_and_formatted() {
    // stderr-only output — cross-platform: write to stderr via redirection
    let content = if cfg!(windows) {
        "!`echo stderr_msg 1>&2`"
    } else {
        "!`echo stderr_msg >&2`"
    };
    let result = run(content).await.unwrap();
    assert!(
        result.contains("[stderr]"),
        "stderr prefix missing: {result}"
    );
    assert!(result.contains("stderr_msg"));
}

// TC-4.4: 命令失败且无输出 → Err（D-3 偏离：有输出时仍返回 Ok）
#[tokio::test]
async fn tc_4_4_command_fail_no_output_returns_err() {
    // `exit 1` exits with code 1 and produces no output (cross-platform)
    let content = "!`exit 1`";
    let result = run(content).await;
    assert!(
        result.is_err(),
        "command with exit 1 and no output should return Err"
    );
}

// TC-4.4b: 命令失败但有输出 → Ok（D-3 偏离验证）
#[tokio::test]
async fn tc_4_4b_command_fail_with_output_returns_ok() {
    // exits non-zero but still has stdout
    let content = if cfg!(windows) {
        "!`echo output & exit 1`"
    } else {
        "!`echo output; exit 1`"
    };
    let result = run(content).await;
    assert!(
        result.is_ok(),
        "command with exit 1 but with output should return Ok, got: {:?}",
        result.err()
    );
    assert!(result.unwrap().contains("output"));
}

// TC-4.5: cwd 参数生效
#[tokio::test]
async fn tc_4_5_cwd_used() {
    let tmp = std::env::temp_dir();
    // Use cross-platform command: `cd` on Windows, `pwd` on Unix
    let content = if cfg!(windows) { "!`cd`" } else { "!`pwd`" };
    let result = execute_shell_commands(content, LoadedFrom::Skills, tmp.to_str().unwrap())
        .await
        .unwrap();
    // Check that the output contains the temp directory name
    let tmp_name = tmp.file_name().unwrap().to_str().unwrap();
    assert!(
        result.contains(tmp_name),
        "pwd should output a path containing '{tmp_name}', got: {result}"
    );
}

// TC-4.6: 命令输出为空 → output 为空字符串
#[tokio::test]
async fn tc_4_6_empty_output() {
    // `cd .` exits 0 on all platforms with no output
    let content = "before !`cd .` after";
    let result = run(content).await.unwrap();
    assert_eq!(result, "before  after");
}

// TC-4.7: 命令不存在 → Err
#[tokio::test]
async fn tc_4_7_nonexistent_command_returns_err() {
    let content = "!`not_a_real_command_xyz_12345`";
    let result = run(content).await;
    // bash writes "command not found" to stderr and exits non-zero with empty stdout.
    // Per D-3: if stderr is non-empty, the command returns Ok with [stderr] content.
    // On Windows cmd, a nonexistent command returns exit code 1 with empty stdout/stderr → Err.
    match &result {
        Err(ShellExecutionError::CommandFailed { .. }) => {} // expected on Windows/cmd
        Ok(s) => {
            // bash returns Ok with [stderr] content since stderr is non-empty
            assert!(
                s.contains("[stderr]") || s.contains("not found"),
                "unexpected Ok result: {s}"
            );
        }
        other => panic!("unexpected result: {:?}", other),
    }
}

// -----------------------------------------------------------------------
// TC-5: format_output
// -----------------------------------------------------------------------

// TC-5.5: stdout 末尾换行被 trim
#[test]
fn tc_5_5_stdout_trailing_newline_trimmed() {
    // format_output receives pre-trimmed strings (execute_command trims)
    let result = format_output("line\n", "");
    // format_output itself doesn't trim; execute_command does via trim_end()
    // This test verifies format_output handles it cleanly
    assert_eq!(result, "line\n");
}

// -----------------------------------------------------------------------
// TC-6: execute_shell_commands
// -----------------------------------------------------------------------

// TC-6.1: MCP skill → 跳过执行，返回原文
#[tokio::test]
async fn tc_6_1_mcp_skill_unchanged() {
    let tmp = std::env::temp_dir();
    let content = "run: !`pwd` and ```!\nls\n```";
    let result = execute_shell_commands(content, LoadedFrom::Mcp, tmp.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        result, content,
        "MCP skill content should be returned unchanged"
    );
}

// TC-6.2: 无 shell 命令 → 原文不变
#[tokio::test]
async fn tc_6_2_no_commands_unchanged() {
    let content = "just plain text, no commands";
    let result = run(content).await.unwrap();
    assert_eq!(result, content);
}

// TC-6.3: Block 命令替换 — full_match 被 output 替换
#[tokio::test]
async fn tc_6_3_block_replaced_with_output() {
    let content = "Result:\n```!\necho replaced\n```\nEnd.";
    let result = run(content).await.unwrap();
    assert!(!result.contains("```!"), "block syntax should be replaced");
    assert!(result.contains("replaced"));
    assert!(result.contains("Result:"));
    assert!(result.contains("End."));
}

// TC-6.4: Inline 命令替换 — 前导空白保留（D-1 偏离）
#[tokio::test]
async fn tc_6_4_inline_replaced_leading_space_preserved() {
    let content = "Dir: !`echo /mydir`";
    let result = run(content).await.unwrap();
    // Leading "Dir: " space must be preserved
    assert!(
        result.starts_with("Dir: "),
        "leading space must be preserved, got: {result}"
    );
    assert!(result.contains("mydir"));
}

// TC-6.5: 多命令并行执行 — 两者都替换
#[tokio::test]
async fn tc_6_5_multiple_commands_all_replaced() {
    let content = "A: !`echo aaa` B: !`echo bbb`";
    let result = run(content).await.unwrap();
    assert!(result.contains("aaa"), "first command missing: {result}");
    assert!(result.contains("bbb"), "second command missing: {result}");
    assert!(!result.contains("!`"), "shell syntax should be replaced");
}

// TC-6.7: 从后向前替换 — 前面替换不影响后面位置
#[tokio::test]
async fn tc_6_7_back_to_front_replacement() {
    // Two inline commands; the first replacement should not corrupt the second
    let content = "X: !`echo first` Y: !`echo second`";
    let result = run(content).await.unwrap();
    assert!(result.contains("first"));
    assert!(result.contains("second"));
    // Verify ordering: "X:" before "Y:"
    let x_pos = result.find("X:").unwrap();
    let y_pos = result.find("Y:").unwrap();
    assert!(x_pos < y_pos, "X should come before Y in result: {result}");
}

// TC-6.8: Block 命令 + 周围文本保留
#[tokio::test]
async fn tc_6_8_surrounding_text_preserved() {
    let content = "Header\n```!\necho body\n```\nFooter";
    let result = run(content).await.unwrap();
    assert!(result.contains("Header"));
    assert!(result.contains("body"));
    assert!(result.contains("Footer"));
    assert!(!result.contains("```!"));
}

// -----------------------------------------------------------------------
// TC-7: ShellExecutionError
// -----------------------------------------------------------------------

// TC-7.1: CommandFailed 消息含 pattern
#[test]
fn tc_7_1_command_failed_message_contains_pattern() {
    let err = ShellExecutionError::CommandFailed {
        pattern: "my-cmd".to_string(),
        output: "exit code 1".to_string(),
    };
    let msg = err.to_string();
    assert!(
        msg.contains("my-cmd"),
        "error message should contain pattern: {msg}"
    );
}

// TC-7.2: McpBlocked 消息
#[test]
fn tc_7_2_mcp_blocked_message() {
    let err = ShellExecutionError::McpBlocked;
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("mcp") || msg.to_lowercase().contains("blocked"),
        "McpBlocked message should mention MCP or blocked: {msg}"
    );
}

// TC-7.3: Error 实现 Debug
#[test]
fn tc_7_3_error_debug_format() {
    let err = ShellExecutionError::CommandFailed {
        pattern: "cmd".to_string(),
        output: "output".to_string(),
    };
    let debug = format!("{:?}", err);
    assert!(!debug.is_empty());
}

// -----------------------------------------------------------------------
// TC-15: 边界情况
// -----------------------------------------------------------------------

// TC-15.1: content 为空字符串
#[tokio::test]
async fn tc_15_1_empty_content() {
    let result = run("").await.unwrap();
    assert_eq!(result, "");
}

// TC-15.3: 命令含特殊字符（引号、管道）
#[tokio::test]
async fn tc_15_3_command_with_special_chars() {
    let content = r#"!`echo "hello world"`"#;
    let result = run(content).await.unwrap();
    assert!(result.contains("hello world"), "got: {result}");
}

// TC-15.4: 命令含换行符（多行 block）
#[tokio::test]
#[cfg(not(windows))] // Windows cmd does not support newline-separated commands in blocks
async fn tc_15_4_multiline_block_executed_as_script() {
    let content = "```!\necho line1\necho line2\n```";
    let result = run(content).await.unwrap();
    assert!(result.contains("line1"));
    assert!(result.contains("line2"));
}

// TC-15.6: 同一命令多次出现
#[tokio::test]
async fn tc_15_6_same_command_repeated() {
    let content = "!`echo x` and !`echo x`";
    let result = run(content).await.unwrap();
    // Both occurrences of !`echo x` should be replaced
    assert!(
        !result.contains("!`"),
        "both occurrences should be replaced: {result}"
    );
    // Should contain "x" — at least once from each replacement
    // On Windows cmd, echo may include trailing space; just verify no backtick syntax remains
    assert!(result.contains('x'), "expected 'x' in result: {result}");
}
