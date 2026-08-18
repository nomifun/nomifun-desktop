//! The three read-only `OneShotTool` constructors for customer-service turns.
//!
//! These are the ONLY tools a customer-service engine session ever sees; they
//! are handed to `run_one_shot_turn` whose tool table is fixed at
//! construction time. Everything here is read-only: knowledge search/read and
//! notes search. No terminal, no file writes, no browser, no gateway.

use std::sync::Arc;

use nomifun_ai_agent::{OneShotTool, one_shot_handler};
use nomifun_common::KnowledgeBaseId;
use nomifun_common::text_search::{NoteQueryTerms, expand_query};
use nomifun_db::{ICustomerServiceRepository, NoteMatchChannel};
use nomifun_knowledge::KnowledgeService;

/// Max knowledge hits surfaced per search.
const KNOWLEDGE_SEARCH_LIMIT: usize = 8;
/// Max notes surfaced per search.
const NOTES_SEARCH_LIMIT: usize = 10;
/// Cap on document content returned by `knowledge_read`.
const KNOWLEDGE_READ_MAX_CHARS: usize = 6000;
/// Max queries honoured in one `cs_notes_search` call. More than this is the
/// model padding rather than genuinely rephrasing, and each query costs a scan.
const MAX_QUERIES_PER_CALL: usize = 5;
/// Topics listed when a search misses, to seed the model's retry.
const NOTE_TOPIC_LIMIT: usize = 12;

/// Reply when the queries carried no searchable signal at all (blank, pure
/// punctuation, or only question words). Distinguished from a genuine miss so
/// the model corrects its input rather than concluding no answer exists.
const NO_SEARCHABLE_TERMS: &str =
    "查询词为空或只包含疑问词，无法检索。请传入具体名词，例如产品名或功能名。";

fn queries_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "queries": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "description": "1-5 个简短关键词或不同问法，会被合并召回；不要传整句问题"
            }
        },
        "required": ["queries"]
    })
}

/// Read the `queries` array, tolerating a bare string.
///
/// The lenient string case is deliberate: a model that has seen the older
/// single-`query` contract (or that ignores the schema) would otherwise get a
/// hard error instead of a search, which is a worse failure than accepting the
/// shape and expanding it server-side.
fn extract_queries(input: &serde_json::Value) -> Result<Vec<String>, String> {
    let raw = input
        .get("queries")
        .or_else(|| input.get("query"))
        .ok_or_else(|| "missing required field 'queries' (array of strings)".to_owned())?;
    let candidates: Vec<String> = match raw {
        serde_json::Value::String(single) => vec![single.clone()],
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        _ => return Err("'queries' must be an array of strings".to_owned()),
    };
    let mut queries: Vec<String> = Vec::new();
    for candidate in candidates {
        let trimmed = candidate.trim();
        if !trimmed.is_empty() && !queries.iter().any(|existing| existing == trimmed) {
            queries.push(trimmed.to_owned());
        }
        if queries.len() >= MAX_QUERIES_PER_CALL {
            break;
        }
    }
    if queries.is_empty() {
        return Err("'queries' must contain at least one non-empty string".to_owned());
    }
    Ok(queries)
}

/// Union one expansion into the accumulated terms, preserving order and
/// dropping duplicates so overlapping paraphrases do not inflate the term list.
fn merge_terms(target: &mut NoteQueryTerms, source: NoteQueryTerms) {
    for (dst, src) in [
        (&mut target.fts, source.fts),
        (&mut target.like, source.like),
        (&mut target.bigrams, source.bigrams),
    ] {
        for term in src {
            if !dst.contains(&term) {
                dst.push(term);
            }
        }
    }
}

