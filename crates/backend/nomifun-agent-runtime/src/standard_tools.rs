//! Standard model-facing Nomi Tool definitions.
//!
//! These definitions are a presentation layer only. Every Tool is compiled
//! against the immutable Snapshot by `compile_agent_tool_plan`, and execution
//! still goes through the NomiFun Capability Kernel.

use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
use nomifun_chat_model_broker::ChatToolDefinition;
use serde_json::{Value, json};

use crate::kernel::AgentToolExposure;

/// Candidate platform actions known by this adapter. The exact Snapshot and
/// Action grants filter them during compilation; there is no runtime/profile
/// level that can widen or narrow authority.
pub fn standard_agent_tool_exposures() -> Vec<AgentToolExposure> {
    STANDARD_TOOLS
        .iter()
        .map(StandardTool::exposure)
        .collect()
}

struct StandardTool {
    model_name: &'static str,
    capability_id: &'static str,
    action_id: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

impl StandardTool {
    fn exposure(&self) -> AgentToolExposure {
        let description = if matches!(
            self.action_id,
            "workspace.process/exec" | "workspace.process/start"
        ) {
            let host_guidance = match std::env::consts::OS {
                "windows" => "The JSON field cmd is a Windows PowerShell 5.1 script, NOT cmd.exe. For workspace text and literal search, use read_file/search_files. Windows PowerShell 5.1 Get-Content without -Encoding can decode a BOM-less UTF-8 file as the local ANSI code page: correct hash/line counts do not prove correct decoded text. If a native shell read of a known UTF-8 file is required, use Get-Content -LiteralPath 'path' -Encoding UTF8 -Raw; use -Encoding UTF8 for Select-String too. Do not guess an unknown file encoding. Do not use cd && dir /a, &&, ||, ??, ??=, or ?: in that field. For the exact current directory use {\"cmd\":\"(Get-Location).Path\"}; plain Get-Location prints a table that can truncate the path. For root entries and their actual flags use {\"cmd\":\"Get-ChildItem -LiteralPath . -Force | ForEach-Object { [pscustomobject]@{Name=$_.Name;Attributes=$_.Attributes.ToString();Hidden=[bool]($_.Attributes -band [IO.FileAttributes]::Hidden);System=[bool]($_.Attributes -band [IO.FileAttributes]::System);LinkType=$_.LinkType} } | ConvertTo-Json -Compress\"}. Attributes is a flags enum: test each Hidden/System bit with a Boolean cast or compare the bitwise result to zero. FileAttributes.Normal is 128, not zero; comparing a masked Hidden bit to Normal incorrectly marks every entry hidden. Keep the shown Boolean-mask example. Include all returned names; describe Hidden/System only from those flags. A dot-prefixed Archive-only file is not hidden on Windows. A names-only listing cannot prove attributes. Do not recurse or follow links unless requested. For Command Prompt scripts use {\"command\":\"cmd.exe\",\"args\":[\"/d\",\"/c\",\"cd & dir /a\"]}. Do not prefix cmd with powershell.exe or pwsh. command plus args invokes a literal executable.",
                "macos" => "cmd invokes /bin/sh -c; command plus args invokes a literal executable. For the physical current directory, use {\"command\":\"/bin/pwd\",\"args\":[\"-P\"]}. For a top-level listing including dot entries, use {\"command\":\"/bin/ls\",\"args\":[\"-a\"]}. These are executable forms; omit cmd. Do not put the entire command line in command. Omit . and .. from business entry counts.",
                "linux" => "cmd invokes /bin/sh -c; command plus args invokes a literal executable. For the physical current directory, use {\"command\":\"/usr/bin/pwd\",\"args\":[\"-P\"]}. For a top-level listing including dot entries, use {\"command\":\"/usr/bin/ls\",\"args\":[\"-a\"]}. These are executable forms; omit cmd. Do not put the entire command line in command. Omit . and .. from business entry counts.",
                _ => "Select native commands for this process host; the UI client's OS does not determine command syntax.",
            };
            format!("Workspace process host OS: {}. The following command examples describe input syntax only. Run them only when the accepted task requires their operation; do not add a directory listing, cwd probe or other preflight solely to prepare an exact-path file task. {host_guidance} {}", std::env::consts::OS, self.description)
        } else {
            self.description.to_owned()
        };
        let description = if matches!(self.model_name,
            "read_file" | "search_files" | "write_file" | "apply_patch" | "delete_path" | "exec_command" | "start_process") {
            format!("{} The workspace root is already the selected project directory; the process default cwd is the same root. Do not prepend a project or conversation display name or create a directory to compensate for that prefix. Keep the exact workspace-relative paths supplied by the user. Discover instructions at '.', not a guessed project subfolder.",description)
        } else { description };
        AgentToolExposure {
            definition: ChatToolDefinition {
                name: self.model_name.to_owned(),
                description,
                input_schema: StrictJsonValue((self.schema)()),
                deferred: false,
            },
            capability_id: CapabilityId::from(self.capability_id),
            action_id: ActionId::from(self.action_id),
        }
    }
}

const STANDARD_TOOLS: &[StandardTool] = &[
    StandardTool {
        model_name: "read_file",
        capability_id: "workspace.files",
        action_id: "workspace.files/read",
        description: "Read a workspace file or inspect instruction scope. Repository instructions are not ordinary text: never use the default text format for AGENTS.md or AGENTS.override.md. Their applicable bodies are injected separately after instruction discovery. Before reading a source file in any scope not already discovered, call read_file alone on its directory with format=instruction_scope (path=. for the workspace root; recursive=true only when needed); after that result, reconsider and send source reads with fresh call IDs. For format=instruction_scope, recursive is the only optional mode field; NEVER include missing_ok, offset, limit, start_line, line_count, or expected_sha256. Default format=text. For source inspection use start_line (1-based) and line_count (default 200), without offset/limit. A small file can be read with just path. Byte pagination remains available: bounded UTF-8 pages, source at most 8 MiB; offset reads are independent current-version observations. In every successful text result, sha256 always identifies the entire source and total_bytes is its full size, even when content is only one line/byte page and eof=false; do not read the whole file only to obtain its digest. To assemble consistent pages, follow next_offset with the prior expected_sha256 until eof; explicit hash mismatches still fail. Offsets/limit are bytes, not lines. FILE_CONTENT_CHANGED means discard prior pages and restart. Text-only missing_ok=true returns workspace_file_absent for genuine absence, never for denied access. format=image: PNG/JPEG/WebP at most 4 MiB, omit offset/limit/missing_ok and submit alone; requires an image-capable exact model route. Returns prepared pixels, not base64 text; images may be resized and must be re-read after history/compaction omitted pixels. Optional expected_sha256 guards text/image source versions. format=instruction_scope: metadata for a file or directory (path=. for workspace root). Optional recursive=true discovers descendant instruction directories, including hidden/ignored entries, within bounded limits. Check complete/incomplete_reasons and use canonical_path; incomplete is not absence. This is not a filesystem snapshot or proof of shell access scope.",
        schema: read_schema,
    },
    StandardTool {
        model_name: "search_files",
        capability_id: "workspace.files",
        action_id: "workspace.files/search",
        description: "Fresh bounded literal single-line UTF-8 workspace search. Directory walks respect hidden/ignore rules and skip symlink entries. One match per line includes a snippet around the match, byte_offset and whole-source sha256 for read_file. Empty matches are not proof of absence; inspect truncated/incomplete_reasons/files_skipped and narrow path/query when needed. Not a filesystem snapshot.",
        schema: search_schema,
    },
    StandardTool {
        model_name: "git_status",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/status",
        description: "Inspect Git status for the bound workspace. A new directory without Git returns is_repository=false as a normal observation; this does not initialize Git.",
        schema: empty_schema,
    },
    StandardTool {
        model_name: "git_diff",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/diff",
        description: "Return the bound workspace diff, optionally scoped to one workspace-relative path. Omit path or use '.' for the workspace root, even when the workspace is a repository subdirectory.",
        schema: optional_path_schema,
    },
    StandardTool {
        model_name: "write_file",
        capability_id: "workspace.files",
        action_id: "workspace.files/write",
        description: "Write exact complete UTF-8 text (at most 8 MiB) through the workspace owner; no trailing newline is added. Include any requested final line break in content itself. Missing parent directories are created automatically; no shell mkdir is needed. The receipt includes published bytes, line_count and sha256, so a command solely to count the file is unnecessary. These facts are not functional-test results. This replaces the whole file; inspect existing content and prefer apply_patch for focused edits. A prior read is not a write lock.",
        schema: write_schema,
    },
    StandardTool {
        model_name: "apply_patch",
        capability_id: "workspace.files",
        action_id: "workspace.files/patch",
        description: "Apply bounded, ordered, exact line hunks through the workspace owner. Missing parent directories are created after the entire patch is validated. Prefer each file's expected_source={kind:existing,sha256:<full read_file digest>} or {kind:absent} after observing absence; this detects changes outside the hunk too. Omission/any preserves legacy line-only matching, not a source-version check. Never remove a rejected guard just to retry; re-read and replan. Supply logical text without CR/LF or a file-leading UTF-8 BOM; source BOM, unchanged line endings and EOF-newline policy are preserved, added lines use the first source ending (LF if none). For a one-line replacement use old_start=1, old_lines=1, new_start=1, new_lines=1 and lines=[{kind:remove,text:old},{kind:add,text:new}]. A context line is retained and counts in both old_lines and new_lines; context+add inserts after that line and does not replace it. Nonempty ranges are 1-based; old_lines=0 inserts after old_start source lines (0=BOF), new_lines=0 names the surviving output prefix. Context/remove text must match exactly; re-read on mismatch. All targets and source guards are prepared before writes, with per-file atomic publication and best-effort restoration of existing files, not a multi-file transaction. Failed patches may retain newly created files; inspect zero-based request.files indices in the failure observation and re-read every target before replanning/retrying. A restored result is historical, not a current-state lock. Concurrent native edits remain possible. Each file's written_sha256 identifies published bytes for a later guarded read_file or patch, not task completion.",
        schema: patch_schema,
    },
    StandardTool {
        model_name: "delete_path",
        capability_id: "workspace.files",
        action_id: "workspace.files/delete",
        description: "Delete one workspace-relative file or directory through the owner.",
        schema: path_schema,
    },
    StandardTool {
        model_name: "git_stage",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/stage",
        description: "Stage one workspace-relative path.",
        schema: path_schema,
    },
    StandardTool {
        model_name: "exec_command",
        capability_id: "workspace.process",
        action_id: "workspace.process/exec",
        description: "Run a workspace process synchronously: this call waits for exit/timeout and does not return a live handle for a later stdin write. If the process needs later input or EOF, use start_process(wait_ms=0,tty=false), then poll/input/close/poll instead; do not choose exec_command merely to obtain READY. Use exactly one input form. For an ordinary executable, use {\"command\":\"program\",\"args\":[\"literal\",\"tokens\"]}; for example, Git status is {\"command\":\"git\",\"args\":[\"status\",\"--short\"]}. For shell syntax, use {\"cmd\":\"script text\"} with no args field. Never put an executable name in cmd when supplying args, and never combine cmd with command or args. With command, args must be an actual JSON array, never a quoted string containing JSON. Command alone never gets silently split or evaluated as shell text. Use read_file/search_files for workspace file contents, source digests and literal text search; instruction_scope reports instruction locations, not directory entries or OS file attributes. Inspect truncated/incomplete_reasons before claiming a complete or absent search result. Do not substitute a shell wildcard such as .\\**\\* for a recursive workspace search. Never suppress stderr, catch and ignore errors, or redirect errors to null to manufacture a successful empty search; preserve errors and narrow the requested path. Shell exit 0 alone does not prove complete search coverage. A zero exit is an observation, not proof that verification passed.",
        schema: process_launch_schema,
    },
    StandardTool {
        model_name: "start_process",
        capability_id: "workspace.process",
        action_id: "workspace.process/start",
        description: "Start one turn-owned background or interactive process and return its live handle without waiting for exit. For READY/input/EOF use wait_ms=0,tty=false (pipe), then poll_process, write_process_stdin, close_process_stdin and poll to exit; PTY is not pipe EOF. The process lifetime defaults to 30 seconds; polling does not renew this deadline. For a user-requested longer interaction or wait, set timeout_ms explicitly to cover that requested duration (maximum 600000 ms). A timeout ends the owned process and triggers cleanup; it is not a user stop. For a start/wait/stop lifecycle, call start_process once with wait_ms=0, then make at least one distinct poll_process call until the expected output, then cancel_process with the exact returned process_id, all in the same Turn. Even when a start receipt already contains output, do not skip poll_process. Do not pre-read a user-specified existing executable or manually probe instruction files solely to launch it; the host injects applicable workspace instructions. Do not run ls, pwd, test -x, or another process preflight solely for that launch; the owner validates executable and cwd. Never use shell backgrounding, raw PID or temporary-log indirection, or split the lifecycle across AgentExecution steps. No process survives turn cleanup.",
        schema: process_start_schema,
    },
    StandardTool {
        model_name: "poll_process",
        capability_id: "workspace.process",
        action_id: "workspace.process/poll",
        description: "Poll a turn-owned process from a bounded output cursor and observe its current state/cleanup evidence. For each following poll, copy the previous receipt's output.next_cursor into cursor and use wait_ms for bounded waiting. Omitting cursor means 0 and replays retained output; unread output returns immediately even with wait_ms=30000, so it cannot serve as a wait loop. After start_process, use poll_process with the exact process_id to wait for expected output before cancellation; do not infer readiness from a PID or temporary file. Report readiness once, then wait without repeating the same running-status narration. A terminal state=cancelled with cleanup.reaped=true is a successful poll observation, not a tool failure.",
        schema: process_poll_schema,
    },
    StandardTool {
        model_name: "write_process_stdin",
        capability_id: "workspace.process",
        action_id: "workspace.process/input",
        description: "Write bounded input to a turn-owned process without trimming or normalization. For exactly one trailing LF, either use input without the LF and append_newline=true, or include the LF in input and omit append_newline/set it false. Combining a final LF in input with true sends two LFs, not one. This is an effect and invalidates older workspace evidence.",
        schema: process_input_schema,
    },
    StandardTool {
        model_name: "close_process_stdin",
        capability_id: "workspace.process",
        action_id: "workspace.process/close_stdin",
        description: "Close stdin for a turn-owned process and then poll it for a terminal observation.",
        schema: process_id_schema,
    },
    StandardTool {
        model_name: "resize_process",
        capability_id: "workspace.process",
        action_id: "workspace.process/resize",
        description: "Resize the PTY of a turn-owned interactive process.",
        schema: process_resize_schema,
    },
    StandardTool {
        model_name: "cancel_process",
        capability_id: "workspace.process",
        action_id: "workspace.process/cancel",
        description: "Cancel a turn-owned process and obtain cleanup evidence. For an explicit stop request, state=cancelled with cleanup.reaped=true is successful stop evidence, not a task failure; cancellation is not rollback.",
        schema: process_id_schema,
    },
    StandardTool {
        model_name: "read_artifact",
        capability_id: "workspace.artifacts",
        action_id: "workspace.artifacts/read",
        description: "Read one bounded page of an exact Session artifact by content-addressed identity.",
        schema: artifact_read_schema,
    },
    StandardTool {
        model_name: "publish_artifact",
        capability_id: "workspace.artifacts",
        action_id: "workspace.artifacts/publish",
        description: "Publish a workspace file to the Session artifact owner with an optional exact source digest.",
        schema: artifact_publish_schema,
    },
    StandardTool {
        model_name: "git_commit",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/commit",
        description: "Create a commit from the staged workspace changes.",
        schema: commit_schema,
    },
    StandardTool {
        model_name: "git_push",
        capability_id: "workspace.vcs",
        action_id: "workspace.vcs/push",
        description: "Publish an explicit refspec to an already configured local/file Git remote. Requires the Agent's workspace.vcs/push grant. Network SSH/HTTPS credentials, force push and ref deletion are unavailable. A timeout or unknown outcome is not permission to retry; inspect platform effect history. Do not push unless the user's task authorizes publication.",
        schema: push_schema,
    },
];

fn empty_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {},
        "required": []
    })
}

