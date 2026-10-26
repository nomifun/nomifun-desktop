//! Native Docx blocks through the official CLI raw API. Editor text is literal
//! text, never Markdown or a reconstruction of an exported document.
use super::*;

pub struct NativeDocumentMetadata {
    pub id: String,
    pub title: String,
    pub revision: String,
}
pub struct NativeDocumentPage {
    pub blocks: Vec<Value>,
    pub next_cursor: Option<String>,
}

fn native_id(value: &str) -> Result<(), HubError> {
    if value.is_empty() || matches!(value, "0" | "-1") || value.len() > 512 || !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)) {
        return Err(HubError::invalid("文档和段落必须使用读取返回的原生标识"));
    }
    Ok(())
}
fn native_revision(value: &str) -> Result<u64, HubError> {
    value.bytes().all(|byte| byte.is_ascii_digit()).then(|| value.parse::<u64>().ok()).flatten().filter(|revision| *revision > 0)
        .ok_or_else(|| HubError::invalid("文档编辑必须使用实际版本"))
}
pub fn document_metadata_command(document_id: &str, profile: Option<&str>) -> Result<CliCommand, HubError> {
    native_id(document_id)?;
    user_command(["api", "GET", &format!("/open-apis/docx/v1/documents/{document_id}"), "--format", "json"], profile, false)
}
pub fn normalize_document_metadata(output: &CliOutput) -> Result<NativeDocumentMetadata, HubError> {
    let data = business_value(output, false)?;
    let document = data.get("document").ok_or_else(|| HubError::unavailable("文档基本信息缺失"))?;
    let id = document.get("document_id").and_then(Value::as_str).ok_or_else(|| HubError::unavailable("文档缺少原生标识"))?.to_owned();
    native_id(&id)?;
    let revision = text(document.get("revision_id")).ok_or_else(|| HubError::unavailable("文档缺少实际版本"))?;
    native_revision(&revision)?;
    let title = document.get("title").and_then(Value::as_str).ok_or_else(|| HubError::unavailable("文档标题缺失"))?.to_owned();
    if !warnings(&data).is_empty() { return Err(HubError::unavailable("文档基本信息包含未确认警告")); }
    Ok(NativeDocumentMetadata { id, title, revision })
}
pub fn document_blocks_command(document_id: &str, revision: &str, cursor: Option<&str>, profile: Option<&str>) -> Result<CliCommand, HubError> {
    native_id(document_id)?;
    let mut params = json!({"document_revision_id": native_revision(revision)?, "page_size": 500, "user_id_type": "open_id"});
    if let Some(cursor) = cursor { valid_id(cursor, "文档分页标记")?; params["page_token"] = json!(cursor); }
    user_command(["api".into(), "GET".into(), format!("/open-apis/docx/v1/documents/{document_id}/blocks"),
        "--params".into(), params.to_string(), "--format".into(), "json".into()], profile, false)
}
pub fn normalize_document_blocks(output: &CliOutput) -> Result<NativeDocumentPage, HubError> {
    let data = business_value(output, false)?;
    if !warnings(&data).is_empty() { return Err(HubError::unavailable("文档块包含未确认警告")); }
    let blocks = data.get("items").and_then(Value::as_array).ok_or_else(|| HubError::unavailable("文档块列表缺失"))?.clone();
    let has_more = data.get("has_more").and_then(Value::as_bool).ok_or_else(|| HubError::unavailable("文档块分页状态缺失"))?;
    let next_cursor = if has_more {
        Some(data.get("page_token").and_then(Value::as_str).filter(|cursor| !cursor.is_empty())
            .ok_or_else(|| HubError::unavailable("文档块分页标记缺失"))?.to_owned())
    } else { None };
    Ok(NativeDocumentPage { blocks, next_cursor })
}

