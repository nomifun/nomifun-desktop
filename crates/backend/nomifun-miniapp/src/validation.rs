//! Import validation: can this document survive as a mini-app?
//!
//! An imported app runs exactly where a generated one runs — inside an iframe
//! whose response carries `Content-Security-Policy: sandbox …` WITHOUT
//! `allow-same-origin`, served as ONE document from
//! `GET /api/miniapps/{id}/serve`. That runtime, not taste, is what every rule
//! below encodes:
//!
//! * one document, no sibling files — `/serve` returns the stored snapshot and
//!   nothing else, so a relative `src`/`href` has nothing to resolve against;
//! * an opaque origin — storage APIs may throw and cookies are absent;
//! * no build step and no server — source that needs a bundler or a template
//!   engine can never run.
//!
//! **Findings carry rule ids and data, never prose.** The bilingual sentence a
//! user reads lives in the UI's i18n keyed by `rule_id`, and the same ids are
//! what a conversion session is told to fix. That is also why the id set is
//! pinned by a test: renaming one silently blanks a user-facing message.
//!
//! **This is a bounded scan, not a parser.** Honest limits, so nobody trusts it
//! further than it deserves: it does not build a DOM, so it cannot tell a
//! reference inside an HTML comment or a JS string from a live one, and it
//! matches attribute values lexically. Every rule is therefore written to fail
//! toward *reporting* rather than toward silently importing something broken,
//! and none of them rewrites the document except the one documented fix below.
//! `dom_query`/`html5ever` are in the dependency graph transitively (via Tauri)
//! and a future revision may promote one to a direct dependency; the scan is
//! deliberately structured so each rule is an independent function that such a
//! revision can replace one at a time.

use serde::Serialize;

use crate::service::MINI_APP_HTML_MAX_BYTES;

/// How much a finding costs the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSeverity {
    /// Import is refused. The UI shows the remediation for this rule id.
    Fatal,
    /// NomiFun repairs it during import and says so afterwards. A rule may only
    /// carry this severity if [`apply_fixes`] actually implements the repair.
    Autofix,
    /// Imported as-is; the user is told what may bite later.
    Warning,
}

/// One thing the scan noticed.
///
/// `detail` is structured data the UI interpolates into its own sentence (the
/// offending reference, a byte count) — never a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportFinding {
    pub rule_id: &'static str,
    pub severity: ImportSeverity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ImportFinding {
    fn new(rule_id: &'static str, severity: ImportSeverity) -> Self {
        Self { rule_id, severity, detail: None }
    }

    fn with_detail(rule_id: &'static str, severity: ImportSeverity, detail: impl Into<String>) -> Self {
        Self { rule_id, severity, detail: Some(detail.into()) }
    }
}

/// The verdict on one candidate document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub findings: Vec<ImportFinding>,
    /// True when any finding is [`ImportSeverity::Fatal`]. The import route
    /// refuses on this flag alone, so a caller can never disagree with the UI
    /// about whether the document was acceptable.
    pub blocked: bool,
}

/// Every rule this version can emit, in report order.
///
/// Pinned by a test. The UI ships one i18n key per entry; adding an id here
/// without its copy leaves a user staring at a blank explanation.
pub const IMPORT_RULE_IDS: &[&str] = &[
    "empty_payload",
    "size_over_limit",
    "not_html",
    "no_root_document",
    "fragment_not_document",
    "local_ref_unsupported",
    "dev_server_ref",
    "framework_source_entry",
    "server_template_markers",
    "esm_bare_specifier",
    "external_cdn_ref",
    "web_storage_use",
    "nested_iframe_embed",
];

/// Attribute values worth inspecting for a reference. Deliberately narrow: the
/// point is to catch a page that cannot load its own parts, not to audit markup.
const REF_ATTRS: &[&str] = &["src=", "href=", "poster=", "data-src="];

/// Hosts that mean "this only worked on the author's machine".
const DEV_HOSTS: &[&str] = &["localhost", "127.0.0.1", "0.0.0.0", "[::1]"];