fn workspace_path_value() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "maxLength": 4096,
        "pattern": "\\S",
        "description":"Exact path relative to the selected workspace root, already the project directory. Copy the user's relative path without prepending the project/conversation display name. Use '.' for root instruction discovery. Do not invent another project folder or compensate with mkdir."
    })
}

fn path_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": workspace_path_value()
        },
        "required": ["path"]
    })
}

fn read_schema() -> Value {
    json!({
        "type":"object", "additionalProperties":false, "required":["path"],
        "properties": {
            "format":{"type":"string", "enum":["text", "image", "instruction_scope"], "default":"text"},
            "recursive":{"type":"boolean", "default":false},
            "missing_ok":{"type":"boolean", "default":false},
            "path":workspace_path_value(),
            "start_line":{"type":"integer", "minimum":1, "maximum":8388609, "description":"One-based source line. Use this with line_count to inspect code; omit byte offset/limit."},
            "line_count":{"type":"integer", "minimum":1, "maximum":2000, "default":200,"description":"Number of source lines to inspect, still bounded by the response byte budget."},
            "offset":{"type":"integer", "minimum":0, "maximum":8388608, "default":0,"description":"Byte offset, NOT a line number. For pagination use the exact prior next_offset and expected_sha256."},
            "limit":{"type":"integer", "minimum":4, "maximum":16384, "default":16384,"description":"Byte budget, NOT a number of lines. Omit for normal reads; use line_count for source lines."},
            "expected_sha256":{"type":"string", "pattern":"^[0-9a-f]{64}$", "description":"The whole-source SHA-256 returned by any successful text or image read. Use it to pin a later page or source version."}
        },
        "allOf":[{"if":{"properties":{"format":{"enum":["image","instruction_scope"]}},"required":["format"]},"then":{"properties":{"missing_ok":{"const":false}}}},{"if":{"anyOf":[{"required":["start_line"]},{"required":["line_count"]}]},
                  "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]}]}}},
                 {"if":{"properties":{"format":{"const":"image"}},"required":["format"]},
                  "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]},{"required":["start_line"]},{"required":["line_count"]}]}}},
                 {"if":{"properties":{"format":{"const":"instruction_scope"}},"required":["format"]},
                  "then":{"not":{"anyOf":[{"required":["offset"]},{"required":["limit"]},{"required":["expected_sha256"]},{"required":["start_line"]},{"required":["line_count"]}]}},
                  "else":{"properties":{"recursive":{"const":false}}}}]
    })
}

fn optional_path_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": workspace_path_value()
        },
        "required": []
    })
}

