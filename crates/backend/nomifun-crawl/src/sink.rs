//! Where crawled pages land.
//!
//! Default target is the knowledge base, through `write_document` — the same
//! canonical path the agent write-back uses, so the inbox review policy applies
//! to crawled pages exactly as it does to agent writes.

use std::sync::Arc;

use nomifun_common::{CrawlJobId, KnowledgeBaseId};
use nomifun_knowledge::service::{
    KnowledgeService, WriteMode, WritePolicy, WriteRequest, WriteSurface, WriteTargetSpec,
};
use nomifun_knowledge::source_url::slug_for_url;
use url::Url;

use crate::error::CrawlError;
use crate::model::CrawlJob;

/// Prefix of the per-job inbox scope. Not a scope by itself: a shared scope
/// would put every job's pages in one undifferentiated review pile.
pub const CRAWL_INBOX_SCOPE_PREFIX: &str = "crawl-";

/// Root directory (inside the base) that the crawler owns. The URL-source
/// snapshots own `snapshots/` and are rebuilt wholesale from their entries, so
/// the two must never share a directory.
pub const CRAWL_REL_DIR: &str = "crawl";

/// How much of the job id is folded into the directory name.
const JOB_ID_SUFFIX_LEN: usize = 8;

/// The *tail* of the job id. A UUIDv7's leading hex digits are the millisecond
/// timestamp, whose top 32 bits only change every ~65s — a head slice would
/// hand two jobs created in the same minute the same directory, which is the
/// collision this suffix exists to prevent. The tail is the random half.
fn id_suffix(job_id: &CrawlJobId) -> &str {
    let raw = job_id.as_str();
    raw.get(raw.len().saturating_sub(JOB_ID_SUFFIX_LEN)..).unwrap_or(raw)
}

#[derive(Debug, Clone)]
pub struct IngestPage {
    pub url: String,
    pub title: Option<String>,
    pub markdown: String,
}

/// Result of persisting one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestReceipt {
    pub rel_path: String,
    pub staged: bool,
}

#[async_trait::async_trait]
pub trait CrawlSinkWriter: Send + Sync {
    /// Persist one page. `Ok(None)` means the job has no configured target and
    /// the content was intentionally dropped.
    async fn write(&self, job: &CrawlJob, page: &IngestPage)
    -> Result<Option<IngestReceipt>, CrawlError>;
}

pub struct KnowledgeSink {
    service: Arc<KnowledgeService>,
}

impl KnowledgeSink {
    pub fn new(service: Arc<KnowledgeService>) -> Self {
        Self { service }
    }
}

#[async_trait::async_trait]
impl CrawlSinkWriter for KnowledgeSink {
    async fn write(
        &self,
        job: &CrawlJob,
        page: &IngestPage,
    ) -> Result<Option<IngestReceipt>, CrawlError> {
        let Some(raw_kb_id) = job.sink.knowledge_base_id.clone() else {
            return Ok(None);
        };
        let kb_id = KnowledgeBaseId::parse(raw_kb_id)
            .map_err(|e| CrawlError::UrlRejected(format!("invalid knowledge base id: {e}")))?;
        let rel_path = document_path(&job.job_id, &job.name, &page.url);
        let mode = if job.sink.via_inbox {
            WriteMode::Staged { scope: inbox_scope(&job.job_id) }
        } else {
            WriteMode::Direct
        };
        let request = WriteRequest {
            spec: WriteTargetSpec::Path { kb_id: kb_id.clone(), rel_path },
            content: render_document(page),
            policy: WritePolicy {
                mode,
                allow_create: true,
                surface: WriteSurface::RegularChat,
            },
            bound_kb_ids: vec![kb_id],
        };
        let outcome = self.service.write_document(request).await?;
        Ok(Some(IngestReceipt {
            rel_path: outcome.final_rel_path,
            staged: outcome.staged,
        }))
    }
}

/// Staging namespace for one job. Per-job rather than shared so the review
/// panel can group (and later bulk-accept) by job.
pub fn inbox_scope(job_id: &CrawlJobId) -> String {
    format!("{CRAWL_INBOX_SCOPE_PREFIX}{job_id}")
}

/// `crawl/{job}-{id8}/{page}.md`. The id suffix is what actually separates two
/// crawls of the same site — the name slug alone collides whenever two jobs
/// share a name, or differ only past the slug's length cap.
pub fn document_path(job_id: &CrawlJobId, job_name: &str, url: &str) -> String {
    let page = Url::parse(url)
        .map(|u| slug_for_url(&u))
        .unwrap_or_else(|_| "page".to_string());
    format!("{CRAWL_REL_DIR}/{}-{}/{page}.md", slugify(job_name), id_suffix(job_id))
}

