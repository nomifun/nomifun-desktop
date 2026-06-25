//! Production [`PageFetcher`]: a **rendering** page-fetch backend for knowledge
//! URL sources, backed by the in-process self-hosted CDP browser engine
//! (`nomi-browser-engine`). Same late-wire layering as [`crate::knowledge_completer`]:
//! the trait ([`nomifun_knowledge::PageFetcher`]) lives in the knowledge crate,
//! this is the engine-backed implementation, and the app layer wires it via
//! `KnowledgeService::with_url_fetcher` (P3 task K2, anti-cycle decision ②).
//!
//! **Why a browser backend at all** — the default [`nomifun_knowledge::HttpFetcher`]
//! does a plain reqwest GET, so a JS-rendered SPA (whose body is injected by
//! client-side scripts) yields an empty/skeletal snapshot. `BrowserFetcher`
//! navigates a real Chromium, lets the page settle + run its JS, then reads the
//! **rendered** DOM.
//!
//! **X6 (engine-product → clean markdown)** — the engine's `act(GetPageText)` /
//! `act(Extract)` products are deliberately LLM-facing: they `redact` page
//! secrets (lossily/irreversibly), wrap text in `<data origin=…>` anti-injection
//! delimiters, and prefix a human note. Feeding that into a knowledge snapshot
//! would corrupt it (redaction placeholders + `<data>` tags + lost title/markdown
//! structure). So `BrowserFetcher` does NOT route through `act`; it reads the raw
//! rendered HTML via [`BrowserEngine::rendered_html`] and runs it through the
//! knowledge crate's own [`html_to_markdown`] converter — the **identical**
//! pipeline `HttpFetcher` uses, just sourced from the post-JS DOM. Result: clean
//! markdown + title, byte-for-byte the same snapshot shape as HTTP sources.
//!
//! **Concurrency (decision ⑪)** — knowledge fetching runs `buffer_unordered(4)`,
//! but `BrowserEngine` is `is_concurrency_safe == false` (one page-session,
//! observe ⊥ act, per-target serial). So `BrowserFetcher` holds **one** lazily
//! constructed engine behind a [`tokio::sync::Mutex`] and **serializes** every
//! `fetch_page` on it. Spinning up a fresh Chromium per fetch would be correct but
//! pay a process-launch + CDP-connect cost (seconds) on every URL in the batch;
//! one shared, serialized engine is the cheap-and-correct choice (same lazy-engine
//! discipline as `nomi_browser::BrowserTool`).
//!
//! **Egress policy (decision ⑩)** — the engine is built with a `FirewallConfig`
//! that keeps the SSRF/IP-block guard ON (knowledge fetching must not be a pivot
//! into private networks). The per-pet domain allowlist (`allow_etld1`) is a
//! later task (X2, populated from the secret vault); K2 ships the IP-block default
//! so the rendering path is governed, not wide open.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_common::AppError;
use nomifun_knowledge::PageFetcher;
use nomifun_knowledge::source_url::{FETCH_MAX_BYTES, FetchedPage, html_to_markdown, truncate_to_bytes};
use tokio::sync::Mutex;

use nomi_browser_engine::firewall::FirewallConfig;
use nomi_browser_engine::{BrowserEngine, EngineConfig, create_engine};

/// The minimal engine surface `BrowserFetcher` needs: navigate to a URL and read
/// back where it settled + the rendered HTML. Abstracted behind a trait so the
/// serialization + conversion logic is unit-testable without launching Chromium
/// (the real impl wraps `Arc<dyn BrowserEngine>`; tests use a canned in-memory
/// backend). `dyn`-friendly (async_trait) and `Send + Sync` for the shared lock.
#[async_trait]
trait RenderBackend: Send + Sync {
    /// Navigate to `raw_url` (letting the page settle + run its JS), then return
    /// `(final_url_after_redirects, rendered_html)`.
    async fn navigate_and_render(&self, raw_url: &str) -> Result<(String, String), AppError>;
}

/// `RenderBackend` over the real CDP engine. Navigates on the engine's single
/// page-session, then reads `document.documentElement.outerHTML`.
struct EngineRenderBackend {
    engine: Arc<dyn BrowserEngine>,
}