fn children(block: &Value) -> Result<Vec<&str>, HubError> {
    match block.get("children") {
        None => Ok(Vec::new()),
        Some(Value::Array(children)) => children.iter().map(|child| child.as_str().filter(|id| !id.is_empty())
            .ok_or_else(|| HubError::unavailable("文档块子节点关系不完整"))).collect(),
        _ => Err(HubError::unavailable("文档块子节点关系不完整")),
    }
}
fn text_payload(block: &Value) -> Option<&Value> {
    let kind = block.get("block_type")?.as_u64()?;
    block.get(match kind { 1 => "page", 2 => "text", 3 => "heading1", 4 => "heading2", 5 => "heading3",
        6 => "heading4", 7 => "heading5", 8 => "heading6", 9 => "heading7", 10 => "heading8", 11 => "heading9",
        12 => "bullet", 13 => "ordered", 14 => "code", 15 => "quote", 17 => "todo", _ => return None })
}
fn plain_content(block: &Value) -> String {
    text_payload(block).and_then(|text| text.get("elements")).and_then(Value::as_array)
        .map(|elements| elements.iter().map(|element| {
            element.pointer("/text_run/content").and_then(Value::as_str).map(str::to_owned)
                .unwrap_or_else(|| "[此内容请在飞书查看]".into())
        }).collect::<String>()).unwrap_or_default()
}
fn paragraph_reason(block: &Value) -> Option<DocumentReadonlyReason> {
    use DocumentReadonlyReason::*;
    if block.get("block_type").and_then(Value::as_u64) != Some(2) { return Some(Structure); }
    match children(block) { Ok(children) if children.is_empty() => {}, _ => return Some(Structure) }
    if block.as_object().is_none_or(|block| block.keys().any(|key| !matches!(key.as_str(), "block_id" | "block_type" | "parent_id" | "children" | "text"))) { return Some(Metadata); }
    let Some(payload) = block.get("text").and_then(Value::as_object) else { return Some(Metadata); };
    if payload.keys().any(|key| !matches!(key.as_str(), "elements" | "style")) { return Some(Metadata); }
    let Some(elements) = payload.get("elements") else { return None; }; // Native empty paragraphs omit elements.
    let Some(elements) = elements.as_array() else { return Some(Metadata); };
    for element in elements {
        let Some(element) = element.as_object() else { return Some(Metadata); };
        if element.len() != 1 || !element.contains_key("text_run") { return Some(RichText); }
        let Some(run) = element.get("text_run").and_then(Value::as_object) else { return Some(Metadata); };
        if !run.get("content").is_some_and(Value::is_string) || run.keys().any(|key| !matches!(key.as_str(), "content" | "text_element_style")) { return Some(Metadata); }
        if let Some(style) = run.get("text_element_style") {
            let Some(style) = style.as_object() else { return Some(Metadata); };
            for (key, value) in style {
                if key == "comment_ids" {
                    if !value.as_array().is_some_and(Vec::is_empty) { return Some(Metadata); }
                } else if matches!(key.as_str(), "bold" | "italic" | "strikethrough" | "underline" | "inline_code") {
                    if value != &Value::Bool(false) { return Some(RichText); }
                } else { return Some(RichText); }
            }
        }
    }
    None
}

