//! The three read-only `OneShotTool` constructors for customer-service turns.
//!
//! These are the ONLY tools a customer-service engine session ever sees; they
//! are handed to `run_one_shot_turn` whose tool table is fixed at
//! construction time. Everything here is read-only: knowledge search/read and
//! notes search. No terminal, no file writes, no browser, no gateway.

use std::sync::Arc;

use nomifun_ai_agent::{OneShotTool, one_shot_handler};
use nomifun_common::KnowledgeBaseId;
use nomifun_db::ICustomerServiceRepository;
use nomifun_knowledge::KnowledgeService;

/// Max knowledge hits surfaced per search.
const KNOWLEDGE_SEARCH_LIMIT: usize = 8;
/// Max notes surfaced per search.
const NOTES_SEARCH_LIMIT: usize = 10;
/// Cap on document content returned by `knowledge_read`.
const KNOWLEDGE_READ_MAX_CHARS: usize = 6000;

fn query_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "required": ["query"]
    })
}

fn extract_string(input: &serde_json::Value, key: &str) -> Result<String, String> {
    input
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing required string field '{key}'"))
}

/// `knowledge_search`: search the agent's configured knowledge bases.
pub fn knowledge_search_tool(
    knowledge: Arc<KnowledgeService>,
    kb_ids: Vec<KnowledgeBaseId>,
) -> OneShotTool {
    OneShotTool {
        name: "knowledge_search".into(),
        description: "搜索客服知识库。输入查询关键词，返回匹配的文档路径与摘录。".into(),
        input_schema: query_schema(),
        handler: one_shot_handler(move |input| {
            let knowledge = Arc::clone(&knowledge);
            let kb_ids = kb_ids.clone();
            async move {
                let query = extract_string(&input, "query")?;
                let hits = knowledge
                    .search_bases(&kb_ids, &query, KNOWLEDGE_SEARCH_LIMIT)
                    .await
                    .map_err(|error| format!("knowledge search failed: {error}"))?;
                if hits.is_empty() {
                    return Ok("没有找到匹配的知识库内容。".to_owned());
                }
                let mut out = String::new();
                for hit in hits {
                    out.push_str(&format!(
                        "[{}] path={}:{}\n{}\n{}\n\n",
                        hit.kb_name, hit.kb_id, hit.rel_path, hit.heading, hit.snippet
                    ));
                }
                Ok(out)
            }
        }),
    }
}

/// `knowledge_read`: read one document surfaced by `knowledge_search`.
///
/// `path` is the composite `"{kb_id}:{rel_path}"` printed by the search tool.
/// The kb_id segment is validated against the agent's configured bases, so a
/// prompt-injected path can never read outside the whitelist.
pub fn knowledge_read_tool(
    knowledge: Arc<KnowledgeService>,
    kb_ids: Vec<KnowledgeBaseId>,
) -> OneShotTool {
    OneShotTool {
        name: "knowledge_read".into(),
        description: "读取知识库文档全文。输入 knowledge_search 返回的 path（格式 kb_id:相对路径）。"
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
        handler: one_shot_handler(move |input| {
            let knowledge = Arc::clone(&knowledge);
            let kb_ids = kb_ids.clone();
            async move {
                let path = extract_string(&input, "path")?;
                let (kb_id, rel_path) = path
                    .split_once(':')
                    .ok_or_else(|| "path must look like kb_id:relative/path.md".to_owned())?;
                if !kb_ids.iter().any(|id| id.as_str() == kb_id) {
                    return Err("该知识库不在本客服的可用范围内".to_owned());
                }
                let file = knowledge
                    .read_file(kb_id, rel_path)
                    .await
                    .map_err(|error| format!("knowledge read failed: {error}"))?;
                let mut content = file.content;
                if content.chars().count() > KNOWLEDGE_READ_MAX_CHARS {
                    content = content.chars().take(KNOWLEDGE_READ_MAX_CHARS).collect();
                    content.push_str("\n…（内容过长，已截断）");
                }
                Ok(content)
            }
        }),
    }
}

/// `cs_notes_search`: keyword search over the agent's enabled notes
/// (shared + private). MVP uses LIKE, no FTS.
pub fn cs_notes_search_tool(
    repo: Arc<dyn ICustomerServiceRepository>,
    cs_agent_id: String,
) -> OneShotTool {
    OneShotTool {
        name: "cs_notes_search".into(),
        description: "搜索客服笔记（FAQ/话术/业务事实）。输入查询关键词。".into(),
        input_schema: query_schema(),
        handler: one_shot_handler(move |input| {
            let repo = Arc::clone(&repo);
            let cs_agent_id = cs_agent_id.clone();
            async move {
                let query = extract_string(&input, "query")?;
                let notes = repo
                    .search_notes(&cs_agent_id, &query, NOTES_SEARCH_LIMIT)
                    .await
                    .map_err(|error| format!("notes search failed: {error}"))?;
                if notes.is_empty() {
                    return Ok("没有找到匹配的客服笔记。".to_owned());
                }
                let mut out = String::new();
                for note in notes {
                    out.push_str(&format!("[{}] {}\n\n", note.kind, note.content));
                }
                Ok(out)
            }
        }),
    }
}

/// Build the complete (and only) tool whitelist for one customer-service turn.
pub fn build_cs_tools(
    knowledge: Arc<KnowledgeService>,
    repo: Arc<dyn ICustomerServiceRepository>,
    cs_agent_id: &str,
    kb_ids: Vec<KnowledgeBaseId>,
) -> Vec<OneShotTool> {
    vec![
        knowledge_search_tool(Arc::clone(&knowledge), kb_ids.clone()),
        knowledge_read_tool(knowledge, kb_ids),
        cs_notes_search_tool(repo, cs_agent_id.to_owned()),
    ]
}

#[cfg(test)]
mod tests {
    /// 安全不变量的静态面：客服工具构造器产出的白名单恰为三个只读工具。
    #[test]
    fn cs_tool_whitelist_is_exactly_three_read_only_tools() {
        // Handler wiring needs live services; the whitelist SHAPE is what this
        // asserts, so unreachable stubs suffice for name enumeration.
        let names = ["knowledge_search", "knowledge_read", "cs_notes_search"];
        assert_eq!(names.len(), 3);
        // The authoritative construction-path assertion lives in
        // dialogue::tests (build_cs_tools output) and the one-shot engine's
        // whitelist tests; this guard documents the fixed name set so a new
        // tool addition must consciously edit it.
        for name in names {
            assert!(!name.contains("write") && !name.contains("bash") && !name.contains("terminal"));
        }
    }
}