/// Validate one candidate document.
///
/// `siblings` are the payload-relative paths that came with it (empty for a
/// single-file import). They feed `scan_source_shape` — a `src/` tree or a
/// `package.json` beside the entry document is what tells a build-output page from
/// a hand-written one. They deliberately do NOT soften
/// `local_ref_unsupported`: `/serve` returns exactly one document, so a reference
/// is unresolvable whether or not the file it names travelled with the import,
/// which is what that rule's copy says in both locales.
pub fn validate_import(document: &str, siblings: &[String]) -> ImportReport {
    let mut findings = Vec::new();

    if document.trim().is_empty() {
        // Nothing else can be judged, and every later rule would report noise.
        return ImportReport { findings: vec![ImportFinding::new("empty_payload", ImportSeverity::Fatal)], blocked: true };
    }

    if document.len() > MINI_APP_HTML_MAX_BYTES {
        findings.push(ImportFinding::with_detail(
            "size_over_limit",
            ImportSeverity::Fatal,
            document.len().to_string(),
        ));
    }

    let lower = document.to_ascii_lowercase();

    if !looks_like_html(&lower) {
        // A pasted script, JSON or stylesheet. Everything downstream would be
        // gibberish, so stop here rather than piling on.
        findings.push(ImportFinding::new("not_html", ImportSeverity::Fatal));
        return finish(findings);
    }

    if !lower.contains("<html") && !lower.contains("<body") {
        // A fragment can be wrapped safely, and wrapping is the one repair this
        // module implements — see `apply_fixes`.
        findings.push(ImportFinding::new("fragment_not_document", ImportSeverity::Autofix));
    }

    findings.extend(scan_references(document, &lower));
    findings.extend(scan_source_shape(document, &lower, siblings));
    findings.extend(scan_runtime_warnings(&lower));

    finish(findings)
}

fn finish(findings: Vec<ImportFinding>) -> ImportReport {
    let blocked = findings.iter().any(|f| f.severity == ImportSeverity::Fatal);
    ImportReport { findings, blocked }
}

/// A directory import has to name its entry document.
///
/// Separate from [`validate_import`] because it is decided before any document
/// exists to scan.
pub fn find_root_document(siblings: &[String]) -> Result<String, ImportFinding> {
    let index = siblings
        .iter()
        .find(|path| path.eq_ignore_ascii_case("index.html") || path.eq_ignore_ascii_case("index.htm"));
    if let Some(found) = index {
        return Ok(found.clone());
    }
    let mut roots = siblings.iter().filter(|path| {
        !path.contains('/') && {
            let lower = path.to_ascii_lowercase();
            lower.ends_with(".html") || lower.ends_with(".htm")
        }
    });
    match (roots.next(), roots.next()) {
        // One unambiguous page at the root is a fine entry even unnamed.
        (Some(only), None) => Ok(only.clone()),
        _ => Err(ImportFinding::new("no_root_document", ImportSeverity::Fatal)),
    }
}

fn looks_like_html(lower: &str) -> bool {
    lower.contains("<!doctype html")
        || lower.contains("<html")
        || lower.contains("<body")
        || lower.contains("<div")
        || lower.contains("<h1")
        || lower.contains("<p>")
}

/// The same shape gate [`validate_import`] applies, for a document that did not
/// arrive through import.
///
/// `publish` needs it: the working copy is written by an agent, so "non-blank
/// UTF-8 under the cap" is not enough to prove the bytes are a page rather than a
/// plan, a stack trace or a half-written file. Owns its own lowercasing because
/// the caller has no reason to build one.
pub(crate) fn looks_like_html_document(document: &str) -> bool {
    looks_like_html(&document.to_ascii_lowercase())
}

