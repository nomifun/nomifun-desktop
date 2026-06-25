//! Minimal Language Server Protocol client for the agent's code-navigation tool
//! (design §3.3 "LSP 工具": goToDefinition / findReferences / documentSymbol /
//! hover). Hand-rolled (no new crate dep, keeping the agent layer dependency-
//! light) and deliberately small: only the handful of methods the tool needs.
//!
//! # Status: experimental, opt-in, default OFF
//!
//! Registered only when `tools.lsp_servers` maps a file extension to a server
//! command, so existing behaviour is unchanged. The two bug-prone, spec-exact
//! pieces — Content-Length framing ([`codec`]) and UTF-16 position conversion
//! ([`position`]) — are unit-tested here. The live server handshake / request
//! path in [`client`] cannot be exercised without a real language server in the
//! environment; treat it as experimental until validated against one.

pub mod codec {
    //! `Content-Length`-framed JSON-RPC message framing (LSP base protocol).

    /// Encode a JSON payload as an LSP base-protocol message:
    /// `Content-Length: N\r\n\r\n<json>`. The length is the payload's **byte**
    /// length, not its char length.
    pub fn encode_message(json: &str) -> Vec<u8> {
        let mut out = format!("Content-Length: {}\r\n\r\n", json.len()).into_bytes();
        out.extend_from_slice(json.as_bytes());
        out
    }

    /// Try to split one complete framed message off the front of `buf`. On
    /// success the consumed bytes (header + body) are drained from `buf` and the
    /// JSON body is returned. Returns `None` when `buf` does not yet hold a full
    /// message (caller should read more bytes and retry). Malformed headers
    /// (missing/invalid Content-Length) drain the bad header and return `None`
    /// so the stream can resynchronise rather than wedge.
    pub fn try_decode(buf: &mut Vec<u8>) -> Option<String> {
        // Find the header/body separator.
        let sep = find_subsequence(buf, b"\r\n\r\n")?;
        let header = &buf[..sep];
        let header_str = String::from_utf8_lossy(header);
        let content_len = header_str
            .lines()
            .find_map(|line| {
                let (k, v) = line.split_once(':')?;
                if k.trim().eq_ignore_ascii_case("Content-Length") {
                    v.trim().parse::<usize>().ok()
                } else {
                    None
                }
            });
        let body_start = sep + 4;
        let Some(content_len) = content_len else {
            // Bad header: drop it so a later valid frame can be found.
            buf.drain(..body_start);
            return None;
        };
        if buf.len() < body_start + content_len {
            return None; // body not fully arrived yet
        }
        let body = buf[body_start..body_start + content_len].to_vec();
        buf.drain(..body_start + content_len);
        Some(String::from_utf8_lossy(&body).into_owned())
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn encode_uses_byte_length_and_crlf_framing() {
            // "héllo" is 6 bytes (é = 2 bytes), 5 chars — length must be 6.
            let framed = encode_message("héllo");
            let s = String::from_utf8_lossy(&framed);
            assert!(s.starts_with("Content-Length: 6\r\n\r\n"), "got: {s:?}");
            assert!(s.ends_with("héllo"));
        }

        #[test]
        fn decode_roundtrips_a_single_message() {
            let mut buf = encode_message("{\"jsonrpc\":\"2.0\"}");
            let body = try_decode(&mut buf).unwrap();
            assert_eq!(body, "{\"jsonrpc\":\"2.0\"}");
            assert!(buf.is_empty(), "consumed bytes must be drained");
        }

        #[test]
        fn decode_handles_two_concatenated_messages() {
            let mut buf = encode_message("AAA");
            buf.extend(encode_message("BB"));
            assert_eq!(try_decode(&mut buf).unwrap(), "AAA");
            assert_eq!(try_decode(&mut buf).unwrap(), "BB");
            assert!(try_decode(&mut buf).is_none());
        }

        #[test]
        fn decode_waits_for_incomplete_body() {
            let full = encode_message("HELLO");
            // Feed everything except the last byte.
            let mut buf = full[..full.len() - 1].to_vec();
            assert!(try_decode(&mut buf).is_none(), "must wait for the full body");
            buf.push(full[full.len() - 1]);
            assert_eq!(try_decode(&mut buf).unwrap(), "HELLO");
        }

        #[test]
        fn decode_skips_a_malformed_header_to_resync() {
            // A header with no Content-Length is dropped; the following valid
            // frame still decodes.
            let mut buf = b"Garbage: 1\r\n\r\n".to_vec();
            buf.extend(encode_message("OK"));
            assert!(try_decode(&mut buf).is_none()); // drops the bad header
            assert_eq!(try_decode(&mut buf).unwrap(), "OK");
        }
    }
}

