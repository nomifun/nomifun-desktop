use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;

use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};

use crate::Tool;

pub struct GrepTool {
    cwd: PathBuf,
}

impl GrepTool {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Searches file contents using regex patterns (powered by ripgrep).\n\n\
         IMPORTANT: ALWAYS use this Grep tool for content search. \
         NEVER run grep or rg as a Bash command.\n\n\
         - Supports full regex syntax (e.g., \"log.*Error\", \"fn\\\\s+\\\\w+\").\n\
         - Use the glob parameter to filter by file pattern (e.g., \"*.rs\").\n\
         - Set context_lines (e.g. 2) to include surrounding lines for each match.\n\
         - Output is capped at 250 lines; when truncated, a notice reports the \
         true total so you can narrow the pattern or glob.\n\
         - Set case_insensitive to true for case-insensitive search."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default: cwd)"
                },
                "glob": {
                    "type": "string",
                    "description": "File filter pattern, e.g. \"*.rs\""
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Lines of context to show around each match (rg -C). Default 0."
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case insensitive search"
                }
            },
            "required": ["pattern"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let Some(pattern) = input["pattern"].as_str() else {
            return ToolResult {
                content: "Missing required parameter: pattern".to_string(),
                is_error: true,
                images: Vec::new(),
            };
        };

        for name in ["path", "glob"] {
            if input.get(name).is_some_and(|value| !value.is_null() && !value.is_string()) {
                return ToolResult::error(format!("{name} must be a string"));
            }
        }
        let raw_path = input["path"].as_str().unwrap_or(".");
        let path = crate::path_guard::resolve_against_cwd(raw_path, Some(&self.cwd));

        tracing::debug!(cwd = %self.cwd.display(), resolved_path = %path, pattern = %pattern, "GrepTool searching");

        let glob_pattern = input["glob"].as_str();
        let case_insensitive = match input.get("case_insensitive") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            _ => return ToolResult::error("case_insensitive must be a boolean"),
        };
        let context_lines = match input.get("context_lines") {
            None | Some(Value::Null) => 0,
            Some(value) => match value.as_u64().and_then(|value| usize::try_from(value).ok()) {
                Some(value) => value,
                None => return ToolResult::error("context_lines must be a non-negative integer that fits usize"),
            },
        };

        // Try ripgrep first, fallback to grep
        let result = try_ripgrep(pattern, &path, glob_pattern, case_insensitive, context_lines).await;

        match result {
            Ok(output) => output,
            Err(_) => {
                // Fallback to grep (now also honours glob + context_lines on unix)
                try_grep(pattern, &path, glob_pattern, case_insensitive, context_lines).await
            }
        }
    }

    fn max_result_size(&self) -> usize {
        20_000
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn describe(&self, input: &Value) -> String {
        let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        let raw_path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        format!("Grep '{}' in {}", pattern, raw_path)
    }
}

const GREP_MAX_LINES: usize = 250;

/// Cap grep output to `max_lines`, appending a truncation notice with the true
/// total when exceeded — so the model knows results were cut and can narrow the
/// search, instead of silently losing matches.
fn format_grep_output(stdout: &str, max_lines: usize) -> String {
    let total = stdout.lines().count();
    if total <= max_lines {
        return stdout.trim_end().to_string();
    }
    let shown: Vec<&str> = stdout.lines().take(max_lines).collect();
    format!(
        "{}\n... [truncated: showing first {} of {} output lines — narrow your pattern or set a `glob` filter]",
        shown.join("\n"),
        max_lines,
        total
    )
}