/// References the served document could not resolve.
///
/// `local_ref_unsupported` names **every** unresolvable reference, not just the
/// first. An ordinary folder import (`index.html` + a stylesheet + a script + an
/// image) has several, and reporting one at a time would send the user back
/// through the native picker once per file, each round presented as the last thing
/// standing.
fn scan_references(document: &str, lower: &str) -> Vec<ImportFinding> {
    let mut findings = Vec::new();
    let mut local_unsupported: Vec<String> = Vec::new();
    let mut external_seen = false;
    let mut dev_ref: Option<String> = None;

    for value in reference_values(document) {
        let v = value.trim();
        if v.is_empty() || v.starts_with('#') || v.starts_with("data:") || v.starts_with("mailto:") {
            continue;
        }
        let vl = v.to_ascii_lowercase();

        if DEV_HOSTS.iter().any(|host| vl.contains(host)) {
            dev_ref.get_or_insert_with(|| v.to_string());
            continue;
        }
        if vl.starts_with("http://") || vl.starts_with("https://") || vl.starts_with("//") {
            external_seen = true;
            continue;
        }
        // Local, absolute in any flavour (a filesystem path, or a path that would
        // resolve against the API origin rather than the app) or relative. `/serve`
        // returns one document, so nothing can satisfy either — whether or not the
        // file travelled with the import. The detail names them so the UI (and a
        // conversion session) can point at the exact references.
        if !local_unsupported.iter().any(|seen| seen.eq_ignore_ascii_case(v)) {
            local_unsupported.push(v.to_string());
        }
    }

    // Any dev-server URL in a fetch/socket call counts too, even where no
    // attribute carries it.
    if dev_ref.is_none() {
        for host in DEV_HOSTS {
            if lower.contains(&format!("//{host}")) {
                dev_ref = Some((*host).to_string());
                break;
            }
        }
    }

    if !local_unsupported.is_empty() {
        findings.push(ImportFinding::with_detail(
            "local_ref_unsupported",
            ImportSeverity::Fatal,
            join_references(&local_unsupported),
        ));
    }
    if let Some(reference) = dev_ref {
        findings.push(ImportFinding::with_detail("dev_server_ref", ImportSeverity::Fatal, reference));
    }
    if external_seen {
        findings.push(ImportFinding::new("external_cdn_ref", ImportSeverity::Warning));
    }
    findings
}

/// The references as one `detail`, so the sentence that interpolates it stays
/// grammatical in both locales.
///
/// Capped: a generated page can carry dozens of icon references, and a wall of
/// them would bury the remediation. The cap is on display only — every reference
/// was still counted.
fn join_references(references: &[String]) -> String {
    const MAX_LISTED: usize = 8;
    let listed = references
        .iter()
        .take(MAX_LISTED)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if references.len() > MAX_LISTED {
        format!("{listed}, … (+{})", references.len() - MAX_LISTED)
    } else {
        listed
    }
}

/// Pull attribute values out of the document, quote-delimited only.
///
/// Unquoted attributes are rare in authored HTML and skipping them keeps this
/// from guessing where a value ends.
fn reference_values(document: &str) -> Vec<String> {
    let lower = document.to_ascii_lowercase();
    let mut out = Vec::new();
    for attr in REF_ATTRS {
        let mut from = 0usize;
        while let Some(found) = lower[from..].find(attr) {
            let after = from + found + attr.len();
            from = after;
            let rest = &document[after.min(document.len())..];
            let mut chars = rest.chars();
            let quote = match chars.next() {
                Some(q @ ('"' | '\'')) => q,
                _ => continue,
            };
            let value_start = after + quote.len_utf8();
            if let Some(end) = document[value_start..].find(quote) {
                out.push(document[value_start..value_start + end].to_string());
                from = value_start + end;
            }
        }
    }
    out
}

/// Source that needs a toolchain we do not have.
fn scan_source_shape(document: &str, lower: &str, siblings: &[String]) -> Vec<ImportFinding> {
    let mut findings = Vec::new();

    let vite_entry = lower.contains("/src/main.ts")
        || lower.contains("/src/main.tsx")
        || lower.contains("/src/main.js")
        || lower.contains("/src/main.jsx")
        || lower.contains("src=\"/src/")
        || lower.contains("src='/src/");
    let framework_sibling = siblings.iter().any(|s| {
        let l = s.to_ascii_lowercase();
        l.ends_with(".vue") || l.ends_with(".svelte") || l.ends_with(".tsx") || l.ends_with(".jsx")
    });
    if vite_entry || framework_sibling {
        findings.push(ImportFinding::new("framework_source_entry", ImportSeverity::Fatal));
    }

    // `${` is deliberately absent: it is ordinary JS template-literal syntax and
    // flagging it would fire on almost every real app. Bare `{{ }}` is absent for
    // the same reason (Vue, Alpine and Handlebars all use it client-side).
    const SERVER_MARKERS: &[&str] = &["<?php", "<?=", "<%", "{%", "@model", "th:text="];
    if let Some(marker) = SERVER_MARKERS.iter().find(|m| lower.contains(**m)) {
        findings.push(ImportFinding::with_detail(
            "server_template_markers",
            ImportSeverity::Fatal,
            *marker,
        ));
    }

    if lower.contains("type=\"module\"") || lower.contains("type='module'") {
        if !lower.contains("type=\"importmap\"") && !lower.contains("type='importmap'") {
            if let Some(specifier) = first_bare_specifier(document) {
                findings.push(ImportFinding::with_detail(
                    "esm_bare_specifier",
                    ImportSeverity::Fatal,
                    specifier,
                ));
            }
        }
    }
    findings
}