pub mod position {
    //! Conversion between editor-style 1-based char columns and LSP's 0-based
    //! UTF-16 code-unit positions. LSP `Position.character` counts UTF-16 code
    //! units by default — getting this wrong silently mis-targets every request
    //! on any line containing non-BMP characters (emoji, some CJK), so it is
    //! tested explicitly.

    /// Convert a 1-based character column (counting Unicode scalar values, the
    /// usual editor convention) on `line_text` into a 0-based UTF-16 code-unit
    /// offset for LSP. A column past the end clamps to the line's UTF-16 length.
    pub fn char_col_to_utf16(line_text: &str, char_col_1based: usize) -> u32 {
        let take = char_col_1based.saturating_sub(1);
        line_text
            .chars()
            .take(take)
            .map(|c| c.len_utf16() as u32)
            .sum()
    }

    /// Convert a 0-based UTF-16 offset (as returned by a server) back to a
    /// 1-based character column for display. An offset past the end clamps to
    /// the line's char length + 1.
    pub fn utf16_to_char_col(line_text: &str, utf16_offset: u32) -> usize {
        let mut remaining = utf16_offset;
        let mut chars = 0usize;
        for c in line_text.chars() {
            let w = c.len_utf16() as u32;
            if remaining < w {
                break;
            }
            remaining -= w;
            chars += 1;
        }
        chars + 1
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn ascii_columns_are_one_to_one() {
            assert_eq!(char_col_to_utf16("hello", 1), 0);
            assert_eq!(char_col_to_utf16("hello", 3), 2);
            assert_eq!(char_col_to_utf16("hello", 6), 5);
        }

        #[test]
        fn bmp_chars_are_one_utf16_unit_each() {
            // CJK characters are single UTF-16 units.
            assert_eq!(char_col_to_utf16("你好world", 3), 2); // after 你好
        }

        #[test]
        fn non_bmp_chars_are_two_utf16_units() {
            // "a😀b": 😀 (U+1F600) is a surrogate pair = 2 UTF-16 units.
            // Column 3 (1-based) = after "a😀" = 1 + 2 = 3 UTF-16 units.
            assert_eq!(char_col_to_utf16("a😀b", 3), 3);
            assert_eq!(char_col_to_utf16("a😀b", 2), 1); // after "a"
        }

        #[test]
        fn utf16_to_char_col_inverts_the_conversion() {
            let line = "a😀b€c"; // €=1 unit, 😀=2 units
            for col in 1..=6 {
                let u16 = char_col_to_utf16(line, col);
                assert_eq!(utf16_to_char_col(line, u16), col, "col {col} round-trips");
            }
        }
    }
}

pub mod client {
    //! A session-cached LSP client: one server process per (command, root),
    //! reused across tool calls so the server is indexed once and stays warm.
    //! Requests are serialized by the caller (the tool holds the client behind a
    //! mutex), so a simple send-then-read-until-matching-id loop is correct
    //! without a concurrent dispatcher.
    //!
    //! EXPERIMENTAL: the live handshake/request path is not exercisable without a
    //! real language server in the environment. Server→client requests (e.g.
    //! `workspace/configuration`) are currently ignored and rely on the overall
    //! timeout; a server that blocks on them will time out rather than hang.