#[async_trait]
impl RenderBackend for EngineRenderBackend {
    async fn navigate_and_render(&self, raw_url: &str) -> Result<(String, String), AppError> {
        // navigate settles to a load state (`NavResult`); `new_tab=false` keeps the
        // single page-session (BrowserFetcher owns this engine, no tab juggling).
        let nav = self
            .engine
            .navigate(raw_url, false)
            .await
            .map_err(|e| AppError::BadGateway(format!("browser navigate failed for {raw_url}: {e}")))?;
        let html = self
            .engine
            .rendered_html()
            .await
            .map_err(|e| AppError::BadGateway(format!("reading rendered HTML failed for {raw_url}: {e}")))?;
        Ok((nav.final_url, html))
    }
}

/// Rendering page-fetch backend: navigate a managed Chromium, read the post-JS
/// DOM, convert it to markdown via the knowledge crate's own pipeline.
///
/// Holds the engine lazily (constructed on first fetch, cached — including a
/// construction *failure*, so an unavailable browser is reported without
/// relaunching per URL) and a [`Mutex`] over the render backend that serializes
/// concurrent `fetch_page` calls (decision ⑪).
pub struct BrowserFetcher {
    /// Engine data dir (managed-chrome download fallback + dedicated user-data-dir
    /// parent). Never the user's real browser profile.
    data_dir: PathBuf,
    /// The lazily-built render backend. `None` ⇒ not built yet; `Some(Err)` caches
    /// an unavailable engine. Behind a `Mutex` so (a) construction is one-shot and
    /// (b) every fetch is serialized on the non-concurrency-safe engine.
    backend: Mutex<Option<Result<Arc<dyn RenderBackend>, String>>>,
}

impl BrowserFetcher {
    /// Construct with the engine data directory. Does NOT launch a browser; the
    /// engine is built (and any failure cached) on the first `fetch_page`.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            backend: Mutex::new(None),
        }
    }

    /// `EngineConfig` for the knowledge-fetch engine: dedicated data dir, headless,
    /// SSRF/IP-block firewall ON (decision ⑩). No per-pet workspace/context — this
    /// engine is a shared knowledge-ingestion fetcher, not a session tool, so
    /// downloads (irrelevant here, we only read DOM) fall back under `data_dir` and
    /// there is no storage_state.
    fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            data_dir: self.data_dir.clone(),
            // Knowledge fetching is SSRF-sensitive: keep the IP-block guard on so a
            // rendered fetch can't be steered into a private/metadata endpoint. The
            // per-pet domain allowlist (allow_etld1) is X2's job; default() keeps
            // block_private_ips=true + gate_cross_origin_post=true.
            firewall: FirewallConfig::default(),
            ..EngineConfig::default()
        }
    }

    /// Lazily build (and cache) the render backend. The error string is cached too,
    /// so an unavailable engine is reported without relaunching per call. Must be
    /// invoked while holding the `backend` lock (it mutates the cached slot).
    async fn ensure_backend(
        slot: &mut Option<Result<Arc<dyn RenderBackend>, String>>,
        config: EngineConfig,
    ) -> Result<Arc<dyn RenderBackend>, AppError> {
        if let Some(cached) = slot.as_ref() {
            return cached
                .clone()
                .map_err(|msg| AppError::BadGateway(format!("browser engine unavailable: {msg}")));
        }
        let built = create_engine(config).await.map(|engine| {
            Arc::new(EngineRenderBackend { engine }) as Arc<dyn RenderBackend>
        });
        let stored = built.map_err(|e| e.to_string());
        *slot = Some(stored.clone());
        stored.map_err(|msg| AppError::BadGateway(format!("browser engine unavailable: {msg}")))
    }
}

#[async_trait]
impl PageFetcher for BrowserFetcher {
    async fn fetch_page(&self, raw_url: &str) -> Result<FetchedPage, AppError> {
        // Hold the lock for the whole fetch: this both (a) builds the engine once
        // and (b) serializes fetches on the non-concurrency-safe engine (⑪). The
        // `buffer_unordered(4)` callers above queue here rather than racing the
        // single page-session.
        let mut guard = self.backend.lock().await;
        let backend = Self::ensure_backend(&mut guard, self.engine_config()).await?;
        let (final_url, html) = backend.navigate_and_render(raw_url).await?;
        Ok(rendered_to_page(&final_url, &html))
    }
}

