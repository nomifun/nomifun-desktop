//! Feishu (Lark) docx block JSON → Markdown converter.
//!
//! Converts the `items` array returned by `GET /open-apis/docx/v1/documents/{id}/blocks`
//! into a Markdown string. Pure function, no network, no side effects.

use std::collections::HashMap;

/// Convert a Feishu docx document's block list (the `items` array returned by
/// GET /open-apis/docx/v1/documents/{id}/blocks) into Markdown.
pub fn blocks_to_markdown(blocks: &[serde_json::Value]) -> String {
    let index = build_index(blocks);
    let root_id = find_root(&index);
    let mut out = String::new();
    if let Some(root_id) = root_id {
        render_children(&index, &root_id, 0, &mut out, &mut OrderContext::default());
    }
    // Trim trailing whitespace but keep a single trailing newline
    let trimmed = out.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

// ─── Block index ────────────────────────────────────────────────────────────

type BlockIndex<'a> = HashMap<&'a str, &'a serde_json::Value>;

fn build_index(blocks: &[serde_json::Value]) -> BlockIndex<'_> {
    let mut map = HashMap::new();
    for block in blocks {
        if let Some(id) = block.get("block_id").and_then(|v| v.as_str()) {
            map.insert(id, block);
        }
    }
    map
}

fn find_root<'a>(index: &'a BlockIndex<'a>) -> Option<&'a str> {
    // Root = block_type 1, or block whose parent_id is empty / not in the set
    for (id, block) in index.iter() {
        let bt = block.get("block_type").and_then(|v| v.as_i64()).unwrap_or(0);
        if bt == 1 {
            return Some(id);
        }
    }
    // Fallback: block whose parent_id is not in the index
    for (id, block) in index.iter() {
        let parent = block.get("parent_id").and_then(|v| v.as_str()).unwrap_or("");
        if parent.is_empty() || !index.contains_key(parent) {
            return Some(id);
        }
    }
    None
}

// ─── Ordered-list numbering context ─────────────────────────────────────────

#[derive(Default)]
struct OrderContext {
    /// Maps (parent_block_id, depth) → running counter for ordered lists
    counters: HashMap<(String, usize), usize>,
}

impl OrderContext {
    fn next_number(&mut self, parent_id: &str, depth: usize) -> usize {
        let key = (parent_id.to_owned(), depth);
        let counter = self.counters.entry(key).or_insert(0);
        *counter += 1;
        *counter
    }

    fn reset(&mut self, parent_id: &str, depth: usize) {
        self.counters.remove(&(parent_id.to_owned(), depth));
    }
}

// ─── Rendering ──────────────────────────────────────────────────────────────

fn render_children(
    index: &BlockIndex<'_>,
    block_id: &str,
    depth: usize,
    out: &mut String,
    ctx: &mut OrderContext,
) {
    let block = match index.get(block_id) {
        Some(b) => *b,
        None => return,
    };
    let children: Vec<&str> = block
        .get("children")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut prev_was_ordered = false;

    for child_id in &children {
        let child = match index.get(*child_id) {
            Some(b) => *b,
            None => continue,
        };
        let bt = child.get("block_type").and_then(|v| v.as_i64()).unwrap_or(0);

        // Reset ordered list counter when switching away from ordered list
        if bt != 14 && prev_was_ordered {
            ctx.reset(block_id, depth);
        }
        prev_was_ordered = bt == 14;

        render_block(index, child_id, child, bt, depth, block_id, out, ctx);
    }
    // Reset counter at end of children list
    if prev_was_ordered {
        ctx.reset(block_id, depth);
    }
}