    use std::path::Path;
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::process::{Child, ChildStdin, ChildStdout};

    use super::codec;

    /// Overall deadline for any single request (covers cold-start indexing).
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

    pub struct LspClient {
        child: Child,
        stdin: ChildStdin,
        stdout: ChildStdout,
        buf: Vec<u8>,
        next_id: i64,
    }

    impl LspClient {
        /// Spawn `command` (program + args) rooted at `root` and complete the
        /// `initialize` / `initialized` handshake.
        pub async fn start(command: &[String], root: &Path) -> Result<Self, String> {
            let (program, args) = command
                .split_first()
                .ok_or_else(|| "empty LSP server command".to_string())?;
            let mut child = tokio::process::Command::new(program)
                .args(args)
                .current_dir(root)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("failed to spawn LSP server '{program}': {e}"))?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let mut c = Self {
                child,
                stdin,
                stdout,
                buf: Vec::new(),
                next_id: 0,
            };
            let root_uri = path_to_uri(root);
            c.request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "rootUri": root_uri,
                    "capabilities": {
                        "textDocument": {
                            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                            "definition": {}, "references": {}, "hover": {}
                        }
                    }
                }),
            )
            .await?;
            c.notify("initialized", json!({})).await?;
            Ok(c)
        }

        /// Send `textDocument/didOpen` so the server has the file contents.
        pub async fn did_open(&mut self, uri: &str, language_id: &str, text: &str) -> Result<(), String> {
            self.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": { "uri": uri, "languageId": language_id, "version": 1, "text": text }
                }),
            )
            .await
        }

        /// Send a request and return its `result` (or an `Err` carrying the
        /// server's error message).
        pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
            self.next_id += 1;
            let id = self.next_id;
            let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
            self.write(&msg).await?;
            self.read_until_id(id).await
        }

        pub async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
            let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
            self.write(&msg).await
        }

        async fn write(&mut self, msg: &Value) -> Result<(), String> {
            let framed = codec::encode_message(&msg.to_string());
            self.stdin
                .write_all(&framed)
                .await
                .map_err(|e| format!("LSP write failed: {e}"))?;
            self.stdin.flush().await.map_err(|e| format!("LSP flush failed: {e}"))
        }

        /// Read framed messages until the response with `id` arrives, skipping
        /// notifications and unrelated messages. Bounded by `REQUEST_TIMEOUT`.
        async fn read_until_id(&mut self, id: i64) -> Result<Value, String> {
            let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
            loop {
                // Drain any already-buffered complete frames first.
                while let Some(body) = codec::try_decode(&mut self.buf) {
                    let v: Value = serde_json::from_str(&body)
                        .map_err(|e| format!("LSP response parse error: {e}"))?;
                    if v.get("id").and_then(|i| i.as_i64()) == Some(id) {
                        if let Some(err) = v.get("error") {
                            return Err(format!("LSP server error: {err}"));
                        }
                        return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                    }
                    // Otherwise: a notification or a server→client request we
                    // don't handle — ignore and keep reading.
                }
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(format!("LSP request '{id}' timed out"));
                }
                let mut chunk = [0u8; 8192];
                let n = match tokio::time::timeout(remaining, self.stdout.read(&mut chunk)).await {
                    Ok(Ok(0)) => return Err("LSP server closed the connection".to_string()),
                    Ok(Ok(n)) => n,
                    Ok(Err(e)) => return Err(format!("LSP read failed: {e}")),
                    Err(_) => return Err(format!("LSP request '{id}' timed out")),
                };
                self.buf.extend_from_slice(&chunk[..n]);
            }
        }
    }

    impl Drop for LspClient {
        fn drop(&mut self) {
            // Best-effort: kill the server when the session is dropped.
            let _ = self.child.start_kill();
        }
    }

    /// Convert an absolute filesystem path to a `file://` URI (minimal, not a
    /// full RFC 3986 encoder — adequate for local paths).
    pub fn path_to_uri(path: &Path) -> String {
        let p = path.to_string_lossy().replace('\\', "/");
        if p.starts_with('/') {
            format!("file://{p}")
        } else {
            format!("file:///{p}")
        }
    }
}