/// **[纯逻辑] 渲染后 HTML + 落点 URL → [`FetchedPage`]**（不进浏览器，便于单测）。
///
/// 走 K1 **同一条** [`html_to_markdown`] 管线（title + markdown），故快照形态与 HTTP 源逐字一致
/// （无 `<data>`/脱敏占位污染）。markdown 超 [`FETCH_MAX_BYTES`] 时按字符边界截断并置 `truncated`
/// （镜像 `HttpFetcher` 的 body 截断语义；下游 `prepare_snapshot_body` 再做压缩/落盘）。
fn rendered_to_page(final_url: &str, html: &str) -> FetchedPage {
    let (title, markdown) = html_to_markdown(html);
    let truncated = markdown.len() > FETCH_MAX_BYTES;
    let markdown = if truncated {
        truncate_to_bytes(&markdown, FETCH_MAX_BYTES).to_string()
    } else {
        markdown
    };
    FetchedPage {
        final_url: final_url.to_string(),
        title,
        markdown,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── [纯逻辑] rendered_to_page：HTML→markdown 映射 / title / 截断 ────────────

    #[test]
    fn rendered_to_page_converts_html_like_the_http_pipeline() {
        // JS-rendered markup (a real SPA's post-JS DOM). Converts via the SAME
        // html_to_markdown K1's HttpFetcher uses → clean title + markdown, NOT the
        // `<data>`-wrapped / redacted act(GetPageText) product.
        let html = "<html><head><title>渲染后标题</title><script>noise()</script></head>\
                    <body><h1>动态正文</h1><p>只有 <b>浏览器</b> 才看得到</p></body></html>";
        let page = rendered_to_page("https://spa.example.com/app", html);
        assert_eq!(page.title.as_deref(), Some("渲染后标题"));
        assert!(page.markdown.contains("# 动态正文"), "got: {}", page.markdown);
        assert!(page.markdown.contains("只有"), "body text missing: {}", page.markdown);
        assert!(page.markdown.contains("**浏览器**"), "got: {}", page.markdown);
        // No LLM-safety transforms leak into the snapshot.
        assert!(!page.markdown.contains("<data"), "leaked <data> wrap: {}", page.markdown);
        assert!(!page.markdown.contains("[REDACTED"), "leaked redaction: {}", page.markdown);
        assert!(!page.markdown.contains("noise()"), "script content leaked: {}", page.markdown);
        assert_eq!(page.final_url, "https://spa.example.com/app");
        assert!(!page.truncated);
    }

    #[test]
    fn rendered_to_page_truncates_oversized_markdown() {
        // A rendered body larger than FETCH_MAX_BYTES is cut (not failed), flagged
        // truncated — mirrors HttpFetcher's body-cap semantics.
        let big = "x".repeat(FETCH_MAX_BYTES + 10_000);
        let html = format!("<html><body><p>{big}</p></body></html>");
        let page = rendered_to_page("https://e.com", &html);
        assert!(page.truncated, "oversized markdown must be flagged truncated");
        assert!(page.markdown.len() <= FETCH_MAX_BYTES, "len={}", page.markdown.len());
    }

    #[test]
    fn rendered_to_page_handles_missing_title() {
        let page = rendered_to_page("https://e.com", "<html><body><p>no title here</p></body></html>");
        assert!(page.title.is_none(), "got: {:?}", page.title);
        assert!(page.markdown.contains("no title here"));
        assert!(!page.truncated);
    }

    // ── [纯逻辑] 串行化 + 懒构造（fake RenderBackend，不进浏览器）────────────────

    /// A canned `RenderBackend` that counts overlapping calls — proves
    /// `fetch_page` serializes on the Mutex (decision ⑪) even under concurrent
    /// callers, and never launches Chromium in tests.
    struct CountingBackend {
        in_flight: std::sync::atomic::AtomicUsize,
        max_in_flight: std::sync::atomic::AtomicUsize,
        html: String,
    }

    #[async_trait]
    impl RenderBackend for CountingBackend {
        async fn navigate_and_render(&self, raw_url: &str) -> Result<(String, String), AppError> {
            use std::sync::atomic::Ordering;
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            // Yield so a second task would interleave here IF not serialized.
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok((raw_url.to_string(), self.html.clone()))
        }
    }

    /// Build a `BrowserFetcher` whose backend slot is pre-seeded with a canned
    /// backend (so no Chromium launch, no `ensure_backend` create_engine).
    fn seeded_fetcher(backend: Arc<dyn RenderBackend>) -> BrowserFetcher {
        let f = BrowserFetcher::new(std::env::temp_dir());
        // Seed synchronously: no other task holds the lock yet.
        *f.backend.try_lock().expect("fresh lock") = Some(Ok(backend));
        f
    }

    #[tokio::test]
    async fn fetch_page_drives_the_backend_and_maps_to_fetched_page() {
        let backend = Arc::new(CountingBackend {
            in_flight: Default::default(),
            max_in_flight: Default::default(),
            html: "<html><head><title>T</title></head><body><h1>H</h1></body></html>".into(),
        });
        let fetcher = seeded_fetcher(backend);
        let page = fetcher.fetch_page("https://x.test/a").await.unwrap();
        assert_eq!(page.final_url, "https://x.test/a");
        assert_eq!(page.title.as_deref(), Some("T"));
        assert!(page.markdown.contains("# H"), "got: {}", page.markdown);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn fetch_page_serializes_concurrent_calls() {
        use std::sync::atomic::Ordering;
        let backend = Arc::new(CountingBackend {
            in_flight: Default::default(),
            max_in_flight: Default::default(),
            html: "<html><body><p>p</p></body></html>".into(),
        });
        let fetcher = Arc::new(seeded_fetcher(backend.clone()));

        // Fire 4 concurrent fetches (mirrors knowledge's buffer_unordered(4)).
        let mut handles = Vec::new();
        for i in 0..4 {
            let f = Arc::clone(&fetcher);
            handles.push(tokio::spawn(async move {
                f.fetch_page(&format!("https://x.test/{i}")).await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }
        // The Mutex must have kept max concurrency at 1 (the engine is not
        // concurrency-safe — overlapping calls would corrupt the page-session).
        assert_eq!(
            backend.max_in_flight.load(Ordering::SeqCst),
            1,
            "fetches must be serialized on the non-concurrency-safe engine"
        );
    }

    #[tokio::test]
    async fn fetch_page_surfaces_a_cached_engine_failure() {
        // Pre-seed a cached construction failure; fetch must report it (BadGateway),
        // not retry / not panic — same failure-cache discipline as BrowserTool.
        let f = BrowserFetcher::new(std::env::temp_dir());
        *f.backend.try_lock().expect("fresh lock") = Some(Err("chrome not resolvable".into()));
        let err = f.fetch_page("https://x.test/a").await.unwrap_err();
        assert!(matches!(err, AppError::BadGateway(_)), "{err:?}");
        assert!(err.to_string().contains("chrome not resolvable"), "{err}");
    }

    #[test]
    fn engine_config_keeps_ssrf_guard_on() {
        // decision ⑩: the knowledge-fetch engine must keep the IP-block guard on.
        let f = BrowserFetcher::new(PathBuf::from("/tmp/kb-browser"));
        let cfg = f.engine_config();
        assert!(cfg.firewall.block_private_ips, "SSRF IP-block must stay on");
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/kb-browser"));
        // Shared knowledge fetcher: no storage_state.
        assert!(cfg.storage_state.is_none());
    }

    // ── [#[ignore]] 真 Chrome：JS 渲染后抓取 vs 静态抓不到（X6 端到端证据）────────────
    //
    // 证明 BrowserFetcher 抓的是**渲染后**的 DOM：fixture 的 <body> 初始为空，正文由
    // <script> 在 load 后注入。静态 HTTP GET 看到的是脚本未执行的原始 HTML（无正文）；
    // BrowserFetcher 跑真 Chromium → 脚本执行 → markdown 必含 JS 注入的 sentinel。
    // 用 data: URL（无网络请求，故不触发 firewall IP 封禁，也无外网依赖/无 flakiness）。
    //
    // 本机/打包 chrome 手动跑：
    //   set NOMIFUN_CHROME_BINARY=C:\Program Files\Google\Chrome\Application\chrome.exe
    //   cargo nextest run -p nomifun-ai-agent --features browser-use --run-ignored all \
    //       browser_fetcher::tests::js_rendered_page_is_captured_after_render
    // 跑完核对任务管理器无残留 chrome（引擎 kill_on_drop / 清理网应自动清）。
    #[tokio::test]
    #[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored"]
    async fn js_rendered_page_is_captured_after_render() {
        // sentinel 只在脚本执行后才出现在 DOM 里（静态 HTML 的 <body> 是空的）。
        // 用 markdown 安全字符（无下划线/星号——htmd 会转义成 `\_`，破坏裸 contains 比对）。
        const SENTINEL: &str = "RENDEREDbyJSsentinel8675309";
        // 初始空 body + 脚本在 load 时把正文写进 DOM。**关键**：脚本里把 sentinel 拆成多段
        // 在运行时拼接，使**静态脚本源文本不含连续的 sentinel 串**——这样「静态转换看不到
        // sentinel、渲染后才看得到」是诚实对照（否则 htmd 的 script 文本泄漏会让对照失真）。
        let html = format!(
            "<!doctype html><html><head><title>SPA 渲染页</title></head>\
             <body><div id=\"app\"></div>\
             <script>\
               var s = ['RENDERED','by','JS','sentinel','8675309'].join('');\
               document.getElementById('app').innerHTML = \
                 '<h1>动态标题</h1><p>' + s + '</p>';\
             </script></body></html>"
        );
        // data: URL（无网络请求 → 不被 firewall IP 封禁拦）。
        let data_url = format!(
            "data:text/html;charset=utf-8,{}",
            urlencode_minimal(&html)
        );

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let fetcher = BrowserFetcher::new(tmp.path().join("kb-browser"));

        let page = fetcher
            .fetch_page(&data_url)
            .await
            .expect("BrowserFetcher.fetch_page should render the page");

        // 渲染后抓取成功：markdown 含 JS 注入的 sentinel + 动态标题（证明跑了脚本）。
        assert!(
            page.markdown.contains(SENTINEL),
            "rendered markdown must contain the JS-injected sentinel (proving post-render \
             capture); got:\n{}",
            page.markdown
        );
        assert!(
            page.markdown.contains("# 动态标题"),
            "rendered markdown should contain the JS-injected heading; got:\n{}",
            page.markdown
        );
        // title 来自 <title>（静态 head，html_to_markdown 提取）——与 HTTP 源同管线。
        assert_eq!(page.title.as_deref(), Some("SPA 渲染页"));
        // 干净 markdown：无 LLM-safety 包裹/脱敏泄漏进知识快照。
        assert!(!page.markdown.contains("<data"), "leaked <data> wrap:\n{}", page.markdown);
        assert!(!page.markdown.contains("[REDACTED"), "leaked redaction:\n{}", page.markdown);

        // 对照：静态视角（脚本**未**执行的原始 HTML）里 sentinel 在 <script> 文本中，
        // 但 html_to_markdown(skip script) 会丢弃它——故静态转换的 markdown **不含** sentinel。
        // 这正是「HTTP 抓不到、浏览器抓得到」的差异证据（同 html_to_markdown 管线，输入不同）。
        let (_t, static_md) =
            nomifun_knowledge::source_url::html_to_markdown(&html);
        assert!(
            !static_md.contains(SENTINEL),
            "static (script-not-executed) conversion must NOT contain the sentinel — that is the \
             rendered-vs-HTTP difference; got:\n{static_md}"
        );
    }

    /// 最小 percent-encode（仅 data: URL 测试用）：转义会破坏 data URL 解析的字符。
    /// 不求通用，只覆盖本 fixture（空格 / # / % / 引号等）。
    fn urlencode_minimal(s: &str) -> String {
        let mut out = String::with_capacity(s.len() * 2);
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/'
                | b'(' | b')' | b'<' | b'>' | b'=' | b';' | b':' | b'!' => out.push(b as char),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
}