/// Render a genuine miss as an actionable reply: what is available, and what to
/// do next. The bare "没有找到匹配的客服笔记。" it replaces gave the model no way
/// to recover, so a single unlucky query ended the search.
fn render_miss(topics: Vec<String>) -> String {
    if topics.is_empty() {
        return "没有找到相关客服笔记，且当前没有可用笔记。请如实告知访客无法确认，并建议联系主人。"
            .to_owned();
    }
    let listed = topics
        .iter()
        .filter(|topic| !topic.is_empty())
        .map(|topic| format!("- {topic}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "没有找到相关客服笔记。当前可用笔记主题如下，如与访客问题相关，请挑选其中的\
         核心名词重新调用 cs_notes_search：\n{listed}"
    )
}

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

/// `cs_notes_search`: hybrid recall over the agent's enabled notes
/// (shared + private).
///
/// The input is an ARRAY of queries, not one string. That is the fix for the
/// original defect as much as the index is: the model used to emit a single
/// natural-language string that was matched as one contiguous `LIKE` pattern,
/// so `"NomiFun 是什么"` missed a note reading `"Q：NomiFun是什么？"` purely
/// because of the inserted space. Accepting several short queries turns the
/// model's own paraphrases into OR-branches, and each one is additionally
/// normalized and split server-side by `expand_query`.
///
/// A miss returns the available note topics rather than a bare "not found":
/// `run_one_shot_turn` allows several tool rounds per turn, so the model can
/// re-query — it was simply never told what else existed.
pub fn cs_notes_search_tool(
    repo: Arc<dyn ICustomerServiceRepository>,
    cs_agent_id: String,
) -> OneShotTool {
    OneShotTool {
        name: "cs_notes_search".into(),
        description: "搜索客服笔记（FAQ/话术/业务事实）。传 1-5 个简短关键词或不同问法\
             （数组），不要传整句问题；系统会自动归一化、切词并合并召回。若返回\
             「没有找到」，请换更短的关键词（例如只保留产品名或核心名词）再搜一次。"
            .into(),
        input_schema: queries_schema(),
        handler: one_shot_handler(move |input| {
            let repo = Arc::clone(&repo);
            let cs_agent_id = cs_agent_id.clone();
            async move {
                let queries = extract_queries(&input)?;
                // Every query is expanded and the term sets are unioned, so one
                // good paraphrase rescues four bad ones.
                let mut terms = NoteQueryTerms::default();
                for query in &queries {
                    merge_terms(&mut terms, expand_query(query));
                }
                if terms.is_empty() {
                    return Ok(NO_SEARCHABLE_TERMS.to_owned());
                }
                let hits = repo
                    .search_notes(&cs_agent_id, &terms, NOTES_SEARCH_LIMIT)
                    .await
                    .map_err(|error| format!("notes search failed: {error}"))?;
                if hits.is_empty() {
                    return Ok(render_miss(
                        repo.note_topics(&cs_agent_id, NOTE_TOPIC_LIMIT).await.unwrap_or_default(),
                    ));
                }
                let mut out = String::new();
                for hit in hits {
                    // Weak fallback hits are labelled so the model does not
                    // present a loose bigram overlap as a confident answer.
                    let confidence = match hit.channel {
                        NoteMatchChannel::Bigram => "（弱相关，请自行判断是否切题）",
                        _ => "",
                    };
                    out.push_str(&format!("[{}]{}\n{}\n\n", hit.note.kind, confidence, hit.note.content));
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
    use super::*;

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

    #[test]
    fn queries_accept_an_array_and_dedupe_and_cap() {
        let input = serde_json::json!({ "queries": ["nomifun", " nomifun ", "价格", "", "  "] });
        assert_eq!(extract_queries(&input).unwrap(), vec!["nomifun", "价格"]);

        // More paraphrases than the cap is the model padding; take the first N.
        let many = serde_json::json!({ "queries": ["a", "b", "c", "d", "e", "f", "g"] });
        assert_eq!(extract_queries(&many).unwrap().len(), MAX_QUERIES_PER_CALL);
    }

    /// A model that ignores the array schema, or that learned the older
    /// single-`query` contract, must still get a search rather than an error —
    /// a hard failure here would be a worse regression than the original bug.
    #[test]
    fn queries_tolerate_a_bare_string_and_the_legacy_field_name() {
        let bare = serde_json::json!({ "queries": "NomiFun是什么" });
        assert_eq!(extract_queries(&bare).unwrap(), vec!["NomiFun是什么"]);
        let legacy = serde_json::json!({ "query": "NomiFun是什么" });
        assert_eq!(extract_queries(&legacy).unwrap(), vec!["NomiFun是什么"]);
    }

    #[test]
    fn queries_reject_empty_and_wrongly_typed_input() {
        for bad in [
            serde_json::json!({}),
            serde_json::json!({ "queries": [] }),
            serde_json::json!({ "queries": ["", "   "] }),
            serde_json::json!({ "queries": 42 }),
        ] {
            assert!(extract_queries(&bad).is_err(), "{bad} must be rejected");
        }
    }

    /// Merging is what makes several paraphrases additive: one good query
    /// rescues the others, which is the point of accepting an array.
    #[test]
    fn merging_unions_terms_without_duplicates() {
        let mut merged = NoteQueryTerms::default();
        merge_terms(&mut merged, expand_query("NomiFun是什么"));
        let before = merged.fts.len();
        // The same query again must add nothing.
        merge_terms(&mut merged, expand_query("nomifun 是什么"));
        assert_eq!(merged.fts.len(), before, "{merged:?}");
        // A different query contributes its own terms.
        merge_terms(&mut merged, expand_query("怎么安装"));
        assert!(merged.fts.len() > before, "{merged:?}");
        assert!(merged.fts.contains(&"nomifun".to_owned()), "{merged:?}");
    }

    /// A miss must hand the model something to act on. The bare
    /// "没有找到匹配的客服笔记。" it replaces ended the search, because the model
    /// had no way to know what else to try.
    #[test]
    fn miss_reply_lists_available_topics_for_a_retry() {
        let rendered = render_miss(vec!["NomiFun是什么？".into(), "怎么安装？".into()]);
        assert!(rendered.contains("NomiFun是什么？"), "{rendered}");
        assert!(rendered.contains("cs_notes_search"), "must name the tool to retry: {rendered}");

        // With genuinely no notes there is nothing to retry, so the reply must
        // instead steer toward an honest "cannot confirm".
        let empty = render_miss(Vec::new());
        assert!(!empty.contains("cs_notes_search"), "{empty}");
        assert!(empty.contains("联系主人"), "{empty}");
    }

    #[test]
    fn schema_declares_an_array_of_queries() {
        let schema = queries_schema();
        assert_eq!(schema["properties"]["queries"]["type"], "array");
        assert_eq!(schema["properties"]["queries"]["items"]["type"], "string");
        assert_eq!(schema["required"][0], "queries");
    }
}