fn render_block(
    index: &BlockIndex<'_>,
    block_id: &str,
    block: &serde_json::Value,
    block_type: i64,
    depth: usize,
    parent_id: &str,
    out: &mut String,
    ctx: &mut OrderContext,
) {
    let indent = "  ".repeat(depth);

    match block_type {
        // Page (root) — just render children
        1 => {
            render_children(index, block_id, depth, out, ctx);
        }
        // Text paragraph
        2 => {
            let text = render_elements(get_elements(block, "text"));
            out.push_str(&format!("{indent}{text}\n\n"));
            render_children(index, block_id, depth, out, ctx);
        }
        // Headings h1..h9 — block_type 3..=11 (Feishu: heading1=3 … heading9=11)
        3..=11 => {
            let level = (block_type - 2) as usize; // 3→1, 4→2, …, 11→9
            let key = format!("heading{level}");
            let text = render_elements(get_elements(block, &key));
            let hashes = "#".repeat(level);
            out.push_str(&format!("{indent}{hashes} {text}\n\n"));
            render_children(index, block_id, depth, out, ctx);
        }
        // Bullet list (block_type 12)
        12 => {
            let text = render_elements(get_elements(block, "bullet"));
            out.push_str(&format!("{indent}- {text}\n"));
            render_children(index, block_id, depth + 1, out, ctx);
        }
        // Ordered list (block_type 13)
        13 => {
            let num = ctx.next_number(parent_id, depth);
            let text = render_elements(get_elements(block, "ordered"));
            out.push_str(&format!("{indent}{num}. {text}\n"));
            render_children(index, block_id, depth + 1, out, ctx);
        }
        // Code block (block_type 14)
        14 => {
            let payload = block.get("code");
            let lang = payload
                .and_then(|p| p.get("style"))
                .and_then(|s| s.get("language"))
                .and_then(|l| l.as_i64())
                .map(lang_id_to_str)
                .unwrap_or("");
            let text = render_elements(
                payload
                    .and_then(|p| p.get("elements"))
                    .and_then(|e| e.as_array()),
            );
            out.push_str(&format!("{indent}```{lang}\n{indent}{text}\n{indent}```\n\n"));
        }
        // Quote (block_type 15)
        15 => {
            let text = render_elements(get_elements(block, "quote"));
            if !text.is_empty() {
                for line in text.lines() {
                    out.push_str(&format!("{indent}> {line}\n"));
                }
                out.push('\n');
            }
            // Render children as quoted
            render_children_quoted(index, block_id, depth, out, ctx);
        }
        // Todo (block_type 17)
        17 => {
            let payload = block.get("todo");
            let done = payload
                .and_then(|p| p.get("style"))
                .and_then(|s| s.get("done"))
                .and_then(|d| d.as_bool())
                .unwrap_or(false);
            let checkbox = if done { "[x]" } else { "[ ]" };
            let text = render_elements(
                payload
                    .and_then(|p| p.get("elements"))
                    .and_then(|e| e.as_array()),
            );
            out.push_str(&format!("{indent}- {checkbox} {text}\n"));
            render_children(index, block_id, depth + 1, out, ctx);
        }
        // Callout → blockquote
        19 => {
            let text = render_elements(get_elements(block, "callout"));
            if !text.is_empty() {
                for line in text.lines() {
                    out.push_str(&format!("{indent}> {line}\n"));
                }
                out.push('\n');
            }
            render_children_quoted(index, block_id, depth, out, ctx);
        }
        // Divider
        22 => {
            out.push_str(&format!("{indent}---\n\n"));
        }
        // Image
        27 => {
            let token = block
                .get("image")
                .and_then(|img| img.get("token"))
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            out.push_str(&format!("{indent}![](feishu-image:{token})\n\n"));
        }
        // Table
        31 => {
            render_table(index, block, depth, out, ctx);
        }
        // Table cell — normally rendered by table handler, but if encountered standalone:
        32 => {
            render_children(index, block_id, depth, out, ctx);
        }
        // Quote container
        34 => {
            render_children_quoted(index, block_id, depth, out, ctx);
        }
        // Unknown
        _ => {
            out.push_str(&format!(
                "{indent}<!-- feishu:unsupported block_type={block_type} -->\n"
            ));
            render_children(index, block_id, depth, out, ctx);
        }
    }
}

