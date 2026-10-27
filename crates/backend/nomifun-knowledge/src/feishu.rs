//! Feishu remote-backed knowledge surface.
//!
//! A Feishu knowledge base is an ordinary registered base whose `extra.source`
//! carries `kind: "feishu"`. Its managed entries are metadata-only stubs — the
//! document body is never persisted; `knowledge/read` fetches it live through
//! the injected [`FeishuKnowledgePort`]. Search is remote Search v2 recall
//! intersected with the curated token set derived from source items.
//!
//! This crate deliberately knows nothing about lark-cli, profiles, or device
//! flows; the application composition wires the office-hub adapter.

use nomifun_common::{AppError, KnowledgeBaseId, KnowledgeSourceItemId};
use url::Url;

/// Persisted source kind discriminator for Feishu-backed bases.
pub const FEISHU_SOURCE_KIND: &str = "feishu";
/// Pseudo rel-path namespace for remote Feishu documents inside a `kdoc_`
/// handle. It can never collide with a real file: `.`-prefixed components are
/// excluded from the tree, search walk, and write policy.
pub const FEISHU_REMOTE_PREFIX: &str = ".remote/feishu";
/// Curated Feishu entries per base. Each one costs a metadata call at index
/// refresh, so the cap stays comfortably inside a bounded mount-time budget.
pub const MAX_FEISHU_SOURCE_ENTRIES: usize = 64;

/// One curated/remote document identity shared by index refresh and Search v2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuIndexHit {
    /// Remote token as addressed by the curated link — the wiki node_token
    /// for wiki sources, the doc/docx token otherwise.
    pub token: String,
    pub kind: String,
    pub title: String,
    pub url: Option<String>,
    pub modified_at: Option<String>,
    /// Search v2 summary highlight (plain text), empty for inspect results.
    pub summary: Option<String>,
}

/// Live-fetched document body (never persisted by this crate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeishuDocContent {
    /// Canonical object token returned by the fetch (wiki links unwrap here).
    pub document_id: String,
    pub markdown: String,
    pub revision_id: Option<i64>,
}

/// Consumer-owned port for Feishu knowledge reads. Implementations dispatch
/// through the supervised office-hub connection; Knowledge never sees a CLI,
/// profile, or credential.
#[async_trait::async_trait]
pub trait FeishuKnowledgePort: Send + Sync {
    /// Live Search v2 against the connected tenant (doc/docx/wiki only).
    async fn search_documents(&self, query: &str) -> Result<Vec<FeishuIndexHit>, AppError>;
    /// Metadata-only resolve of one official document link (index refresh).
    async fn inspect_link(&self, url: &str) -> Result<FeishuIndexHit, AppError>;
    /// Live markdown body fetch for an official link or bare token.
    async fn fetch_document(&self, target: &str) -> Result<FeishuDocContent, AppError>;
}

/// Default port used until the composition root wires the office hub. It is
/// deliberately an error, never a synthetic empty result.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableFeishuKnowledgePort;

#[async_trait::async_trait]
impl FeishuKnowledgePort for UnavailableFeishuKnowledgePort {
    async fn search_documents(&self, _query: &str) -> Result<Vec<FeishuIndexHit>, AppError> {
        Err(unavailable())
    }
    async fn inspect_link(&self, _url: &str) -> Result<FeishuIndexHit, AppError> {
        Err(unavailable())
    }
    async fn fetch_document(&self, _target: &str) -> Result<FeishuDocContent, AppError> {
        Err(unavailable())
    }
}

fn unavailable() -> AppError {
    AppError::Conflict(
        "Feishu knowledge access is unavailable; connect and authorize the Feishu office integration first"
            .into(),
    )
}

/// Extract `(kind, token)` from an official Feishu/Lark document URL.
/// Phase one accepts `doc`, `docx` and `wiki` only; everything else returns
/// `None` so callers reject rather than silently misclassify.
pub fn feishu_link_target(url: &str) -> Option<(String, String)> {
    let url = official_feishu_url(url)?;
    let parsed = Url::parse(&url).ok()?;
    let mut parts = parsed.path().trim_matches('/').split('/');
    let (kind, token) = (parts.next()?, parts.next()?);
    if parts.next().is_some() || token.is_empty() {
        return None;
    }
    let kind = match kind {
        "docs" => "doc",
        "sheets" => "sheet",
        "base" => "bitable",
        other => other,
    };
    if !matches!(kind, "doc" | "docx" | "wiki") || !is_resource_token(token) {
        return None;
    }
    Some((kind.to_owned(), token.to_owned()))
}

/// Keep the exact URL after verifying an official HTTPS Feishu/Lark host.
/// Mirrors the office-hub policy: no userinfo, no non-HTTPS, no encoded hosts.
pub fn official_feishu_url(value: &str) -> Option<String> {
    let remaining = value.strip_prefix("https://")?;
    let authority = remaining.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') || value.contains(['\\', '\r', '\n']) {
        return None;
    }
    let host = authority.strip_suffix(":443").unwrap_or(authority);
    if !host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-') {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let allowed = ["feishu.cn", "larksuite.com", "larkoffice.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    allowed.then(|| value.to_owned())
}

pub fn is_resource_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 512
        && token.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