fn search_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {
                "type": "string",
                "minLength": 1,
                "maxLength": 1024,
                "pattern": "\\S"
            },
            "path": {
                "type": "string",
                "minLength": 1,
                "maxLength": 4096,
                "description": "Accepts a workspace-relative file or directory. If the user limits search to one named file, copy that file's full relative path exactly; do not substitute its parent directory or expand the search scope."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 200,
                "default": 100
            }
        },
        "required": ["query"]
    })
}

fn write_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": workspace_path_value(),
            "content": {
                "type": "string",
                "maxLength": 8_388_608,
                "description":"Exact complete UTF-8 text; no trailing newline is added. For two lines with a requested final LF use {\"content\":\"first\\nsecond\\n\"}. Include the final LF in content, not just between lines. Host also enforces an 8 MiB byte limit. Prefer apply_patch for focused edits."
            }
        },
        "required": ["path", "content"]
    })
}

fn patch_schema() -> Value {
    let line = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": {"type":"string", "enum":["context", "add", "remove"]},
            "text": {"type": "string", "maxLength": 1_048_576, "pattern":"^[^\\r\\n\\u0000]*$"}
        },
        "required": ["kind", "text"]
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "files": {
                "type": "array",
                "minItems": 1,
                "maxItems": 64,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": workspace_path_value(),
                        "expected_source": {
                            "description":"Bind to the full sha256 from read_file, or to observed absence. Omission/any uses legacy line-only matching. A conflict requires re-reading and replanning, not removing the guard.",
                            "oneOf":[
                                {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"any"}},"required":["kind"]},
                                {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"absent"}},"required":["kind"]},
                                {"type":"object","additionalProperties":false,"properties":{"kind":{"const":"existing"},"sha256":{"type":"string","pattern":"^[0-9a-f]{64}$","description":"Copy the entire sha256 from the matching read_file result verbatim, all 64 lowercase hex characters. Do not calculate it from remembered text, guess, shorten or substitute another file's digest. On conflict re-read and replan; keep the source guard."}},"required":["kind","sha256"]}
                            ]
                        },
                        "hunks": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": 256,
                            "items": {
                                "type": "object",
                                "description":"For replacement, lines must contain remove(old) then add(new). context is unchanged text and counts in both old_lines and new_lines; it does not replace itself. Schema keywords such as minItems/maxItems are not arguments.",
                                "additionalProperties": false,
                                "properties": {
                                    "old_start": {"type": "integer", "minimum": 0, "maximum":131072},
                                    "old_lines": {"type": "integer", "minimum": 0, "maximum":131072},
                                    "new_start": {"type": "integer", "minimum": 0, "maximum":131072},
                                    "new_lines": {"type": "integer", "minimum": 0, "maximum":131072},
                                    "lines": {
                                        "type": "array",
                                        "minItems":1,
                                        "maxItems": 16384,
                                        "items": line
                                    }
                                },
                                "required": [
                                    "old_start",
                                    "old_lines",
                                    "new_start",
                                    "new_lines",
                                    "lines"
                                ]
                            }
                        }
                    },
                    "required": ["path", "hunks"]
                }
            }
        },
        "required": ["files"]
    })
}