pub mod tool {
    //! `Lsp` tool: code navigation via a configured language server. Registered
    //! only when `tools.lsp_servers` is non-empty (default off → no behaviour
    //! change). EXPERIMENTAL — see the module header.

    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::{Value, json};
    use tokio::sync::Mutex;

    use nomi_protocol::events::ToolCategory;
    use nomi_types::tool::{JsonSchema, ToolResult};

    use super::client::{LspClient, path_to_uri};
    use super::position::char_col_to_utf16;
    use crate::Tool;

    pub struct LspTool {
        /// Maps a file extension (without dot, lowercase) to the server command.
        servers: HashMap<String, Vec<String>>,
        cwd: PathBuf,
        /// One live server per distinct command, reused across calls.
        sessions: Arc<Mutex<HashMap<String, Arc<Mutex<LspClient>>>>>,
    }

    impl LspTool {
        pub fn new(servers: HashMap<String, Vec<String>>, cwd: PathBuf) -> Self {
            Self {
                servers,
                cwd,
                sessions: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        /// Whether any server is configured (the bootstrap gate).
        pub fn has_servers(&self) -> bool {
            !self.servers.is_empty()
        }

        async fn session_for(&self, command: &[String]) -> Result<Arc<Mutex<LspClient>>, String> {
            let key = command.join("\u{0}");
            let mut map = self.sessions.lock().await;
            if let Some(existing) = map.get(&key) {
                return Ok(existing.clone());
            }
            let client = LspClient::start(command, &self.cwd).await?;
            let arc = Arc::new(Mutex::new(client));
            map.insert(key, arc.clone());
            Ok(arc)
        }
    }

    fn err(msg: impl Into<String>) -> ToolResult {
        ToolResult { content: msg.into(), is_error: true, images: Vec::new() }
    }

    /// LSP `languageId` for a file extension (a few common ones; falls back to
    /// the extension itself).
    fn language_id(ext: &str) -> &str {
        match ext {
            "rs" => "rust",
            "ts" => "typescript",
            "tsx" => "typescriptreact",
            "js" | "mjs" | "cjs" => "javascript",
            "jsx" => "javascriptreact",
            "py" => "python",
            "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
            "cs" => "csharp",
            "rb" => "ruby",
            other => other,
        }
    }

    #[async_trait]
    impl Tool for LspTool {
        fn name(&self) -> &str {
            "Lsp"
        }

        fn description(&self) -> &str {
            "Code navigation via a language server (experimental).\n\n\
             operation:\n\
             - documentSymbol: list the file's symbols (functions/classes/...). No position needed.\n\
             - definition / references / hover: require `line` and `character` (1-based, as shown in an editor).\n\
             Returns file:line locations. Configure servers under [tools] lsp_servers."
        }

        fn input_schema(&self) -> JsonSchema {
            json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["documentSymbol", "definition", "references", "hover"],
                        "description": "The navigation query to run."
                    },
                    "file_path": { "type": "string", "description": "File to query (absolute, or relative to the workspace)." },
                    "line": { "type": "integer", "description": "1-based line (required for definition/references/hover)." },
                    "character": { "type": "integer", "description": "1-based column (required for definition/references/hover)." }
                },
                "required": ["operation", "file_path"]
            })
        }

        fn is_concurrency_safe(&self, _input: &Value) -> bool {
            false // sessions are serialized
        }

        fn category(&self) -> ToolCategory {
            ToolCategory::Info
        }

        fn describe(&self, input: &Value) -> String {
            let op = input.get("operation").and_then(|v| v.as_str()).unwrap_or("lsp");
            let f = input.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            format!("Lsp {op}: {}", crate::truncate_utf8(f, 60))
        }

        async fn execute(&self, input: Value) -> ToolResult {
            let Some(operation) = input["operation"].as_str() else {
                return err("Missing required parameter: operation");
            };
            let Some(file_path) = input["file_path"].as_str() else {
                return err("Missing required parameter: file_path");
            };
            let abs = {
                let p = Path::new(file_path);
                if p.is_absolute() { p.to_path_buf() } else { self.cwd.join(p) }
            };
            let ext = abs
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            let Some(command) = self.servers.get(&ext).cloned() else {
                return err(format!(
                    "No LSP server configured for '.{ext}'. Add one under [tools] lsp_servers."
                ));
            };
            let text = match std::fs::read_to_string(&abs) {
                Ok(t) => t,
                Err(e) => return err(format!("Failed to read {}: {e}", abs.display())),
            };
            let uri = path_to_uri(&abs);

            let session = match self.session_for(&command).await {
                Ok(s) => s,
                Err(e) => return err(e),
            };
            let mut client = session.lock().await;
            if let Err(e) = client.did_open(&uri, language_id(&ext), &text).await {
                return err(e);
            }

            // Position-bearing operations need line+character.
            let position = || -> Result<Value, String> {
                let line = input["line"].as_u64().ok_or("`line` is required for this operation")? as usize;
                let character = input["character"].as_u64().ok_or("`character` is required")? as usize;
                let line_text = text.lines().nth(line.saturating_sub(1)).unwrap_or("");
                Ok(json!({ "line": line.saturating_sub(1), "character": char_col_to_utf16(line_text, character) }))
            };

            let result = match operation {
                "documentSymbol" => {
                    client.request("textDocument/documentSymbol", json!({ "textDocument": { "uri": uri } })).await
                }
                "definition" => match position() {
                    Ok(pos) => {
                        client.request("textDocument/definition", json!({ "textDocument": { "uri": uri }, "position": pos })).await
                    }
                    Err(e) => return err(e),
                },
                "references" => match position() {
                    Ok(pos) => {
                        client.request("textDocument/references", json!({ "textDocument": { "uri": uri }, "position": pos, "context": { "includeDeclaration": true } })).await
                    }
                    Err(e) => return err(e),
                },
                "hover" => match position() {
                    Ok(pos) => {
                        client.request("textDocument/hover", json!({ "textDocument": { "uri": uri }, "position": pos })).await
                    }
                    Err(e) => return err(e),
                },
                other => return err(format!("Unknown operation '{other}'")),
            };

            match result {
                Ok(value) => ToolResult {
                    content: format_result(operation, &value),
                    is_error: false,
                    images: Vec::new(),
                },
                Err(e) => err(e),
            }
        }
    }

    /// Render a server result into a compact, human/LLM-readable form.
    fn format_result(operation: &str, value: &Value) -> String {
        match operation {
            "documentSymbol" => format_symbols(value),
            "hover" => format_hover(value),
            _ => format_locations(value), // definition / references
        }
    }

    fn symbol_kind_name(kind: u64) -> &'static str {
        // LSP SymbolKind (1-26).
        match kind {
            1 => "file", 2 => "module", 3 => "namespace", 4 => "package", 5 => "class",
            6 => "method", 7 => "property", 8 => "field", 9 => "constructor", 10 => "enum",
            11 => "interface", 12 => "function", 13 => "variable", 14 => "constant",
            15 => "string", 16 => "number", 17 => "boolean", 18 => "array", 19 => "object",
            20 => "key", 21 => "null", 22 => "enum-member", 23 => "struct", 24 => "event",
            25 => "operator", 26 => "type-param", _ => "symbol",
        }
    }

    fn format_symbols(value: &Value) -> String {
        let Some(arr) = value.as_array() else {
            return "(no symbols)".to_string();
        };
        if arr.is_empty() {
            return "(no symbols)".to_string();
        }
        let mut out = String::new();
        fn walk(out: &mut String, node: &Value, depth: usize) {
            let name = node.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let kind = node.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
            // DocumentSymbol uses `range`; SymbolInformation uses `location.range`.
            let line = node
                .get("range")
                .or_else(|| node.get("location").and_then(|l| l.get("range")))
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("line"))
                .and_then(|l| l.as_u64())
                .map(|l| l + 1)
                .unwrap_or(0);
            out.push_str(&format!(
                "{}{} ({}) :{}\n",
                "  ".repeat(depth),
                name,
                symbol_kind_name(kind),
                line
            ));
            if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
                for child in children {
                    walk(out, child, depth + 1);
                }
            }
        }
        for node in arr {
            walk(&mut out, node, 0);
        }
        out
    }

    fn uri_to_display(uri: &str) -> String {
        uri.strip_prefix("file://").map(|s| s.to_string()).unwrap_or_else(|| uri.to_string())
    }

    fn format_locations(value: &Value) -> String {
        // The result may be a single Location, an array of Location, or null.
        let locations: Vec<&Value> = match value {
            Value::Array(arr) => arr.iter().collect(),
            Value::Null => Vec::new(),
            single => vec![single],
        };
        if locations.is_empty() {
            return "(no results)".to_string();
        }
        let mut out = String::new();
        for loc in locations {
            let uri = loc.get("uri").or_else(|| loc.get("targetUri")).and_then(|u| u.as_str()).unwrap_or("");
            let line = loc
                .get("range")
                .or_else(|| loc.get("targetSelectionRange"))
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("line"))
                .and_then(|l| l.as_u64())
                .map(|l| l + 1)
                .unwrap_or(0);
            out.push_str(&format!("{}:{}\n", uri_to_display(uri), line));
        }
        out
    }

    fn format_hover(value: &Value) -> String {
        let contents = value.get("contents");
        match contents {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Object(o)) => o.get("value").and_then(|v| v.as_str()).unwrap_or("(no hover)").to_string(),
            Some(Value::Array(arr)) => arr
                .iter()
                .map(|e| match e {
                    Value::String(s) => s.clone(),
                    Value::Object(o) => o.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => "(no hover)".to_string(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn no_server_configured_is_a_clear_error() {
            let tool = LspTool::new(HashMap::new(), std::env::temp_dir());
            assert!(!tool.has_servers());
        }

        #[test]
        fn format_symbols_renders_a_hierarchical_tree() {
            let v = json!([
                { "name": "Foo", "kind": 5, "range": { "start": { "line": 9 } },
                  "children": [ { "name": "bar", "kind": 6, "range": { "start": { "line": 11 } } } ] }
            ]);
            let out = format_symbols(&v);
            assert!(out.contains("Foo (class) :10"), "got: {out}");
            assert!(out.contains("  bar (method) :12"), "nested + 1-based line: {out}");
        }

        #[test]
        fn format_locations_handles_single_array_and_null() {
            assert_eq!(format_locations(&Value::Null), "(no results)");
            let one = json!({ "uri": "file:///a/b.rs", "range": { "start": { "line": 41 } } });
            assert_eq!(format_locations(&one), "/a/b.rs:42\n");
            let many = json!([
                { "uri": "file:///x.rs", "range": { "start": { "line": 0 } } },
                { "uri": "file:///y.rs", "range": { "start": { "line": 4 } } }
            ]);
            assert_eq!(format_locations(&many), "/x.rs:1\n/y.rs:5\n");
        }

        #[test]
        fn format_hover_extracts_markup_and_plain_and_array() {
            assert_eq!(format_hover(&json!({ "contents": "plain" })), "plain");
            assert_eq!(format_hover(&json!({ "contents": { "kind": "markdown", "value": "**md**" } })), "**md**");
            assert_eq!(format_hover(&json!({ "contents": ["a", { "value": "b" }] })), "a\nb");
        }
    }
}

pub use tool::LspTool;