/// Pseudo rel-path stored inside the opaque `kdoc_` handle for a curated token.
pub fn remote_rel_path(token: &str) -> String {
    format!("{FEISHU_REMOTE_PREFIX}/{token}")
}

/// Reverse of [`remote_rel_path`]; `None` for any non-remote rel path.
pub fn remote_token(rel_path: &str) -> Option<&str> {
    rel_path
        .strip_prefix(FEISHU_REMOTE_PREFIX)
        .map(|rest| rest.strip_prefix('/').unwrap_or(rest))
        .filter(|token| is_resource_token(token))
}

/// Assemble a stub document: the managed-source identity markers followed by
/// a metadata index card. The stub never carries the remote body; it points
/// the reader at `knowledge_read` with the stable handle.
pub fn feishu_stub_markdown(
    source_item_id: &KnowledgeSourceItemId,
    source_url: &str,
    knowledge_base_id: &KnowledgeBaseId,
    meta: &FeishuIndexHit,
    refreshed_at: &str,
) -> String {
    let handle = crate::service::encode_doc_handle(
        knowledge_base_id,
        &remote_rel_path(&meta.token),
    );
    let title = meta.title.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = if title.is_empty() { meta.token.as_str() } else { title.as_str() };
    let mut out = format!(
        "> **source_url**: {source_url}\n> **fetched_at**: {refreshed_at}\n\
         > **nomifun_source_item_id**: {source_item_id}\n\
         > **nomifun_source_relationship**: managed\n\
         > **nomifun_remote_backend**: feishu\n"
    );
    out.push_str(&format!("> **title**: \"{}\"\n", title.replace('"', "'")));
    out.push_str(&format!("> **kind**: {}\n", meta.kind));
    if let Some(modified_at) = meta.modified_at.as_deref().filter(|s| !s.is_empty()) {
        out.push_str(&format!("> **remote_modified_at**: {modified_at}\n"));
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(
        "Feishu document index entry (metadata only — the body is fetched live).\n\n",
    );
    out.push_str(&format!("- Source: {source_url}\n"));
    out.push_str(&format!("- Handle: `{handle}`\n\n"));
    out.push_str(
        "Call `knowledge_read` with the handle above to fetch the live document body.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feishu_links_parse_only_official_doc_kinds() {
        for (url, want) in [
            ("https://t.feishu.cn/docx/Tok_1-Ab", ("docx", "Tok_1-Ab")),
            ("https://t.feishu.cn/docs/Tok2", ("doc", "Tok2")),
            ("https://t.feishu.cn/wiki/Node_9?x=1", ("wiki", "Node_9")),
            ("https://t.larksuite.com/docx/Tok", ("docx", "Tok")),
        ] {
            assert_eq!(
                feishu_link_target(url),
                Some((want.0.to_owned(), want.1.to_owned())),
                "{url}"
            );
        }
        for bad in [
            "https://t.feishu.cn/sheets/SHT1",
            "https://t.feishu.cn/base/B1",
            "http://t.feishu.cn/docx/x",
            "https://evil.feishu.cn.evil.com/docx/x",
            "https://t.feishu.cn/docx/",
            "https://t.feishu.cn/docx/a/b",
            "not a url",
        ] {
            assert!(feishu_link_target(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn remote_paths_round_trip_and_reject_foreign_tokens() {
        let path = remote_rel_path("Tok_1-Ab");
        assert_eq!(remote_token(&path), Some("Tok_1-Ab"));
        assert!(remote_token("docs/a.md").is_none());
        assert!(remote_token(".remote/feishu/").is_none());
        assert!(remote_token(".remote/feishu/bad token").is_none());
        assert!(remote_token(".remote/feishu-x/t").is_none());
    }

    #[test]
    fn stub_marks_identity_and_points_at_knowledge_read() {
        let meta = FeishuIndexHit {
            token: "Tok_1".into(),
            kind: "docx".into(),
            title: "产品 规划".into(),
            url: Some("https://t.feishu.cn/docx/Tok_1".into()),
            modified_at: Some("2026-10-26T23:31:50+08:00".into()),
            summary: None,
        };
        let item = KnowledgeSourceItemId::new();
        let kb = KnowledgeBaseId::new();
        let stub = feishu_stub_markdown(&item, "https://t.feishu.cn/docx/Tok_1", &kb, &meta, "2026-10-27T00:00:00Z");
        assert!(stub.contains(&format!("nomifun_source_item_id**: {item}")));
        assert!(stub.contains("nomifun_remote_backend**: feishu"));
        assert!(stub.contains("# 产品 规划"));
        assert!(stub.contains("knowledge_read"));
        let handle = crate::service::encode_doc_handle(&kb, &remote_rel_path("Tok_1"));
        assert!(stub.contains(&handle));
        assert!(crate::source_url::snapshot_source_item_id(&stub).as_ref() == Some(&item));
    }

    #[tokio::test]
    async fn default_port_fails_closed_with_actionable_error() {
        let port = UnavailableFeishuKnowledgePort;
        for result in [
            port.search_documents("q").await.map(|_| ()),
            port.inspect_link("https://t.feishu.cn/docx/x").await.map(|_| ()),
            port.fetch_document("x").await.map(|_| ()),
        ] {
            let error = result.unwrap_err();
            assert!(matches!(error, AppError::Conflict(_)), "{error}");
            assert!(!format!("{error}").is_empty());
        }
    }
}