fn process_launch(include_wait: bool) -> Value {
    let cmd_description = if cfg!(target_os = "windows") {
        "Runs script text directly through Windows PowerShell 5.1. PowerShell cmdlets such as Copy-Item, Move-Item, Remove-Item and New-Item are shell operations, not executables: use cmd, not command=Copy-Item or tokenized powershell.exe -Command arguments. For example {\"cmd\":\"Copy-Item -LiteralPath 'a path/source.txt' -Destination 'a path/copy.txt' -ErrorAction Stop\"}. Keep each quoted literal path in this script and preserve errors. Do not add mkdir when the accepted task says the directory already exists. Use exact workspace-relative paths without a project-name prefix. Do not prefix cmd with powershell.exe or pwsh. Do not assume PowerShell 7-only syntax such as ??, ??=, ?:, &&, or ||. Command Prompt syntax such as dir /b requires command=cmd.exe with args beginning [\"/d\",\"/c\"]. Ordinary executables such as bun/git use command plus args. Never combine the forms."
    } else {
        "Use cmd only when shell semantics such as pipelines, redirection, globbing, compound syntax, or a shell script are required. Runs through /bin/sh -c on this process host. For an ordinary single executable use command plus args. Never combine the forms."
    };
    let executable_examples = if cfg!(target_os = "windows") {
        "git, bun, powershell.exe, or cmd.exe"
    } else {
        "git, bun, or /bin/ls"
    };
    let argv_examples = if cfg!(target_os = "windows") {
        "[\"status\",\"--short\"] for git, or [\"/d\",\"/c\",\"echo ready\"] for command=cmd.exe"
    } else {
        "[\"status\",\"--short\"] for git, or [\"-a\"] for /bin/ls"
    };
    let mut properties = json!({
        "cmd":{"type":"string","minLength":1,"maxLength":32768,"description":format!("Shell-script form only: supply {{\"cmd\":\"script text\"}} and omit both command and args. Never use cmd for an executable plus an args array. {cmd_description}")},
        "command":{"type":"string","minLength":1,"maxLength":32768,"description":format!("Executable form: supply {{\"command\":\"program\",\"args\":[...]}} and omit cmd. This field is the executable name or path only, for example {executable_examples}. Never include arguments such as git status in this field.")},
        "args":{"type":"array","maxItems":256,"items":{"type":"string","maxLength":65536},"description":format!("Valid only with command, never with cmd. Literal separate argument tokens as an actual JSON array value, for example {argv_examples}; never a JSON-encoded string such as \"[\\\"status\\\",\\\"--short\\\"]\".")},
        "cwd":{"type":"string","maxLength":4096,"description":"Normalized workspace-relative directory only, for example src or tests. Omit cwd or use '.' for the bound workspace root. Never pass an absolute OS path, a drive prefix, backslashes, or '..'. Command arguments may contain the literal path format required by the executable."},
        "env":{"type":"object","maxProperties":128,"additionalProperties":{"type":"string","maxLength":65536}},
        "timeout_ms":{"type":"integer","minimum":1,"maximum":600000,"description":"Total owned process lifetime in milliseconds, starting at launch. Default 30000; polling does not reset it. Choose an explicit duration when the accepted task requires longer interaction or waiting. The owner terminates and cleans up on expiration; timeout is distinct from user cancellation."},
        "tty":{"type":"boolean","default":false,"description":"false (or omitted) uses pipe; true uses PTY. For a requested pipe stdin/EOF lifecycle use start_process with tty=false and wait_ms=0. PTY is not a substitute for pipe close_stdin, and exec_command still waits for exit regardless of tty."},
        "cols":{"type":"integer","minimum":1,"maximum":32767},
        "rows":{"type":"integer","minimum":1,"maximum":32767}
    });
    if cfg!(target_os = "windows") {
        for name in ["command", "args"] {
            let original = properties[name]["description"].as_str().expect("standard process description");
            properties[name]["description"] = json!(format!("{original} PowerShell cmdlets such as Copy-Item and Move-Item need cmd script text, not a literal executable or tokenized -Command arguments; see cmd's quoted-path example."));
        }
    }
    if include_wait {
        properties["wait_ms"] = json!({"type":"integer","minimum":0,"maximum":30000,"default":0});
    }
    json!({"type":"object","description":"Choose exactly one form: command plus an args array for an executable, or cmd alone for shell script text. cmd plus args is invalid.","additionalProperties":false,"properties":properties,
        "oneOf":[{"required":["cmd"],"not":{"required":["command"]},"properties":{"args":{"maxItems":0}}},
                 {"required":["command"],"not":{"required":["cmd"]}}]})
}