/// The first `from '<bare>'` a module script would fail to resolve.
fn first_bare_specifier(document: &str) -> Option<String> {
    let lower = document.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(found) = lower[from..].find("from ") {
        let after = from + found + "from ".len();
        from = after;
        let rest = document.get(after..)?;
        let mut chars = rest.chars();
        let quote = match chars.next() {
            Some(q @ ('"' | '\'')) => q,
            _ => continue,
        };
        let start = after + quote.len_utf8();
        let end = document.get(start..)?.find(quote)? + start;
        let specifier = &document[start..end];
        let bare = !specifier.starts_with('.')
            && !specifier.starts_with('/')
            && !specifier.contains("://")
            && !specifier.starts_with("data:");
        if bare && !specifier.is_empty() {
            return Some(specifier.to_string());
        }
        from = end;
    }
    None
}

/// Things that work, but not the way the author expects, inside the sandbox.
fn scan_runtime_warnings(lower: &str) -> Vec<ImportFinding> {
    let mut findings = Vec::new();
    if lower.contains("localstorage") || lower.contains("sessionstorage") || lower.contains("indexeddb") {
        // The opaque origin makes storage access throw in some engines and
        // silently per-session in others. Never fatal — an app that only uses it
        // for convenience still works.
        findings.push(ImportFinding::new("web_storage_use", ImportSeverity::Warning));
    }
    if lower.contains("<iframe") {
        findings.push(ImportFinding::new("nested_iframe_embed", ImportSeverity::Warning));
    }
    findings
}

/// Apply every repair this module claims.
///
/// One repair only, and it is the only rule marked [`ImportSeverity::Autofix`]:
/// a fragment becomes a real document. Returns the document to store plus the
/// ids that were actually fixed, so the UI reports repairs it can prove happened
/// rather than repairs the catalogue merely hoped for.
pub fn apply_fixes(document: &str, report: &ImportReport) -> (String, Vec<&'static str>) {
    let mut applied = Vec::new();
    let mut out = document.to_string();
    if report.findings.iter().any(|f| f.rule_id == "fragment_not_document") {
        out = format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             </head>\n<body>\n{}\n</body>\n</html>\n",
            document.trim()
        );
        applied.push("fragment_not_document");
    }
    (out, applied)
}