fn ripgrep_command(
    pattern: &str,
    path: &str,
    glob_pattern: Option<&str>,
    case_insensitive: bool,
    context_lines: usize,
) -> Command {
    let mut cmd = Command::new("rg");
    cmd.arg("-n");
    if let Some(g) = glob_pattern {
        cmd.arg("--glob").arg(g);
    }
    if case_insensitive {
        cmd.arg("-i");
    }
    if context_lines > 0 {
        cmd.arg("-C").arg(context_lines.to_string());
    }
    // Patterns and paths are data, including values beginning with a dash.
    cmd.arg("-e").arg(pattern).arg("--").arg(path);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

fn fallback_command(
    pattern: &str,
    path: &str,
    glob_pattern: Option<&str>,
    case_insensitive: bool,
    context_lines: usize,
) -> Result<Command, String> {
    let cmd = if cfg!(windows) {
        if glob_pattern.is_some() || context_lines > 0 {
            return Err("findstr fallback cannot honor glob or context_lines; install ripgrep to use these options".to_string());
        }
        let mut c = Command::new("findstr");
        let is_dir = Path::new(path).is_dir();
        if is_dir {
            c.arg("/S");
        }
        c.arg("/N").arg("/R");
        if case_insensitive {
            c.arg("/I");
        }
        c.arg(format!("/C:{pattern}"));
        if is_dir {
            c.arg(format!("{}\\*", path.trim_end_matches(['\\', '/'])));
        } else {
            c.arg(path);
        }
        #[cfg(windows)]
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        c
    } else {
        let mut c = Command::new("grep");
        c.arg("-rn");
        if case_insensitive {
            c.arg("-i");
        }
        if let Some(g) = glob_pattern {
            c.arg(format!("--include={g}"));
        }
        if context_lines > 0 {
            c.arg("-C").arg(context_lines.to_string());
        }
        c.arg("-e").arg(pattern).arg("--").arg(path);
        c
    };
    Ok(cmd)
}

fn render_search_output(program: &str, output: std::process::Output) -> ToolResult {
    if !matches!(output.status.code(), Some(0 | 1)) {
        return ToolResult::error(format!(
            "{program} error ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.is_empty() {
        ToolResult::text("No matches found")
    } else {
        ToolResult::text(format_grep_output(&stdout, GREP_MAX_LINES))
    }
}

async fn try_ripgrep(
    pattern: &str,
    path: &str,
    glob_pattern: Option<&str>,
    case_insensitive: bool,
    context_lines: usize,
) -> Result<ToolResult, std::io::Error> {
    let output = ripgrep_command(pattern, path, glob_pattern, case_insensitive, context_lines)
        .output()
        .await?;
    Ok(render_search_output("rg", output))
}

async fn try_grep(
    pattern: &str,
    path: &str,
    glob_pattern: Option<&str>,
    case_insensitive: bool,
    context_lines: usize,
) -> ToolResult {
    let mut cmd = match fallback_command(pattern, path, glob_pattern, case_insensitive, context_lines) {
        Ok(cmd) => cmd,
        Err(error) => return ToolResult::error(error),
    };
    let program = if cfg!(windows) { "findstr" } else { "grep" };
    match cmd.output().await {
        Ok(output) => render_search_output(program, output),
        Err(error) => ToolResult::error(format!("{program} failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_grep_output_appends_truncation_notice_with_total() {
        let lines: String = (0..300).map(|i| format!("line{i}\n")).collect();
        let out = super::format_grep_output(&lines, 250);
        assert!(out.contains("truncated"), "must announce truncation: {out}");
        assert!(out.contains("300 output lines"), "must count output lines, including context");
        // 250 shown lines + 1 notice line
        assert_eq!(out.lines().count(), 251);
    }

    #[test]
    fn format_grep_output_short_is_unchanged() {
        let out = super::format_grep_output("a\nb\nc\n", 250);
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn search_commands_keep_patterns_separate_from_options() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("search file.txt");
        std::fs::write(&file, "").unwrap();
        let path = file.to_str().unwrap();
        for pattern in ["--files", "/?", "two words"] {
            let cmd = ripgrep_command(pattern, path, Some("*.rs"), true, 2);
            let args = cmd.as_std().get_args().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>();
            assert_eq!(args, ["-n", "--glob", "*.rs", "-i", "-C", "2", "-e", pattern, "--", path]);

            let cmd = fallback_command(pattern, path, None, true, 0).unwrap();
            let args = cmd.as_std().get_args().map(|arg| arg.to_str().unwrap()).collect::<Vec<_>>();
            #[cfg(windows)]
            assert_eq!(args, ["/N", "/R", "/I", &format!("/C:{pattern}"), path]);
            #[cfg(not(windows))]
            assert_eq!(args, ["-rn", "-i", "-e", pattern, "--", path]);
        }
        #[cfg(windows)]
        for (glob, context) in [(Some("*.rs"), 0), (None, 2)] {
            assert!(fallback_command("pattern", path, glob, false, context).is_err());
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn search_exit_errors_are_not_reported_as_no_matches() {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;

        for program in ["rg", "grep", "findstr"] {
            for (code, stdout) in [(0, "one\n"), (1, ""), (2, ""), (2, "partial\n")] {
                let raw_status = if cfg!(unix) { code << 8 } else { code };
                let result = render_search_output(program, std::process::Output {
                    status: std::process::ExitStatus::from_raw(raw_status),
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: b"simulated search failure".to_vec(),
                });
                assert_eq!(result.is_error, code == 2);
                if code == 2 {
                    assert!(result.content.contains(program));
                    assert!(result.content.contains("simulated search failure"));
                    assert!(!result.content.contains("No matches"));
                } else {
                    assert_eq!(result.content, if code == 1 { "No matches found" } else { "one" });
                }
            }
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn try_grep_searches_files_and_spaced_directories_with_literal_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        let directory = tmp.path().join("search root");
        let nested = directory.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let file = directory.join("search file.txt");
        std::fs::write(&file, "--files primary_match\r\n/? primary_match\r\ntwo words primary_match\r\n").unwrap();
        std::fs::write(nested.join("search file.txt"), "--files nested_match\r\n/? nested_match\r\ntwo words nested_match\r\n").unwrap();

        for pattern in ["--files", "/?", "two words"] {
            for (path, recursive) in [(&file, false), (&directory, true)] {
                // Call the fallback directly even when ripgrep is installed.
                let result = try_grep(pattern, path.to_str().unwrap(), None, false, 0).await;
                assert!(!result.is_error, "{}", result.content);
                assert!(result.content.contains(&format!("{pattern} primary_match")), "{}", result.content);
                assert_eq!(result.content.contains("nested_match"), recursive, "{}", result.content);
                assert_eq!(result.content.lines().count(), if recursive { 2 } else { 1 }, "{}", result.content);
            }
        }
    }

    #[tokio::test]
    async fn execute_uses_cwd_for_relative_path() {
        use std::fs;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("searchable.txt"), "unique_grep_marker_xyz\n--files\n").unwrap();

        let tool = GrepTool::new(tmp.path().to_path_buf());
        for (name, value) in [
            ("path", json!(false)),
            ("glob", json!(["*.txt"])),
            ("case_insensitive", json!("false")),
            ("context_lines", json!(-1)),
            ("context_lines", json!(1.5)),
        ] {
            let result = tool.execute(json!({"pattern": "marker", (name): value})).await;
            assert!(result.is_error, "invalid {name} must not broaden the search");
            assert!(result.content.contains(name), "{}", result.content);
        }
        let input = json!({"pattern": "unique_grep_marker_xyz", "path": "."});
        let result = tool.execute(input).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(
            result.content.contains("unique_grep_marker_xyz"),
            "should find pattern, got: {}",
            result.content
        );
        let result = tool.execute(json!({"pattern": "--files", "path": "."})).await;
        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("--files"), "pattern must not become an option: {}", result.content);
    }
}