/// Render children of a block as blockquoted lines
fn render_children_quoted(
    index: &BlockIndex<'_>,
    block_id: &str,
    depth: usize,
    out: &mut String,
    ctx: &mut OrderContext,
) {
    let block = match index.get(block_id) {
        Some(b) => *b,
        None => return,
    };
    let children: Vec<&str> = block
        .get("children")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if children.is_empty() {
        return;
    }

    // Render children into a temporary buffer, then prefix each line with `> `
    let mut child_buf = String::new();
    let mut prev_was_ordered = false;
    for child_id in &children {
        let child = match index.get(*child_id) {
            Some(b) => *b,
            None => continue,
        };
        let bt = child.get("block_type").and_then(|v| v.as_i64()).unwrap_or(0);
        if bt != 14 && prev_was_ordered {
            ctx.reset(block_id, depth);
        }
        prev_was_ordered = bt == 14;
        render_block(index, child_id, child, bt, 0, block_id, &mut child_buf, ctx);
    }
    if prev_was_ordered {
        ctx.reset(block_id, depth);
    }

    let indent = "  ".repeat(depth);
    for line in child_buf.lines() {
        if line.is_empty() {
            out.push_str(&format!("{indent}>\n"));
        } else {
            out.push_str(&format!("{indent}> {line}\n"));
        }
    }
    out.push('\n');
}

// ─── Table rendering ────────────────────────────────────────────────────────

fn render_table(
    index: &BlockIndex<'_>,
    block: &serde_json::Value,
    depth: usize,
    out: &mut String,
    ctx: &mut OrderContext,
) {
    let indent = "  ".repeat(depth);

    // Table property may contain row/col counts
    let _table_prop = block.get("table");

    // Children of a table block are rows (implicitly); each row's children are cells
    let row_ids: Vec<&str> = block
        .get("children")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Collect cell text for each row
    let mut rows: Vec<Vec<String>> = Vec::new();
    for row_id in &row_ids {
        let row_block = match index.get(*row_id) {
            Some(b) => *b,
            None => continue,
        };
        // A table_cell (type 32) contains children that are text blocks
        // But the table structure might be: table → cells directly (grid layout)
        // Or table → rows → cells. Feishu uses a flat children list where
        // the table property has `column_size` to determine row breaks.
        let cell_ids: Vec<&str> = row_block
            .get("children")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        if cell_ids.is_empty() {
            // The row_block itself might be a cell
            let cell_text = extract_cell_text(index, row_block, *row_id, ctx);
            rows.push(vec![cell_text]);
        } else {
            let mut row_cells = Vec::new();
            for cell_id in &cell_ids {
                let cell_block = match index.get(*cell_id) {
                    Some(b) => *b,
                    None => continue,
                };
                let cell_text = extract_cell_text(index, cell_block, cell_id, ctx);
                row_cells.push(cell_text);
            }
            rows.push(row_cells);
        }
    }

    // Determine column count from table property or max row width
    let col_size = block
        .get("table")
        .and_then(|t| t.get("property"))
        .and_then(|p| p.get("column_size"))
        .and_then(|c| c.as_u64())
        .unwrap_or(0) as usize;

    // If we got a flat list of cells (each "row" has 1 cell) and col_size > 0,
    // reshape into rows
    let rows = if col_size > 0 && rows.iter().all(|r| r.len() == 1) && rows.len() > col_size {
        let flat: Vec<String> = rows.into_iter().map(|r| r.into_iter().next().unwrap_or_default()).collect();
        flat.chunks(col_size).map(|chunk| chunk.to_vec()).collect()
    } else {
        rows
    };

    if rows.is_empty() {
        return;
    }

    let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);

    for (i, row) in rows.iter().enumerate() {
        out.push_str(&indent);
        out.push('|');
        for col in 0..max_cols {
            let cell = row.get(col).map(|s| s.as_str()).unwrap_or("");
            out.push_str(&format!(" {cell} |"));
        }
        out.push('\n');

        // After first row, emit separator
        if i == 0 {
            out.push_str(&indent);
            out.push('|');
            for _ in 0..max_cols {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.push('\n');
}

fn extract_cell_text(
    index: &BlockIndex<'_>,
    cell_block: &serde_json::Value,
    cell_id: &str,
    ctx: &mut OrderContext,
) -> String {
    // Cell children are text/paragraph blocks — render them inline
    let child_ids: Vec<&str> = cell_block
        .get("children")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut parts = Vec::new();
    for child_id in &child_ids {
        let child = match index.get(*child_id) {
            Some(b) => *b,
            None => continue,
        };
        let bt = child.get("block_type").and_then(|v| v.as_i64()).unwrap_or(0);
        match bt {
            2 => parts.push(render_elements(get_elements(child, "text"))),
            3..=11 => {
                let level = (bt - 2) as usize;
                let key = format!("heading{level}");
                parts.push(render_elements(get_elements(child, &key)));
            }
            _ => {
                // Render generically into buffer
                let mut buf = String::new();
                render_block(index, child_id, child, bt, 0, cell_id, &mut buf, ctx);
                let trimmed = buf.trim().to_owned();
                if !trimmed.is_empty() {
                    parts.push(trimmed);
                }
            }
        }
    }

    // If no children, try elements directly on the cell
    if parts.is_empty() {
        let direct = render_elements(
            cell_block
                .get("elements")
                .and_then(|e| e.as_array()),
        );
        if !direct.is_empty() {
            return direct;
        }
        // Try "text" payload
        let text = render_elements(get_elements(cell_block, "text"));
        if !text.is_empty() {
            return text;
        }
    }

    parts.join(" ").replace('|', "\\|")
}

// ─── Inline element rendering ───────────────────────────────────────────────

fn get_elements<'a>(block: &'a serde_json::Value, payload_key: &str) -> Option<&'a Vec<serde_json::Value>> {
    block
        .get(payload_key)
        .and_then(|p| p.get("elements"))
        .and_then(|e| e.as_array())
}