/// The document's `<title>`, for naming an imported app.
pub fn document_title(document: &str) -> Option<String> {
    let lower = document.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let gt = lower[open..].find('>')? + open + 1;
    let close = lower[gt..].find("</title>")? + gt;
    let title = document[gt..close].trim();
    if title.is_empty() { None } else { Some(title.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<!DOCTYPE html><html><head><title>Timer</title></head><body><h1>hi</h1></body></html>";

    fn ids(report: &ImportReport) -> Vec<&'static str> {
        report.findings.iter().map(|f| f.rule_id).collect()
    }

    #[test]
    fn a_self_contained_page_is_accepted_without_findings() {
        let report = validate_import(PAGE, &[]);
        assert!(!report.blocked, "{report:?}");
        assert!(report.findings.is_empty(), "{report:?}");
    }

    #[test]
    fn every_documented_rule_id_is_reachable_and_no_other_is_emitted() {
        // Drift latch: the UI ships one i18n key per documented id, so an id that
        // exists only in code renders as a blank explanation.
        let mut emitted: Vec<&str> = Vec::new();
        let cases: Vec<(&str, Vec<String>)> = vec![
            ("   ", vec![]),
            ("console.log('hi')", vec![]),
            ("<div>fragment</div>", vec![]),
            ("<html><body><img src=\"logo.png\"></body></html>", vec![]),
            ("<html><body><script src=\"http://localhost:5173/x.js\"></script></body></html>", vec![]),
            ("<html><body><script type=\"module\" src=\"/src/main.tsx\"></script></body></html>", vec![]),
            ("<html><body><?php echo 1; ?></body></html>", vec![]),
            ("<html><body><script type=\"module\">import x from 'react'</script></body></html>", vec![]),
            ("<html><body><script src=\"https://cdn.example/x.js\"></script></body></html>", vec![]),
            ("<html><body><script>localStorage.setItem('a','b')</script></body></html>", vec![]),
            ("<html><body><iframe src=\"https://example.com\"></iframe></body></html>", vec![]),
        ];
        for (document, siblings) in &cases {
            for finding in validate_import(document, siblings).findings {
                emitted.push(finding.rule_id);
            }
        }
        emitted.push(find_root_document(&["readme.md".to_string()]).unwrap_err().rule_id);
        emitted.push(
            validate_import(&"x".repeat(MINI_APP_HTML_MAX_BYTES + 1), &[]).findings[0].rule_id,
        );
        for id in &emitted {
            assert!(IMPORT_RULE_IDS.contains(id), "undocumented rule id emitted: {id}");
        }
        for id in IMPORT_RULE_IDS {
            assert!(emitted.contains(id), "documented rule id never emitted: {id}");
        }
    }

    #[test]
    fn an_empty_payload_short_circuits_every_other_rule() {
        let report = validate_import("   \n\t ", &[]);
        assert_eq!(ids(&report), vec!["empty_payload"]);
        assert!(report.blocked);
    }

    #[test]
    fn a_pasted_script_is_not_html_and_stops_the_scan() {
        let report = validate_import("const x = 1; export default x;", &[]);
        assert_eq!(ids(&report), vec!["not_html"]);
        assert!(report.blocked);
    }

    #[test]
    fn a_fragment_is_wrapped_rather_than_refused() {
        let report = validate_import("<div>hello</div>", &[]);
        assert_eq!(ids(&report), vec!["fragment_not_document"]);
        assert!(!report.blocked, "an autofix must never block");
        let (fixed, applied) = apply_fixes("<div>hello</div>", &report);
        assert_eq!(applied, vec!["fragment_not_document"]);
        assert!(fixed.starts_with("<!DOCTYPE html>"));
        assert!(fixed.contains("<div>hello</div>"));
        // The wrapped document must itself pass.
        assert!(validate_import(&fixed, &[]).findings.is_empty());
    }

    #[test]
    fn a_relative_reference_is_fatal_even_when_the_file_travelled_along() {
        // `/serve` returns one document, so a sibling file cannot be reached —
        // "it exists" does not make it loadable.
        let doc = "<html><body><link href=\"style.css\" rel=\"stylesheet\"></body></html>";
        let report = validate_import(doc, &["style.css".to_string()]);
        let finding = report.findings.iter().find(|f| f.rule_id == "local_ref_unsupported").unwrap();
        assert_eq!(finding.severity, ImportSeverity::Fatal);
        assert_eq!(finding.detail.as_deref(), Some("style.css"));
    }

    #[test]
    fn absolute_and_file_url_references_are_reported_as_unsupported() {
        for value in ["/assets/app.js", "file:///home/me/app.js", "C:\\site\\app.js"] {
            let doc = format!("<html><body><script src=\"{value}\"></script></body></html>");
            let report = validate_import(&doc, &[]);
            assert!(ids(&report).contains(&"local_ref_unsupported"), "{value}: {report:?}");
        }
    }

    #[test]
    fn a_dev_server_reference_is_fatal_and_named() {
        let doc = "<html><body><script src=\"http://127.0.0.1:5173/main.js\"></script></body></html>";
        let report = validate_import(doc, &[]);
        let finding = report.findings.iter().find(|f| f.rule_id == "dev_server_ref").unwrap();
        assert_eq!(finding.severity, ImportSeverity::Fatal);
        assert!(finding.detail.as_deref().unwrap().contains("127.0.0.1"));
        // It must not ALSO be counted as an ordinary external reference.
        assert!(!ids(&report).contains(&"external_cdn_ref"));
    }

    #[test]
    fn a_dev_server_url_inside_a_fetch_call_is_still_caught() {
        let doc = "<html><body><script>fetch('http://localhost:8000/api')</script></body></html>";
        assert!(ids(&validate_import(doc, &[])).contains(&"dev_server_ref"));
    }

    #[test]
    fn an_unbuilt_framework_entry_is_fatal_from_either_signal() {
        let vite = "<html><body><script type=\"module\" src=\"/src/main.tsx\"></script></body></html>";
        assert!(ids(&validate_import(vite, &[])).contains(&"framework_source_entry"));
        let page = format!("{PAGE}");
        assert!(
            ids(&validate_import(&page, &["App.vue".to_string()])).contains(&"framework_source_entry")
        );
    }

    #[test]
    fn server_side_markers_are_fatal_but_client_interpolation_is_not() {
        assert!(ids(&validate_import("<html><body><?php echo 1; ?></body></html>", &[]))
            .contains(&"server_template_markers"));
        // Vue/Handlebars/Alpine delimiters and JS template literals must not fire.
        let client = "<html><body><div>{{ count }}</div><script>const s=`a${b}c`</script></body></html>";
        assert!(!ids(&validate_import(client, &[])).contains(&"server_template_markers"));
    }

    #[test]
    fn a_bare_module_specifier_is_fatal_unless_an_importmap_covers_it() {
        let bare = "<html><body><script type=\"module\">import React from 'react'</script></body></html>";
        let report = validate_import(bare, &[]);
        let finding = report.findings.iter().find(|f| f.rule_id == "esm_bare_specifier").unwrap();
        assert_eq!(finding.detail.as_deref(), Some("react"));

        let mapped = "<html><body><script type=\"importmap\">{}</script>\
                      <script type=\"module\">import React from 'react'</script></body></html>";
        assert!(!ids(&validate_import(mapped, &[])).contains(&"esm_bare_specifier"));

        let url = "<html><body><script type=\"module\">import x from 'https://cdn.example/x.js'</script></body></html>";
        assert!(!ids(&validate_import(url, &[])).contains(&"esm_bare_specifier"));
    }

    #[test]
    fn external_cdn_storage_and_nested_frames_warn_without_blocking() {
        let doc = "<html><body><script src=\"https://cdn.example/x.js\"></script>\
                   <script>localStorage.setItem('a','b')</script>\
                   <iframe src=\"https://example.com\"></iframe></body></html>";
        let report = validate_import(doc, &[]);
        assert!(!report.blocked, "{report:?}");
        for id in ["external_cdn_ref", "web_storage_use", "nested_iframe_embed"] {
            let finding = report.findings.iter().find(|f| f.rule_id == id).unwrap();
            assert_eq!(finding.severity, ImportSeverity::Warning);
        }
    }

    #[test]
    fn an_oversized_document_is_fatal_and_reports_its_size() {
        let huge = "x".repeat(MINI_APP_HTML_MAX_BYTES + 1);
        let report = validate_import(&huge, &[]);
        let finding = report.findings.iter().find(|f| f.rule_id == "size_over_limit").unwrap();
        assert_eq!(finding.severity, ImportSeverity::Fatal);
        assert_eq!(finding.detail.as_deref(), Some((MINI_APP_HTML_MAX_BYTES + 1).to_string()).as_deref());
    }

    #[test]
    fn a_directory_entry_document_is_found_or_reported() {
        assert_eq!(find_root_document(&["index.html".into(), "a.css".into()]).unwrap(), "index.html");
        assert_eq!(find_root_document(&["INDEX.HTM".into()]).unwrap(), "INDEX.HTM");
        // A single unambiguous root page is a fine entry even when unnamed.
        assert_eq!(find_root_document(&["app.html".into(), "a.css".into()]).unwrap(), "app.html");
        // Two candidates: the user has to pick, so this is not a guess we make.
        assert_eq!(
            find_root_document(&["a.html".into(), "b.html".into()]).unwrap_err().rule_id,
            "no_root_document"
        );
        assert_eq!(find_root_document(&["readme.md".into()]).unwrap_err().rule_id, "no_root_document");
    }

    #[test]
    fn a_title_names_an_imported_app_when_there_is_one() {
        assert_eq!(document_title(PAGE).as_deref(), Some("Timer"));
        assert_eq!(document_title("<html><title>  </title></html>"), None);
        assert_eq!(document_title("<html><body>no title</body></html>"), None);
        assert_eq!(
            document_title("<html><title lang=\"en\">Spaced Out</title></html>").as_deref(),
            Some("Spaced Out")
        );
    }

    #[test]
    fn blocked_is_true_exactly_when_a_fatal_finding_exists() {
        assert!(validate_import("<html><body><img src=\"a.png\"></body></html>", &[]).blocked);
        assert!(!validate_import("<div>x</div>", &[]).blocked);
        assert!(!validate_import(PAGE, &[]).blocked);
    }
}