pub fn normalize_document_edit(document_input: &str, metadata: &NativeDocumentMetadata, native_blocks: &[Value]) -> Result<OfficeQueryResult, HubError> {
    native_id(&metadata.id)?;
    native_revision(&metadata.revision)?;
    if native_blocks.is_empty() || native_blocks.len() > 10_000 { return Err(HubError::unavailable("文档块数量不完整或超出编辑范围")); }
    let mut lookup = BTreeMap::new();
    for block in native_blocks {
        let id = block.get("block_id").and_then(Value::as_str).ok_or_else(|| HubError::unavailable("文档块缺少原生标识"))?;
        native_id(id)?;
        if !block.get("block_type").is_some_and(Value::is_u64) || lookup.insert(id, block).is_some() {
            return Err(HubError::unavailable("文档块类型不完整或标识重复"));
        }
    }
    let root = lookup.get(metadata.id.as_str()).ok_or_else(|| HubError::unavailable("文档根节点缺失"))?;
    if root.get("block_type").and_then(Value::as_u64) != Some(1) || root.get("parent_id").and_then(Value::as_str).is_some_and(|id| !id.is_empty()) {
        return Err(HubError::unavailable("文档根节点与基本信息不一致"));
    }
    let mut visited = BTreeSet::new();
    let mut stack = vec![(metadata.id.as_str(), 0usize)];
    while let Some((id, depth)) = stack.pop() {
        if depth > 64 || !visited.insert(id) { return Err(HubError::unavailable("文档块关系重复或过深")); }
        let block = lookup.get(id).ok_or_else(|| HubError::unavailable("文档块未完整返回"))?;
        for child_id in children(block)?.into_iter().rev() {
            let child = lookup.get(child_id).ok_or_else(|| HubError::unavailable("文档子块未完整返回"))?;
            if child.get("parent_id").and_then(Value::as_str) != Some(id) { return Err(HubError::unavailable("文档块父子关系不一致")); }
            stack.push((child_id, depth + 1));
        }
    }
    if visited.len() != lookup.len() { return Err(HubError::unavailable("文档含未归属的块或不完整分页")); }
    let mut blocks = Vec::new();
    for id in children(root)? {
        let block = lookup[id];
        let kind = match block.get("block_type").and_then(Value::as_u64) { Some(2) => DocumentBlockKind::Paragraph, Some(3..=11) => DocumentBlockKind::Heading, _ => DocumentBlockKind::Unsupported };
        let mut readonly_reason = paragraph_reason(block);
        if kind != DocumentBlockKind::Paragraph { readonly_reason = Some(DocumentReadonlyReason::Structure); }
        let mut content = plain_content(block);
        let mut descendants: Vec<_> = children(block)?.into_iter().rev().collect();
        while let Some(child_id) = descendants.pop() {
            let child = lookup[child_id];
            let text = plain_content(child);
            if !text.is_empty() { if !content.is_empty() { content.push('\n'); } content.push_str(&text); }
            descendants.extend(children(child)?.into_iter().rev());
        }
        if kind == DocumentBlockKind::Unsupported && content.is_empty() { content = "[此内容请在飞书查看]".into(); }
        blocks.push(DocumentBlock { id: Some(id.into()), kind, content, editable: readonly_reason.is_none(), readonly_reason });
    }
    let content = blocks.iter().map(|block| block.content.as_str()).collect::<Vec<_>>().join("\n\n");
    Ok(OfficeQueryResult { document: Some(OfficeDocument { id: metadata.id.clone(), title: metadata.title.clone(), content,
        url: official_url(document_input), edit_baseline: Some(DocumentEditBaseline { revision: metadata.revision.clone(), can_insert_first: blocks.is_empty(), blocks }),
        pending_operation_id: None, pending_status: None }), ..OfficeQueryResult::default() })
}