fn process_launch_schema() -> Value {
    process_launch(false)
}

fn process_start_schema() -> Value {
    let mut schema = process_launch(true);
    schema["properties"]["wait_ms"]["maximum"] = json!(0);
    schema["properties"]["wait_ms"]["description"] = json!(
        "Must be 0. Starting only establishes the turn-owned handle; use poll_process with its exact process_id for every output wait."
    );
    schema
}

fn process_poll_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "process_id":{"type":"string","minLength":1,"maxLength":128},
        "cursor":{"type":"integer","minimum":0,"default":0,"description":"For a following poll, pass the previous output.next_cursor. Default 0 deliberately replays retained output; it does not resume at the last observation."},
        "wait_ms":{"type":"integer","minimum":0,"maximum":30000,"default":0,"description":"Per-poll wait in milliseconds, from 0 through 30000 inclusive. This is separate from the process lifetime timeout_ms: never copy a launch timeout such as 120000 here. Use 0 for an immediate observation, 1000 for a short wait, or 30000 for the longest allowed poll. Wait up to this duration for new output or terminal state. Retained output after cursor returns immediately. Advance cursor before waiting again; 0 only observes current state."}
    },"required":["process_id"]})
}

fn process_input_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "process_id":{"type":"string","minLength":1,"maxLength":128},
        "input":{"type":"string","maxLength":1048576,
            "description":"Exact UTF-8 text to write without trimming or normalization. If input already contains the requested final LF, omit append_newline or set it false."},
        "append_newline":{"type":"boolean","default":false,
            "description":"When true, always append one LF byte (0x0A), even if input already ends in LF. To send exactly one final LF use {\"input\":\"hello\",\"append_newline\":true} or {\"input\":\"hello\\n\",\"append_newline\":false}. Including LF in input with true sends two LFs; use that only when two are requested."}
    },"required":["process_id","input"]})
}

fn process_id_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "process_id":{"type":"string","minLength":1,"maxLength":128}
    },"required":["process_id"]})
}

fn process_resize_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "process_id":{"type":"string","minLength":1,"maxLength":128},
        "cols":{"type":"integer","minimum":1,"maximum":32767},
        "rows":{"type":"integer","minimum":1,"maximum":32767}
    },"required":["process_id","cols","rows"]})
}