fn render_elements(elements: Option<&Vec<serde_json::Value>>) -> String {
    let elements = match elements {
        Some(e) => e,
        None => return String::new(),
    };

    let mut result = String::new();
    for elem in elements {
        if let Some(text_run) = elem.get("text_run") {
            let content = text_run
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            if content.is_empty() {
                continue;
            }

            let style = text_run.get("text_element_style");

            // Check for link
            let link_url = style
                .and_then(|s| s.get("link"))
                .and_then(|l| l.get("url"))
                .and_then(|u| u.as_str());

            let bold = style
                .and_then(|s| s.get("bold"))
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let italic = style
                .and_then(|s| s.get("italic"))
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let strikethrough = style
                .and_then(|s| s.get("strikethrough"))
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let inline_code = style
                .and_then(|s| s.get("inline_code"))
                .and_then(|b| b.as_bool())
                .unwrap_or(false);

            let mut text = content.to_owned();

            if inline_code {
                text = format!("`{text}`");
            } else {
                if bold {
                    text = format!("**{text}**");
                }
                if italic {
                    text = format!("*{text}*");
                }
                if strikethrough {
                    text = format!("~~{text}~~");
                }
            }

            if let Some(url) = link_url {
                text = format!("[{text}]({url})");
            }

            result.push_str(&text);
        } else if let Some(mention_user) = elem.get("mention_user") {
            let user_id = mention_user
                .get("user_id")
                .and_then(|u| u.as_str())
                .or_else(|| mention_user.get("name").and_then(|n| n.as_str()))
                .unwrap_or("unknown");
            result.push_str(&format!("@{user_id}"));
        } else if let Some(mention_doc) = elem.get("mention_doc") {
            let title = mention_doc
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("doc");
            let url = mention_doc
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("");
            if url.is_empty() {
                result.push_str(title);
            } else {
                result.push_str(&format!("[{title}]({url})"));
            }
        } else if let Some(equation) = elem.get("equation") {
            let content = equation
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            if !content.is_empty() {
                result.push_str(&format!("${content}$"));
            }
        }
    }
    result
}

// ─── Language mapping ───────────────────────────────────────────────────────