pub(super) fn document_save_command(mutation: &OfficeMutation, client_token: &str, profile: Option<&str>) -> Result<CliCommand, HubError> {
    let OfficeMutation::DocumentSave { document_id, revision, block_id, content, .. } = mutation else { return Err(HubError::invalid("需要文档保存操作")); };
    native_id(document_id)?;
    valid_text(content, "文档段落", 100_000)?;
    if content.encode_utf16().count() > 100_000 { return Err(HubError::invalid("文档段落超过 100000 个 UTF-16 字符")); }
    let params = json!({"document_revision_id": native_revision(revision)?, "client_token": client_token, "user_id_type": "open_id"});
    let text = json!({"elements": [{"text_run": {"content": content}}]});
    let (method, path, body) = if let Some(block_id) = block_id {
        native_id(block_id)?;
        ("PATCH", format!("/open-apis/docx/v1/documents/{document_id}/blocks/{block_id}"), json!({"update_text_elements": text}))
    } else {
        ("POST", format!("/open-apis/docx/v1/documents/{document_id}/blocks/{document_id}/children"), json!({"index": 0, "children": [{"block_type": 2, "text": text}]}))
    };
    let mut command = user_command(["api".into(), method.into(), path, "--params".into(), params.to_string(),
        "--data".into(), format!("@{PAYLOAD_FILE}"), "--format".into(), "json".into()], profile, true)?;
    command.input_file = Some(serde_json::to_vec(&body).map_err(|_| HubError::invalid("文档段落无法编码"))?);
    Ok(command)
}
pub(super) fn document_save_confirmed(mutation: &OfficeMutation, data: &Value) -> bool {
    let OfficeMutation::DocumentSave { document_id, revision, block_id, content, .. } = mutation else { return false; };
    let Some(returned_revision) = data.get("document_revision_id").and_then(Value::as_u64) else { return false; };
    if returned_revision <= revision.parse::<u64>().unwrap_or(u64::MAX) || data.get("client_token").and_then(Value::as_str).is_none_or(str::is_empty) { return false; }
    let block = if let Some(id) = block_id {
        let Some(block) = data.get("block") else { return false; };
        if block.get("block_id").and_then(Value::as_str) != Some(id.as_str()) { return false; }
        block
    } else {
        let Some(children) = data.get("children").and_then(Value::as_array).filter(|children| children.len() == 1) else { return false; };
        if children[0].get("block_id").and_then(Value::as_str).is_none_or(str::is_empty) { return false; }
        &children[0]
    };
    block.get("parent_id").and_then(Value::as_str) == Some(document_id.as_str()) && paragraph_reason(block).is_none() && plain_content(block) == *content
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(data: Value) -> CliOutput {
        CliOutput::fixture(0, json!({"ok": true, "identity": "user", "data": data}).to_string(), "")
    }
    fn metadata() -> NativeDocumentMetadata { NativeDocumentMetadata { id: "doc_real".into(), title: "Title".into(), revision: "12".into() } }
    fn paragraph(id: &str, content: &str) -> Value {
        json!({"block_id":id,"block_type":2,"parent_id":"doc_real","text":{"elements":[{"text_run":{"content":content,"text_element_style":{}}}],"style":{"align":2}}})
    }
    fn snapshot(blocks: Vec<Value>) -> Vec<Value> {
        let ids: Vec<_> = blocks.iter().map(|block| block["block_id"].clone()).collect();
        let mut native = vec![json!({"block_id":"doc_real","block_type":1,"parent_id":"","children":ids,"page":{"elements":[{"text_run":{"content":"Title"}}]}})];
        native.extend(blocks);
        native
    }
    fn save(block_id: Option<&str>, content: &str) -> OfficeMutation {
        OfficeMutation::DocumentSave { client_item_id: "client".into(), document_id: "doc_real".into(), revision: "12".into(), block_id: block_id.map(str::to_owned), content: content.into() }
    }
    #[test]
    fn native_queries_pin_revision_and_require_complete_page_envelopes() {
        let meta = normalize_document_metadata(&output(json!({"document":{"document_id":"doc_real","revision_id":12,"title":"Title"}}))).unwrap();
        assert_eq!(meta.revision,"12");
        let command = document_blocks_command(&meta.id,&meta.revision,Some("next-page"),Some("work")).unwrap();
        assert!(command.args.windows(2).any(|args|args==["--profile","work"]));
        assert!(command.args.windows(2).any(|args|args==["--as","user"]));
        let params: Value = serde_json::from_str(&command.args[command.args.iter().position(|arg|arg=="--params").unwrap()+1]).unwrap();
        assert_eq!(params,json!({"document_revision_id":12,"page_size":500,"page_token":"next-page","user_id_type":"open_id"}));
        assert!(normalize_document_metadata(&output(json!({"document":{"document_id":"doc_real","title":"Title"}}))).is_err());
        assert!(normalize_document_blocks(&output(json!({"items":[],"has_more":true}))).is_err());
        assert!(normalize_document_blocks(&output(json!({"items":[]}))).is_err());
        assert_eq!(normalize_document_blocks(&output(json!({"items":[],"has_more":true,"page_token":"next"}))).unwrap().next_cursor.as_deref(),Some("next"));
        assert!(document_metadata_command("https://team.feishu.cn/docx/doc_real",None).is_err());
    }
    #[test]
    fn native_baseline_preserves_literal_text_and_paragraph_style_without_markup_conversion() {
        let literal = "# 3 * 5; C:\\Users\\test[1]; foo_bar; $100; ~; <tag>; &copy;\n    第二行";
        let mut block = paragraph("block_plain",literal);
        block["text"]["elements"][0]["text_run"]["text_element_style"] = json!({"bold":false,"italic":false,"comment_ids":[]});
        let document = normalize_document_edit("https://team.feishu.cn/docx/doc_real",&metadata(),&snapshot(vec![block])).unwrap().document.unwrap();
        assert_eq!(document.title,"Title");
        let baseline = document.edit_baseline.unwrap();
        assert_eq!(baseline.blocks[0].content,literal);
        assert!(baseline.blocks[0].editable);
        assert!(baseline.blocks[0].readonly_reason.is_none());
        assert!(!baseline.can_insert_first);
    }
    #[test]
    fn native_baseline_marks_rich_text_comments_and_nested_blocks_readonly() {
        let mut rich = paragraph("rich","Bold");
        rich["text"]["elements"][0]["text_run"]["text_element_style"] = json!({"bold":true});
        let mut comment = paragraph("comment","Discussion");
        comment["text"]["elements"][0]["text_run"]["text_element_style"] = json!({"comment_ids":["comment-real"]});
        let mut mention = paragraph("mention","");
        mention["text"]["elements"] = json!([{"mention_user":{"user_id":"ou_real"}}]);
        let mut nested = paragraph("parent","Parent");
        nested["children"] = json!(["child"]);
        let mut child = paragraph("child","Child"); child["parent_id"] = json!("parent");
        let mut blocks = snapshot(vec![rich,comment,mention,nested]); blocks.push(child);
        let baseline = normalize_document_edit("doc_real",&metadata(),&blocks).unwrap().document.unwrap().edit_baseline.unwrap();
        assert!(baseline.blocks.iter().all(|block|!block.editable));
        assert_eq!(baseline.blocks[0].readonly_reason,Some(DocumentReadonlyReason::RichText));
        assert_eq!(baseline.blocks[1].readonly_reason,Some(DocumentReadonlyReason::Metadata));
        assert_eq!(baseline.blocks[2].readonly_reason,Some(DocumentReadonlyReason::RichText));
        assert_eq!(baseline.blocks[3].readonly_reason,Some(DocumentReadonlyReason::Structure));
        assert_eq!(baseline.blocks[3].content,"Parent\nChild");
        assert_eq!(baseline.blocks.len(),4,"nested text never becomes a writable top-level paragraph");
    }
    #[test]
    fn native_baseline_rejects_incomplete_duplicate_and_disconnected_trees() {
        let native = snapshot(vec![paragraph("plain","Text")]);
        let mut missing = native.clone(); missing.pop();
        let mut duplicate = native.clone(); duplicate.push(native[1].clone());
        let mut wrong_parent = native.clone(); wrong_parent[1]["parent_id"] = json!("other");
        let mut disconnected = native.clone(); disconnected.push(paragraph("orphan","Text"));
        let mut repeat_child = native.clone(); repeat_child[0]["children"] = json!(["plain","plain"]);
        let mut cycle = native; cycle[1]["children"] = json!(["doc_real"]);
        for blocks in [missing,duplicate,wrong_parent,disconnected,repeat_child,cycle] {
            assert!(normalize_document_edit("doc_real",&metadata(),&blocks).is_err());
        }
    }
    #[test]
    fn native_empty_paragraph_and_empty_body_remain_distinct() {
        let empty = json!({"block_id":"empty","block_type":2,"parent_id":"doc_real","text":{"style":{}}});
        let baseline = normalize_document_edit("doc_real",&metadata(),&snapshot(vec![empty])).unwrap().document.unwrap().edit_baseline.unwrap();
        assert!(baseline.blocks[0].editable);
        assert_eq!(baseline.blocks[0].content,"");
        assert!(!baseline.can_insert_first);
        let baseline = normalize_document_edit("doc_real",&metadata(),&snapshot(vec![])).unwrap().document.unwrap().edit_baseline.unwrap();
        assert!(baseline.blocks.is_empty()); assert!(baseline.can_insert_first);
    }
    #[test]
    fn saves_update_one_existing_block_in_place_or_insert_the_first_native_child() {
        let literal = "# Heading\n- literal list\n<img src=\"token\"/>; C:\\Users\\test; **literal**";
        for block_id in [Some("block_plain"),None] {
            let command = mutation_command(&save(block_id,literal),"stable-client-token",Some("work")).unwrap();
            assert!(command.mutating);
            assert!(command.args.iter().any(|arg|arg==if block_id.is_some(){"PATCH"}else{"POST"}));
            assert!(!command.args.iter().any(|arg|matches!(arg.as_str(),"block_replace"|"overwrite"|"markdown")));
            assert!(command.args.windows(2).any(|args|args==["--data",&format!("@{PAYLOAD_FILE}")]));
            let body: Value = serde_json::from_slice(command.input_file.as_ref().unwrap()).unwrap();
            assert_eq!(if block_id.is_some(){&body["update_text_elements"]["elements"][0]["text_run"]["content"]}else{&body["children"][0]["text"]["elements"][0]["text_run"]["content"]},literal);
            assert!(!body.to_string().contains("style"),"in-place text updates never replace paragraph style");
            assert!(command.stdin.is_none());
        }
        for content in ["", "  ", "\0"] { assert!(mutation_command(&save(Some("block_plain"),content),"stable",None).is_err()); }
        assert!(mutation_command(&save(Some("block_plain"),&"😀".repeat(50_001)),"stable",None).is_err());
        assert!(mutation_command(&save(Some("-1"),"Text"),"stable",None).is_err());
    }
    #[test]
    fn only_exact_native_receipts_confirm_document_saves() {
        let mutation = save(Some("block_plain"),"Updated");
        let receipt = json!({"document_revision_id":13,"client_token":"stable","block":paragraph("block_plain","Updated")});
        assert_eq!(normalize_mutation(&mutation,&output(receipt.clone())).unwrap().status,ItemStatus::Succeeded);
        for field in ["document_revision_id","client_token","block"] {
            let mut broken = receipt.clone(); broken.as_object_mut().unwrap().remove(field);
            assert_eq!(normalize_mutation(&mutation,&output(broken)).unwrap().status,ItemStatus::Unknown);
        }
        let mut wrong = receipt.clone(); wrong["block"]["block_id"] = json!("other");
        let mut wrong_text = receipt.clone(); wrong_text["block"]["text"]["elements"][0]["text_run"]["content"] = json!("Different");
        let mut stale = receipt; stale["document_revision_id"] = json!(12);
        for broken in [wrong,wrong_text,stale,json!({"result":"success","document":{"revision_id":13}})] {
            assert_eq!(normalize_mutation(&mutation,&output(broken)).unwrap().status,ItemStatus::Unknown);
        }
        let insert = save(None,"First");
        assert_eq!(normalize_mutation(&insert,&output(json!({"document_revision_id":13,"client_token":"stable","children":[paragraph("new_block","First")]}))).unwrap().status,ItemStatus::Succeeded);
    }
    #[test]
    fn title_only_creation_and_optional_due_dates_keep_the_official_explicit_contract() {
        let create = OfficeMutation::DocumentCreate { client_item_id:"new".into(),title:"Empty".into(),content:String::new() };
        let command = mutation_command(&create,"key",None).unwrap();
        assert!(command.args.windows(2).any(|args|args==["--doc-format","xml"]));
        assert!(!command.args.iter().any(|arg|arg=="--content")); assert!(command.stdin.is_none());
        let task = OfficeMutation::TaskCreate { client_item_id:"task".into(),title:"Title".into(),assignee_id:"ou_owner".into(),assignee_name:None,due:None,tasklist_id:None };
        assert!(!mutation_command(&task,"key",None).unwrap().args.iter().any(|arg|arg=="--due"));
    }
}