/// Front matter carries provenance so a reviewer reading the inbox diff can
/// tell where the text came from without opening the crawl UI.
fn render_document(page: &IngestPage) -> String {
    let title = page.title.clone().unwrap_or_else(|| page.url.clone());
    format!(
        "---\ntitle: {}\nsource_url: {}\nsource: nomifun-crawl\n---\n\n{}\n",
        yaml_scalar(&title),
        yaml_scalar(&page.url),
        page.markdown.trim()
    )
}

/// Quote anything that YAML would otherwise reinterpret.
fn yaml_scalar(value: &str) -> String {
    let cleaned = value.replace(['\n', '\r'], " ");
    format!("\"{}\"", cleaned.replace('\\', "\\\\").replace('"', "\\\""))
}

fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 60 {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() { "job".to_string() } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_groups_by_job_and_slugs_the_url() {
        let job_id = CrawlJobId::new();
        let path = document_path(&job_id, "My Site Crawl", "https://example.com/docs/intro");
        assert!(path.starts_with(&format!("crawl/my-site-crawl-{}/", id_suffix(&job_id))), "{path}");
        assert!(path.ends_with(".md"), "{path}");
    }

    #[test]
    fn distinct_urls_get_distinct_paths() {
        let job_id = CrawlJobId::new();
        let a = document_path(&job_id, "j", "https://example.com/a");
        let b = document_path(&job_id, "j", "https://example.com/b");
        assert_ne!(a, b);
    }

    /// The whole point of grouping by job: without the id suffix two jobs
    /// sharing a name (or a name that only differs past the slug cap) write
    /// the same file and silently overwrite each other. Ids minted back to
    /// back are the hard case — a UUIDv7 prefix would still be identical.
    #[test]
    fn same_named_jobs_do_not_share_a_directory() {
        let url = "https://example.com/a";
        let a = document_path(&CrawlJobId::new(), "Docs", url);
        let b = document_path(&CrawlJobId::new(), "Docs", url);
        assert_ne!(a, b);
    }

    #[test]
    fn unparseable_url_still_yields_a_path() {
        let job_id = CrawlJobId::new();
        assert_eq!(
            document_path(&job_id, "j", "not a url"),
            format!("crawl/j-{}/page.md", id_suffix(&job_id))
        );
    }

    #[test]
    fn job_name_of_only_symbols_falls_back() {
        let job_id = CrawlJobId::new();
        let path = document_path(&job_id, "!!!", "https://e.com/x");
        assert_eq!(path.split('/').nth(1), Some(format!("job-{}", id_suffix(&job_id)).as_str()));
    }

    /// `validate_inbox_scope` requires a single portable path component, so a
    /// separator or a Windows-illegal character here would reject every staged
    /// write at runtime rather than at compile time.
    #[test]
    fn inbox_scope_is_a_single_portable_component() {
        let scope = inbox_scope(&CrawlJobId::new());
        assert!(scope.starts_with(CRAWL_INBOX_SCOPE_PREFIX), "{scope}");
        assert!(
            !scope.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*']),
            "{scope}"
        );
        assert!(!scope.ends_with([' ', '.']), "{scope}");
    }

    /// The staged path is `_inbox/{scope}/{rel_path}`; a scope equal to the
    /// rel_path's own root produced `_inbox/crawl/crawl/…`.
    #[test]
    fn inbox_scope_does_not_repeat_the_document_root() {
        assert_ne!(inbox_scope(&CrawlJobId::new()), CRAWL_REL_DIR);
    }

    #[test]
    fn front_matter_escapes_quotes_and_newlines() {
        let doc = render_document(&IngestPage {
            url: "https://e.com/x".into(),
            title: Some("A \"quoted\"\ntitle".into()),
            markdown: "body".into(),
        });
        assert!(doc.contains(r#"title: "A \"quoted\" title""#), "{doc}");
        assert!(doc.contains("source_url: \"https://e.com/x\""));
    }

    #[test]
    fn missing_title_falls_back_to_the_url() {
        let doc = render_document(&IngestPage {
            url: "https://e.com/x".into(),
            title: None,
            markdown: "body".into(),
        });
        assert!(doc.contains(r#"title: "https://e.com/x""#), "{doc}");
    }
}