fn artifact_read_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "artifact_id":{"type":"string","pattern":"^[0-9a-f]{64}$"},
        "offset":{"type":"integer","minimum":0,"maximum":536870912,"default":0},
        "limit":{"type":"integer","minimum":1,"maximum":1048576,"default":16384}
    },"required":["artifact_id"]})
}

fn artifact_publish_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "path":workspace_path_value(),
        "expected_sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"}
    },"required":["path"]})
}

fn commit_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": {
                "type": "string",
                "minLength": 1,
                "maxLength": 512,
                "pattern":"\\S"
            }
        },
        "required": ["message"]
    })
}

fn push_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "remote": {
                "type": "string",
                "minLength": 1,
                "maxLength": 256
            },
            "refspec": {
                "type": "string",
                "minLength": 1,
                "maxLength": 1024,
                "pattern": "^(HEAD|refs/heads/.+):refs/heads/.+$"
            },
            "force": {
                "const": false,
                "default": false
            }
        },
        "required": ["remote", "refspec"]
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn selected_workspace_paths_do_not_invent_a_project_name_prefix() {
        for name in ["read_file","search_files","write_file","apply_patch","delete_path","exec_command","start_process"] {
            let definitions = super::standard_agent_tool_exposures();
            let description = &definitions.iter().find(|tool|tool.definition.name==name).unwrap().definition.description;
            assert!(description.contains("already the selected project directory"),"{name}: root identity missing");
            assert!(description.contains("Do not prepend a project or conversation display name"),"{name}: invented path prefix not addressed");
            assert!(description.contains("Keep the exact workspace-relative paths supplied by the user"),"{name}: accepted paths missing");
        }
    }
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    #[test]
    fn process_examples_prefer_valid_json_argv_without_foreign_host_commands() {
        let exec = standard_agent_tool_exposures().into_iter()
            .find(|tool| tool.definition.name == "exec_command").unwrap();
        assert!(exec.definition.description.contains(r#"{"command":"git","args":["status","--short"]}"#));
        if cfg!(target_os = "macos") {
            assert!(!exec.definition.description.contains("cmd.exe"));
            assert!(exec.definition.description.contains(r#"{"command":"/bin/ls","args":["-a"]}"#));
            assert!(exec.definition.description.contains(r#"{"command":"/bin/pwd","args":["-P"]}"#));
        }
        assert!(exec.definition.description.contains("never gets silently split"));
    }

    #[test]
    fn read_formats_accept_neutral_defaults_but_reject_incompatible_options() {
        let schema = read_schema();
        let validator = jsonschema::validator_for(&schema).unwrap();
        for input in [
            json!({"path":".","format":"instruction_scope","recursive":false,"missing_ok":false}),
            json!({"path":"file.txt","recursive":false,"start_line":2,"line_count":10}),
            json!({"path":"image.png","format":"image","recursive":false,"missing_ok":false}),
        ] { assert!(validator.is_valid(&input), "{input}"); }
        for input in [
            json!({"path":".","format":"instruction_scope","missing_ok":true}),
            json!({"path":"file.txt","recursive":true}),
            json!({"path":"image.png","format":"image","start_line":1}),
            json!({"path":"file.txt","start_line":1,"offset":0}),
            json!({"path":"   "}),
        ] { assert!(!validator.is_valid(&input), "{input}"); }
    }

    #[test]
    fn workspace_model_schema_rejects_push_widening_and_matches_artifact_default() {
        let schema = push_schema();
        let push = jsonschema::validator_for(&schema).unwrap();
        assert!(push.is_valid(&json!({
            "remote":"origin", "refspec":"HEAD:refs/heads/main", "force":false
        })));
        for input in [
            json!({"remote":"origin","refspec":"main"}),
            json!({"remote":"origin","refspec":"HEAD:refs/heads/main","force":true}),
        ] {
            assert!(!push.is_valid(&input), "{input}");
        }
        assert_eq!(
            artifact_read_schema()["properties"]["limit"]["default"],
            16384
        );
    }

    #[test]
    fn read_tool_explains_whole_source_digest_for_partial_pages() {
        let read = standard_agent_tool_exposures()
            .into_iter()
            .find(|tool| tool.definition.name == "read_file")
            .unwrap();
        assert!(
            read.definition
                .description
                .contains("sha256 always identifies the entire source")
        );
        assert!(read.definition.input_schema.0["properties"]["expected_sha256"]
            ["description"]
            .as_str()
            .is_some_and(|description| description.contains("whole-source SHA-256")));
        assert!(read
            .definition
            .description
            .contains("NEVER include missing_ok"));
    }

    #[test]
    fn read_tool_explains_instruction_discovery_before_source_reads() {
        let read = standard_agent_tool_exposures()
            .into_iter()
            .find(|tool| tool.definition.name == "read_file")
            .unwrap();
        let description = &read.definition.description;
        assert!(description.contains("never use the default text format for AGENTS.md"));
        assert!(description.contains("call read_file alone on its directory"));
        assert!(description.contains("format=instruction_scope"));
        assert!(description.contains("source reads with fresh call IDs"));
    }

    #[test]
    fn patch_tool_explains_exact_replacement_shape() {
        let patch = standard_agent_tool_exposures()
            .into_iter()
            .find(|tool| tool.definition.name == "apply_patch")
            .unwrap();
        assert!(patch.definition.description.contains("lines=[{kind:remove,text:old},{kind:add,text:new}]"));
        let hunk = &patch.definition.input_schema.0["properties"]["files"]["items"]
            ["properties"]["hunks"]["items"];
        assert!(hunk["description"].as_str().is_some_and(|description|
            description.contains("context is unchanged text")
                && description.contains("minItems/maxItems are not arguments")));
    }

    #[test]
    fn process_cmd_schema_names_the_actual_host_shell() {
        let tools = standard_agent_tool_exposures();
        let process = tools
            .iter()
            .find(|tool| tool.definition.name == "exec_command")
            .unwrap();
        let start = tools
            .iter()
            .find(|tool| tool.definition.name == "start_process")
            .unwrap();
        let expected_prefix = format!("Workspace process host OS: {}.", std::env::consts::OS);
        assert!(process.definition.description.starts_with(&expected_prefix));
        assert!(start.definition.description.starts_with(&expected_prefix));
        assert!(!process.definition.description.contains("UI client host OS"));
        let description = process.definition.input_schema.0["properties"]["cmd"]["description"]
            .as_str()
            .unwrap();
        if cfg!(target_os = "windows") {
            assert!(!process.definition.description.contains("cmd=ls -la"));
            assert!(
                process
                    .definition
                    .description
                    .contains("Windows PowerShell 5.1")
            );
            assert!(description.contains("PowerShell"));
            assert!(description.contains("Windows PowerShell 5.1"));
            assert!(description.contains("Do not prefix cmd with powershell.exe"));
            assert!(description.contains("PowerShell 7-only syntax such as ??"));
            assert!(description.contains("dir /b"));
            assert!(description.contains("command=cmd.exe"));
        } else {
            assert!(description.contains("/bin/sh -c"));
        }
    }

    #[test]
    fn process_tool_prefers_literal_argv_for_single_executables() {
        let process = standard_agent_tool_exposures()
            .into_iter()
            .find(|tool| tool.definition.name == "exec_command")
            .unwrap();
        assert!(
            process
                .definition
                .description
                .contains(r#"{"command":"program","args":["literal","tokens"]}"#)
        );
        assert!(process.definition.description.contains(r#"{"cmd":"script text"} with no args field"#));
        assert!(process.definition.input_schema.0["description"]
            .as_str().is_some_and(|description| description.contains("cmd plus args is invalid")));
        let properties = &process.definition.input_schema.0["properties"];
        assert!(properties["cmd"]["description"]
            .as_str().is_some_and(|description| description.contains("omit both command and args")
                && description.contains("Never use cmd for an executable plus an args array")));
        assert!(properties["command"]["description"]
            .as_str().is_some_and(|description| description.contains("omit cmd")));
        assert!(properties["args"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("Valid only with command, never with cmd")
                && description.contains("actual JSON array value")));
        for field in ["command", "args"] {
            let description = properties[field]["description"].as_str().unwrap();
            assert_eq!(description.contains("cmd.exe"), cfg!(target_os = "windows"));
            assert_eq!(description.contains("/bin/ls"), !cfg!(target_os = "windows"));
        }
    }

    #[test]
    fn text_tool_guidance_distinguishes_exact_content_from_appended_newlines() {
        let tools = standard_agent_tool_exposures();
        let schema = |name: &str| &tools.iter().find(|tool| tool.definition.name == name)
            .unwrap().definition.input_schema.0;
        let content = schema("write_file")["properties"]["content"]["description"].as_str().unwrap_or_default();
        assert!(content.contains("no trailing newline is added"));
        assert!(content.contains(r#"{"content":"first\nsecond\n"}"#));
        let append = schema("write_process_stdin")["properties"]["append_newline"]["description"].as_str().unwrap();
        assert!(append.contains("even if input already ends in LF"));
        assert!(append.contains(r#"{"input":"hello","append_newline":true}"#));
        assert!(append.contains(r#"{"input":"hello\n","append_newline":false}"#));
        let validator = jsonschema::validator_for(schema("write_process_stdin")).unwrap();
        // These remain distinct, legal byte requests; guidance must not turn
        // them into trimming, deduplication or a new admission restriction.
        for args in [
            json!({"process_id":"owned","input":"你好 MAC-B","append_newline":true}),
            json!({"process_id":"owned","input":"你好 MAC-B\n","append_newline":false}),
            json!({"process_id":"owned","input":"你好 MAC-B\n","append_newline":true}),
        ] { assert!(validator.is_valid(&args), "{args}"); }
    }

    #[test]
    fn process_lifecycle_tools_keep_one_turn_owned_handle() {
        let tools = standard_agent_tool_exposures();
        let description = |name: &str| {
            tools
                .iter()
                .find(|tool| tool.definition.name == name)
                .unwrap()
                .definition
                .description
                .as_str()
        };
        assert!(description("start_process").contains("same Turn"));
        assert!(description("exec_command").contains("waits for exit/timeout"));
        assert!(description("exec_command").contains("does not return a live handle"));
        assert!(description("start_process").contains("wait_ms=0,tty=false (pipe)"));
        assert!(description("start_process").contains("close_process_stdin"));
        let tty=&tools.iter().find(|tool|tool.definition.name=="start_process").unwrap().definition.input_schema.0["properties"]["tty"];
        assert_eq!(tty["default"],false);
        assert!(tty["description"].as_str().unwrap().contains("exec_command still waits"));
        assert!(description("start_process").contains("Never use shell backgrounding"));
        assert!(description("start_process").contains("Do not pre-read"));
        assert!(description("start_process").contains("Do not run ls"));
        assert!(description("poll_process").contains("expected output"));
        assert!(description("poll_process").contains("not a tool failure"));
        assert!(description("write_process_stdin").contains("append_newline=true"));
        assert!(
            tools
                .iter()
                .find(|tool| tool.definition.name == "write_process_stdin")
                .unwrap()
                .definition
                .input_schema
                .0["properties"]["input"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("without trimming or normalization"))
        );
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.definition.name == "write_process_stdin")
                .unwrap()
                .definition
                .input_schema
                .0["properties"]["append_newline"]["default"],
            false
        );
        assert!(description("cancel_process").contains("cleanup.reaped=true"));
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.definition.name == "start_process")
                .unwrap()
                .definition
                .input_schema
                .0["properties"]["wait_ms"]["maximum"],
            0
        );
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.definition.name == "resize_process")
                .unwrap()
                .definition
                .input_schema
                .0["properties"]["cols"]["maximum"],
            32767
        );
    }

    #[test]
    fn tool_names_are_unique_before_exact_snapshot_filtering() {
        let tools = standard_agent_tool_exposures();
        let names = tools
            .iter()
            .map(|tool| tool.definition.name.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), tools.len());
    }

    fn assert_model_schema_is_canonical_subset(
        model: &serde_json::Value,
        canonical: &serde_json::Value,
        path: &str,
    ) {
        match (model, canonical) {
            (serde_json::Value::Object(model), serde_json::Value::Object(canonical)) => {
                let model = model
                    .iter()
                    .filter(|(key, _)| key.as_str() != "description")
                    .collect::<BTreeMap<_, _>>();
                let canonical = canonical
                    .iter()
                    .filter(|(key, _)| key.as_str() != "description")
                    .collect::<BTreeMap<_, _>>();
                let missing = canonical
                    .keys()
                    .filter(|key| !model.contains_key(*key))
                    .map(|key| key.as_str())
                    .collect::<BTreeSet<_>>();
                assert!(missing.is_empty(), "model schema omitted {missing:?} at {path}");
                for (key, model_value) in model {
                    let child = format!("{path}/{key}");
                    let Some(canonical_value) = canonical.get(key).copied() else {
                        assert!(
                            matches!(
                                key.as_str(),
                                "minimum"
                                    | "minItems"
                                    | "minLength"
                                    | "minProperties"
                                    | "maximum"
                                    | "maxItems"
                                    | "maxLength"
                                    | "maxProperties"
                                    | "pattern"
                                    | "const"
                                    | "enum"
                            ),
                            "model schema added a non-narrowing keyword at {child}"
                        );
                        continue;
                    };
                    match key.as_str() {
                        "maximum" | "maxItems" | "maxLength" | "maxProperties" => {
                            assert!(
                                model_value.as_u64().unwrap() <= canonical_value.as_u64().unwrap(),
                                "model schema widened {child}"
                            );
                        }
                        "minimum" | "minItems" | "minLength" | "minProperties" => {
                            assert!(
                                model_value.as_u64().unwrap() >= canonical_value.as_u64().unwrap(),
                                "model schema widened {child}"
                            );
                        }
                        "required" => {
                            let model = model_value
                                .as_array()
                                .unwrap()
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<BTreeSet<_>>();
                            let canonical = canonical_value
                                .as_array()
                                .unwrap()
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .collect::<BTreeSet<_>>();
                            assert!(
                                canonical.is_subset(&model),
                                "model schema omitted canonical required fields at {child}"
                            );
                        }
                        _ => assert_model_schema_is_canonical_subset(
                            model_value,
                            canonical_value,
                            &child,
                        ),
                    }
                }
            }
            (serde_json::Value::Array(model), serde_json::Value::Array(canonical)) => {
                assert_eq!(model.len(), canonical.len(), "schema array drifted at {path}");
                for (index, (model, canonical)) in
                    model.iter().zip(canonical.iter()).enumerate()
                {
                    assert_model_schema_is_canonical_subset(
                        model,
                        canonical,
                        &format!("{path}/{index}"),
                    );
                }
            }
            _ => assert_eq!(model, canonical, "schema value drifted at {path}"),
        }
    }

    #[test]
    fn every_model_workspace_schema_is_an_admission_subset_of_canonical_wave2() {
        let registration = nomifun_agent_domain_wave2::workspace_execution_registration().unwrap();
        let capabilities = registration
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities
            .into_iter()
            .map(|capability| (capability.id.clone(), capability))
            .collect::<BTreeMap<_, _>>();
        let workspace_tools = standard_agent_tool_exposures();
        assert_eq!(workspace_tools.len(), 19);
        for tool in workspace_tools {
            let capability = &capabilities[&tool.capability_id];
            let reference = capability
                .contributions
                .actions
                .iter()
                .find(|action| action.action_id == tool.action_id)
                .unwrap()
                .input_schema
                .clone();
            let canonical = nomifun_agent_domain_wave2::resolve_action_schema(
                tool.capability_id.as_ref(),
                &reference,
            )
            .unwrap();
            assert_model_schema_is_canonical_subset(
                &tool.definition.input_schema.0,
                &canonical.0,
                tool.action_id.as_ref(),
            );
        }
    }

    #[test]
    fn full_surface_maps_only_to_canonical_capability_actions() {
        let mut actions = BTreeSet::new();
        for exposure in standard_agent_tool_exposures() {
            assert!(exposure.action_id.as_ref().starts_with(&format!(
                "{}/",
                exposure.capability_id.as_ref()
            )));
            assert!(actions.insert(exposure.action_id.clone()));
            assert_eq!(exposure.definition.input_schema.0["type"], "object");
            assert_eq!(
                exposure.definition.input_schema.0["additionalProperties"],
                false
            );
        }
        let expected = BTreeSet::from([
            "workspace.files/read",
            "workspace.files/search",
            "workspace.files/write",
            "workspace.files/patch",
            "workspace.files/delete",
            "workspace.vcs/status",
            "workspace.vcs/diff",
            "workspace.vcs/stage",
            "workspace.vcs/commit",
            "workspace.vcs/push",
            "workspace.process/exec",
            "workspace.process/start",
            "workspace.process/poll",
            "workspace.process/input",
            "workspace.process/close_stdin",
            "workspace.process/resize",
            "workspace.process/cancel",
            "workspace.artifacts/read",
            "workspace.artifacts/publish",
        ]);
        assert_eq!(
            actions
                .iter()
                .map(|action| action.as_ref())
                .collect::<BTreeSet<_>>(),
            expected
        );
        assert_eq!(actions.len(), 19);
    }
}