fn lang_id_to_str(id: i64) -> &'static str {
    #[rustfmt::skip]
    const LANGS: &[&str] = &[
        /*  0 */ "", "plaintext", "abap", "ada", "apache", "apex",
        /*  6 */ "assembly", "bash", "basic", "bnf", "c",
        /* 11 */ "c#", "c++", "capnproto", "clojure", "cmake",
        /* 16 */ "coffeescript", "coq", "css", "dart", "delphi",
        /* 21 */ "django", "dockerfile", "elixir", "elm", "erlang",
        /* 26 */ "excel", "fortran", "go", "gradle", "graphql",
        /* 31 */ "groovy", "haskell", "html", "http", "ini",
        /* 36 */ "java", "javascript", "json", "julia", "kotlin",
        /* 41 */ "latex", "less", "lisp", "lua", "makefile",
        /* 46 */ "markdown", "matlab", "nginx", "objectivec", "pascal",
        /* 51 */ "perl", "php", "powershell", "prolog", "protobuf",
        /* 56 */ "python", "r", "ruby", "rust", "sass",
        /* 61 */ "scala", "scheme", "scss", "shell", "sql",
        /* 66 */ "swift", "thrift", "toml", "typescript", "vbnet",
        /* 71 */ "vim", "xml", "yaml",
    ];
    LANGS.get(id as usize).copied().unwrap_or("")
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper: wrap blocks in a page root with children pointing to them
    fn doc(children: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        let child_ids: Vec<String> = children
            .iter()
            .map(|b| b["block_id"].as_str().unwrap().to_owned())
            .collect();
        let mut all = vec![json!({
            "block_id": "root",
            "parent_id": "",
            "block_type": 1,
            "children": child_ids
        })];
        all.extend(children);
        all
    }

    #[test]
    fn test_heading_h1_to_h9() {
        for level in 1..=9u32 {
            let bt = level as i64 + 2; // h1=3, h2=4, ..., h9=11
            let key = format!("heading{level}");
            let block = json!({
                "block_id": format!("h{level}"),
                "parent_id": "root",
                "block_type": bt,
                "children": [],
                key: {
                    "elements": [
                        { "text_run": { "content": format!("Heading {level}"), "text_element_style": {} } }
                    ]
                }
            });
            let md = blocks_to_markdown(&doc(vec![block]));
            let hashes = "#".repeat(level as usize);
            assert_eq!(md.trim(), format!("{hashes} Heading {level}"));
        }
    }

    #[test]
    fn test_paragraph_with_styles() {
        let block = json!({
            "block_id": "p1",
            "parent_id": "root",
            "block_type": 2,
            "children": [],
            "text": {
                "elements": [
                    { "text_run": { "content": "bold", "text_element_style": { "bold": true } } },
                    { "text_run": { "content": " and ", "text_element_style": {} } },
                    { "text_run": { "content": "italic", "text_element_style": { "italic": true } } },
                    { "text_run": { "content": " and ", "text_element_style": {} } },
                    { "text_run": { "content": "code", "text_element_style": { "inline_code": true } } },
                    { "text_run": { "content": " and ", "text_element_style": {} } },
                    { "text_run": { "content": "strike", "text_element_style": { "strikethrough": true } } }
                ]
            }
        });
        let md = blocks_to_markdown(&doc(vec![block]));
        assert_eq!(md.trim(), "**bold** and *italic* and `code` and ~~strike~~");
    }

    #[test]
    fn test_bullet_list_nested() {
        let blocks = doc(vec![
            json!({
                "block_id": "b1",
                "parent_id": "root",
                "block_type": 12,
                "children": ["b2"],
                "bullet": { "elements": [{ "text_run": { "content": "item 1", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "b2",
                "parent_id": "b1",
                "block_type": 12,
                "children": [],
                "bullet": { "elements": [{ "text_run": { "content": "nested", "text_element_style": {} } }] }
            }),
        ]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("- item 1"));
        assert!(md.contains("  - nested"));
    }

    #[test]
    fn test_ordered_list() {
        let blocks = doc(vec![
            json!({
                "block_id": "o1",
                "parent_id": "root",
                "block_type": 13,
                "children": [],
                "ordered": { "elements": [{ "text_run": { "content": "first", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "o2",
                "parent_id": "root",
                "block_type": 13,
                "children": [],
                "ordered": { "elements": [{ "text_run": { "content": "second", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "o3",
                "parent_id": "root",
                "block_type": 13,
                "children": [],
                "ordered": { "elements": [{ "text_run": { "content": "third", "text_element_style": {} } }] }
            }),
        ]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("1. first"));
        assert!(md.contains("2. second"));
        assert!(md.contains("3. third"));
    }

    #[test]
    fn test_todo_done_and_undone() {
        let blocks = doc(vec![
            json!({
                "block_id": "t1",
                "parent_id": "root",
                "block_type": 17,
                "children": [],
                "todo": {
                    "elements": [{ "text_run": { "content": "done task", "text_element_style": {} } }],
                    "style": { "done": true }
                }
            }),
            json!({
                "block_id": "t2",
                "parent_id": "root",
                "block_type": 17,
                "children": [],
                "todo": {
                    "elements": [{ "text_run": { "content": "pending task", "text_element_style": {} } }],
                    "style": { "done": false }
                }
            }),
        ]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("- [x] done task"));
        assert!(md.contains("- [ ] pending task"));
    }

    #[test]
    fn test_code_block_with_language() {
        let blocks = doc(vec![json!({
            "block_id": "c1",
            "parent_id": "root",
            "block_type": 14,
            "children": [],
            "code": {
                "elements": [{ "text_run": { "content": "fn main() {}", "text_element_style": {} } }],
                "style": { "language": 59 }
            }
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("```rust"));
        assert!(md.contains("fn main() {}"));
        assert!(md.contains("```"));
    }

    #[test]
    fn test_quote() {
        let blocks = doc(vec![json!({
            "block_id": "q1",
            "parent_id": "root",
            "block_type": 15,
            "children": [],
            "quote": {
                "elements": [{ "text_run": { "content": "quoted text", "text_element_style": {} } }]
            }
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("> quoted text"));
    }

    #[test]
    fn test_divider() {
        let blocks = doc(vec![json!({
            "block_id": "d1",
            "parent_id": "root",
            "block_type": 22,
            "children": []
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("---"));
    }

    #[test]
    fn test_link() {
        let blocks = doc(vec![json!({
            "block_id": "l1",
            "parent_id": "root",
            "block_type": 2,
            "children": [],
            "text": {
                "elements": [{
                    "text_run": {
                        "content": "click here",
                        "text_element_style": {
                            "link": { "url": "https://example.com" }
                        }
                    }
                }]
            }
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("[click here](https://example.com)"));
    }

    #[test]
    fn test_unknown_block_type() {
        let blocks = doc(vec![json!({
            "block_id": "u1",
            "parent_id": "root",
            "block_type": 999,
            "children": []
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("<!-- feishu:unsupported block_type=999 -->"));
    }

    #[test]
    fn test_equation() {
        let blocks = doc(vec![json!({
            "block_id": "eq1",
            "parent_id": "root",
            "block_type": 2,
            "children": [],
            "text": {
                "elements": [
                    { "text_run": { "content": "Euler: ", "text_element_style": {} } },
                    { "equation": { "content": "e^{i\\pi} + 1 = 0" } }
                ]
            }
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("Euler: $e^{i\\pi} + 1 = 0$"));
    }

    #[test]
    fn test_mention_user() {
        let blocks = doc(vec![json!({
            "block_id": "m1",
            "parent_id": "root",
            "block_type": 2,
            "children": [],
            "text": {
                "elements": [
                    { "text_run": { "content": "Hello ", "text_element_style": {} } },
                    { "mention_user": { "user_id": "ou_abc123" } }
                ]
            }
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("Hello @ou_abc123"));
    }

    #[test]
    fn test_image() {
        let blocks = doc(vec![json!({
            "block_id": "img1",
            "parent_id": "root",
            "block_type": 27,
            "children": [],
            "image": { "token": "boxcnXYZ123" }
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("![](feishu-image:boxcnXYZ123)"));
    }

    #[test]
    fn test_quote_container_with_children() {
        let blocks = doc(vec![
            json!({
                "block_id": "qc1",
                "parent_id": "root",
                "block_type": 34,
                "children": ["qc_child1", "qc_child2"]
            }),
            json!({
                "block_id": "qc_child1",
                "parent_id": "qc1",
                "block_type": 2,
                "children": [],
                "text": { "elements": [{ "text_run": { "content": "line one", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "qc_child2",
                "parent_id": "qc1",
                "block_type": 2,
                "children": [],
                "text": { "elements": [{ "text_run": { "content": "line two", "text_element_style": {} } }] }
            }),
        ]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("> line one"), "got: {md}");
        assert!(md.contains("> line two"), "got: {md}");
    }

    #[test]
    fn test_multi_block_document() {
        let blocks = vec![
            json!({
                "block_id": "root",
                "parent_id": "",
                "block_type": 1,
                "children": ["title", "para", "div", "list1", "list2", "code"]
            }),
            json!({
                "block_id": "title",
                "parent_id": "root",
                "block_type": 3,
                "children": [],
                "heading1": { "elements": [{ "text_run": { "content": "My Document", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "para",
                "parent_id": "root",
                "block_type": 2,
                "children": [],
                "text": { "elements": [
                    { "text_run": { "content": "Hello ", "text_element_style": {} } },
                    { "text_run": { "content": "world", "text_element_style": { "bold": true } } }
                ] }
            }),
            json!({
                "block_id": "div",
                "parent_id": "root",
                "block_type": 22,
                "children": []
            }),
            json!({
                "block_id": "list1",
                "parent_id": "root",
                "block_type": 12,
                "children": [],
                "bullet": { "elements": [{ "text_run": { "content": "bullet one", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "list2",
                "parent_id": "root",
                "block_type": 12,
                "children": [],
                "bullet": { "elements": [{ "text_run": { "content": "bullet two", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "code",
                "parent_id": "root",
                "block_type": 14,
                "children": [],
                "code": {
                    "elements": [{ "text_run": { "content": "let x = 1;", "text_element_style": {} } }],
                    "style": { "language": 37 }
                }
            }),
        ];
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("# My Document"), "got: {md}");
        assert!(md.contains("Hello **world**"), "got: {md}");
        assert!(md.contains("---"), "got: {md}");
        assert!(md.contains("- bullet one"), "got: {md}");
        assert!(md.contains("- bullet two"), "got: {md}");
        assert!(md.contains("```javascript"), "got: {md}");
        assert!(md.contains("let x = 1;"), "got: {md}");
    }

    #[test]
    fn test_table_basic() {
        let blocks = vec![
            json!({
                "block_id": "root",
                "parent_id": "",
                "block_type": 1,
                "children": ["tbl"]
            }),
            json!({
                "block_id": "tbl",
                "parent_id": "root",
                "block_type": 31,
                "children": ["cell1", "cell2", "cell3", "cell4"],
                "table": { "property": { "column_size": 2 } }
            }),
            json!({
                "block_id": "cell1",
                "parent_id": "tbl",
                "block_type": 32,
                "children": ["cell1_text"]
            }),
            json!({
                "block_id": "cell1_text",
                "parent_id": "cell1",
                "block_type": 2,
                "children": [],
                "text": { "elements": [{ "text_run": { "content": "A", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "cell2",
                "parent_id": "tbl",
                "block_type": 32,
                "children": ["cell2_text"]
            }),
            json!({
                "block_id": "cell2_text",
                "parent_id": "cell2",
                "block_type": 2,
                "children": [],
                "text": { "elements": [{ "text_run": { "content": "B", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "cell3",
                "parent_id": "tbl",
                "block_type": 32,
                "children": ["cell3_text"]
            }),
            json!({
                "block_id": "cell3_text",
                "parent_id": "cell3",
                "block_type": 2,
                "children": [],
                "text": { "elements": [{ "text_run": { "content": "C", "text_element_style": {} } }] }
            }),
            json!({
                "block_id": "cell4",
                "parent_id": "tbl",
                "block_type": 32,
                "children": ["cell4_text"]
            }),
            json!({
                "block_id": "cell4_text",
                "parent_id": "cell4",
                "block_type": 2,
                "children": [],
                "text": { "elements": [{ "text_run": { "content": "D", "text_element_style": {} } }] }
            }),
        ];
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("| A | B |"), "got: {md}");
        assert!(md.contains("| --- | --- |"), "got: {md}");
        assert!(md.contains("| C | D |"), "got: {md}");
    }

    #[test]
    fn test_empty_blocks() {
        let md = blocks_to_markdown(&[]);
        assert_eq!(md, "");
    }

    #[test]
    fn test_callout_as_blockquote() {
        let blocks = doc(vec![json!({
            "block_id": "co1",
            "parent_id": "root",
            "block_type": 19,
            "children": [],
            "callout": {
                "elements": [{ "text_run": { "content": "Important note", "text_element_style": {} } }]
            }
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("> Important note"), "got: {md}");
    }

    #[test]
    fn test_mention_doc() {
        let blocks = doc(vec![json!({
            "block_id": "md1",
            "parent_id": "root",
            "block_type": 2,
            "children": [],
            "text": {
                "elements": [
                    { "mention_doc": { "title": "Design Doc", "url": "https://feishu.cn/doc/xxx" } }
                ]
            }
        })]);
        let md = blocks_to_markdown(&blocks);
        assert!(md.contains("[Design Doc](https://feishu.cn/doc/xxx)"), "got: {md}");
    }
}
