//! SkillHub expert-package support: resolve a package entry into its
//! name/instructions/child-skill slugs, and install those child skills by
//! downloading each skill zip and importing it through
//! [`crate::skill_service::import_skills_with_symlink`] (which extracts via
//! the hardened [`crate::zip_safe`] path).

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_api_types::{
    SkillMarketPackageInstallError, SkillMarketPackageInstallResponse, SkillMarketPackageRequest,
    SkillMarketPackageResponse,
};
use nomifun_common::AppError;
use reqwest::header::ACCEPT;

use crate::skill_service::{self, SkillPaths};

use super::client::{
    MAX_SKILLHUB_SKILL_ZIP_BYTES, build_market_client, map_market_fetch_error, read_market_body,
    read_market_bytes, read_market_detail_body,
};
use super::parse::{
    dedup_strings, is_market_slug, json_text, json_text_preserve, last_url_segment, market_ref_suffix,
    title_from_slug,
};
use super::SKILLHUB_PACKAGES_SOURCE;

const SKILLHUB_SKILL_DOWNLOAD_URL: &str = "https://api.skillhub.cn/api/v1/download";
const SKILLHUB_SKILL_SEARCH_URL: &str = "https://api.skillhub.cn/api/v1/search";
const SKILLHUB_PACKAGE_DETAIL_BASE_URL: &str = "https://api.skillhub.cn/api/v1/skillsets/";

/// Resolve a SkillHub expert package and install its child skills. This is
/// the `POST /api/skills/market/package/install` implementation; resolving
/// without installing has no frontend caller, so [`resolve_market_package`]
/// stays internal.
pub async fn install_market_package(
    paths: &SkillPaths,
    req: SkillMarketPackageRequest,
) -> Result<SkillMarketPackageInstallResponse, AppError> {
    let package = resolve_market_package(req).await?;
    let install_result = install_skillhub_package_skills(paths, &package.skill_slugs).await?;
    Ok(SkillMarketPackageInstallResponse {
        package,
        installed_skill_names: install_result.installed_skill_names,
        errors: install_result.errors,
    })
}

/// Fetch one package by slug from the SkillHub detail endpoint and build its
/// response (name, instructions, child skill slugs).
async fn resolve_market_package(req: SkillMarketPackageRequest) -> Result<SkillMarketPackageResponse, AppError> {
    if req.source != SKILLHUB_PACKAGES_SOURCE {
        return Err(AppError::BadRequest(format!(
            "unsupported package market source: {}",
            req.source
        )));
    }
    let slug = resolve_skillhub_package_slug(&req)?;

    let client = build_market_client()?;
    let url = skillhub_package_detail_url(&slug)?;
    let body = read_market_detail_body(&client, url.as_str(), &format!("SkillHub package '{slug}'")).await?;
    parse_skillhub_package_detail(&body, &slug)
}

/// Resolve a package slug from the backend-issued id and URL. A stale cache
/// may contain one malformed field, so a valid peer can recover it; two valid
/// but conflicting fields are rejected rather than silently selecting one.
fn resolve_skillhub_package_slug(req: &SkillMarketPackageRequest) -> Result<String, AppError> {
    let id_slug = market_ref_suffix(&req.id, SKILLHUB_PACKAGES_SOURCE).filter(|slug| is_market_slug(slug));
    let url_slug = last_url_segment(&req.url).filter(|slug| is_market_slug(slug));

    match (id_slug, url_slug) {
        (Some(id_slug), Some(url_slug)) if !id_slug.eq_ignore_ascii_case(&url_slug) => Err(AppError::BadRequest(
            "SkillHub package id and URL refer to different packages".into(),
        )),
        (Some(slug), _) | (_, Some(slug)) => Ok(slug),
        (None, None) => Err(AppError::BadRequest("invalid SkillHub package slug".into())),
    }
}

fn skillhub_package_detail_url(slug: &str) -> Result<reqwest::Url, AppError> {
    if !is_market_slug(slug) {
        return Err(AppError::BadRequest("invalid SkillHub package slug".into()));
    }
    let mut url = reqwest::Url::parse(SKILLHUB_PACKAGE_DETAIL_BASE_URL)
        .map_err(|e| AppError::Internal(format!("invalid SkillHub package detail URL: {e}")))?;
    url.path_segments_mut()
        .map_err(|_| AppError::Internal("invalid SkillHub package detail URL base".into()))?
        .pop_if_empty()
        .push(slug);
    Ok(url)
}

fn parse_skillhub_package_detail(body: &str, slug: &str) -> Result<SkillMarketPackageResponse, AppError> {
    let package = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|e| AppError::BadGateway(format!("SkillHub package JSON parse failed: {e}")))?;
    let returned_slug = package
        .get("slug")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| is_market_slug(value))
        .ok_or_else(|| AppError::BadGateway("SkillHub package response missing slug".into()))?;
    if !returned_slug.eq_ignore_ascii_case(slug) {
        return Err(AppError::BadGateway(format!(
            "SkillHub package response slug mismatch: expected '{slug}', got '{returned_slug}'"
        )));
    }

    build_skillhub_package_response(&package, slug)
}

fn build_skillhub_package_response(
    package: &serde_json::Value,
    slug: &str,
) -> Result<SkillMarketPackageResponse, AppError> {
    let name = json_text(package, "displayName", 96)
        .or_else(|| json_text(package, "displayNameEn", 96))
        .unwrap_or_else(|| title_from_slug(slug));
    let description = json_text(package, "summary", 500)
        .or_else(|| json_text(package, "summaryEn", 500))
        .unwrap_or_default();
    let instructions = json_text_preserve(package, "content", 120_000)
        .or_else(|| json_text_preserve(package, "contentEn", 120_000))
        .ok_or_else(|| AppError::BadGateway("SkillHub package content missing".into()))?;
    let skill_slugs = package_skill_slugs(package, &instructions);
    let avatar = json_text(package, "iconUrl", 260);

    Ok(SkillMarketPackageResponse {
        name,
        description,
        instructions,
        skill_slugs,
        avatar,
    })
}

// ---------------------------------------------------------------------------
// Child skill install
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SkillMarketPackageSkillInstallOutcome {
    installed_skill_names: Vec<String>,
    errors: Vec<SkillMarketPackageInstallError>,
}

/// Install every child skill of a package, best-effort: already-available
/// skills are reused by name, missing ones are downloaded from SkillHub, and
/// a failing child is reported in `errors` without aborting its siblings.
async fn install_skillhub_package_skills(
    paths: &SkillPaths,
    skill_slugs: &[String],
) -> Result<SkillMarketPackageSkillInstallOutcome, AppError> {
    let slugs = normalize_package_skill_install_slugs(skill_slugs.to_vec());
    if slugs.is_empty() {
        return Ok(SkillMarketPackageSkillInstallOutcome::default());
    }

    let available = skill_service::list_available_skills(paths).await?;
    let mut available_names = available
        .into_iter()
        .map(|skill| (skill.name.to_ascii_lowercase(), skill.name))
        .collect::<HashMap<_, _>>();
    let client = build_market_client()?;
    let mut installed_skill_names = Vec::new();
    let mut errors = Vec::new();

    for slug in slugs {
        if let Some(name) = available_names.get(&slug.to_ascii_lowercase()) {
            installed_skill_names.push(name.clone());
            continue;
        }

        let child_result = async {
            let (download_slug, archive) = download_skillhub_skill_zip(&client, &slug).await?;
            import_skillhub_skill_archive(paths, &download_slug, &archive).await
        }
        .await;

        match child_result {
            Ok(imported) => {
                for name in imported {
                    available_names.insert(name.to_ascii_lowercase(), name.clone());
                    installed_skill_names.push(name);
                }
            }
            Err(error) => errors.push(SkillMarketPackageInstallError {
                skill_slug: slug,
                error: error.to_string(),
            }),
        }
    }

    dedup_strings(&mut installed_skill_names);
    Ok(SkillMarketPackageSkillInstallOutcome {
        installed_skill_names,
        errors,
    })
}

/// Download a skill zip by slug, falling back to an exact-match search when
/// the direct download 404s. The slug is validated BEFORE any URL or temp
/// path is built from it.
async fn download_skillhub_skill_zip(
    client: &reqwest::Client,
    skill_slug: &str,
) -> Result<(String, Vec<u8>), AppError> {
    if !is_market_slug(skill_slug) {
        return Err(AppError::BadRequest("invalid SkillHub skill slug".into()));
    }

    match request_skillhub_skill_zip(client, skill_slug).await {
        Ok(bytes) => Ok((skill_slug.to_string(), bytes)),
        Err(AppError::NotFound(_)) => {
            let found_slug = search_skillhub_skill_slug(client, skill_slug).await?;
            let bytes = request_skillhub_skill_zip(client, &found_slug).await?;
            Ok((found_slug, bytes))
        }
        Err(error) => Err(error),
    }
}

async fn request_skillhub_skill_zip(client: &reqwest::Client, skill_slug: &str) -> Result<Vec<u8>, AppError> {
    let url = skillhub_skill_download_url(skill_slug)?;
    let mut response = client
        .get(url)
        .header(ACCEPT, "application/zip,application/octet-stream,*/*")
        .send()
        .await
        .map_err(map_market_fetch_error)?;
    read_market_bytes(&mut response, MAX_SKILLHUB_SKILL_ZIP_BYTES, "SkillHub skill archive").await
}

fn skillhub_skill_download_url(skill_slug: &str) -> Result<reqwest::Url, AppError> {
    if !is_market_slug(skill_slug) {
        return Err(AppError::BadRequest("invalid SkillHub skill slug".into()));
    }
    reqwest::Url::parse_with_params(SKILLHUB_SKILL_DOWNLOAD_URL, &[("slug", skill_slug)])
        .map_err(|e| AppError::Internal(format!("invalid SkillHub download URL: {e}")))
}

async fn search_skillhub_skill_slug(client: &reqwest::Client, skill_slug: &str) -> Result<String, AppError> {
    let url = reqwest::Url::parse_with_params(
        SKILLHUB_SKILL_SEARCH_URL,
        &[("q", skill_slug), ("limit", "20")],
    )
    .map_err(|e| AppError::Internal(format!("invalid SkillHub search URL: {e}")))?;
    let body = read_market_body(client, url.as_str()).await?;
    let root = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| AppError::BadGateway(format!("SkillHub search JSON parse failed: {e}")))?;
    select_skillhub_search_slug(&root, skill_slug)
        .ok_or_else(|| AppError::NotFound(format!("SkillHub skill '{skill_slug}' not found")))
}

fn select_skillhub_search_slug(root: &serde_json::Value, requested_slug: &str) -> Option<String> {
    let results = root
        .get("results")
        .and_then(serde_json::Value::as_array)
        .or_else(|| root.as_array())?;

    results.iter().find_map(|item| {
        let slug = json_text(item, "slug", 96)
            .or_else(|| item.get("skill").and_then(|skill| json_text(skill, "slug", 96)))?;
        if is_market_slug(&slug) && slug.eq_ignore_ascii_case(requested_slug) {
            Some(slug)
        } else {
            None
        }
    })
}

/// Persist the downloaded archive to a nonce-named temp file and hand it to
/// [`skill_service::import_skills_with_symlink`], whose zip path extracts
/// through the hardened [`crate::zip_safe::extract_zip_archive`].
async fn import_skillhub_skill_archive(
    paths: &SkillPaths,
    skill_slug: &str,
    archive: &[u8],
) -> Result<Vec<String>, AppError> {
    let temp_dir = paths.user_skills_dir.join(".market-import");
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let archive_path = temp_dir.join(format!("skillhub-{skill_slug}-{nonce}.zip"));
    tokio::fs::write(&archive_path, archive)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let result = skill_service::import_skills_with_symlink(paths, &archive_path)
        .await
        .map_err(AppError::from);
    let _ = tokio::fs::remove_file(&archive_path).await;
    let _ = tokio::fs::remove_dir(&temp_dir).await;
    result
}

// ---------------------------------------------------------------------------
// Child slug extraction (skillSlugs field + frontmatter children)
// ---------------------------------------------------------------------------

fn package_skill_slugs(package: &serde_json::Value, instructions: &str) -> Vec<String> {
    let mut slugs = package
        .get("skillSlugs")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    slugs.extend(frontmatter_child_slugs(instructions));
    normalize_package_skill_slugs(slugs)
}

fn normalize_package_skill_slugs(slugs: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    slugs
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| is_market_slug(value) && !is_package_metadata_field(value))
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

/// Looser variant used at install time: keeps invalid slugs so the install
/// loop can report a per-slug error instead of silently dropping them.
fn normalize_package_skill_install_slugs(slugs: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    slugs
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && !is_package_metadata_field(value))
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

/// SkillHub package `skillSlugs` arrays sometimes echo frontmatter field
/// names; those are metadata, not installable skills.
fn is_package_metadata_field(value: &str) -> bool {
    const FIELDS: &[&str] = &[
        "aliases",
        "author",
        "children",
        "compatibility",
        "description",
        "display_name",
        "metadata",
        "name",
        "orchestration",
        "package_type",
        "version",
    ];
    FIELDS.iter().any(|field| value.eq_ignore_ascii_case(field))
}

fn frontmatter_child_slugs(markdown: &str) -> Vec<String> {
    let Some(frontmatter) = markdown_frontmatter(markdown) else {
        return Vec::new();
    };
    let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) else {
        return Vec::new();
    };
    let Some(children) = root.get("orchestration").and_then(|value| value.get("children")) else {
        return Vec::new();
    };

    children
        .as_sequence()
        .into_iter()
        .flatten()
        .filter_map(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

/// Return the YAML frontmatter body of a markdown document: the content
/// between a leading `---` line and the closing `---`/`...` line. `None`
/// when the document has no (closed) frontmatter block.
fn markdown_frontmatter(markdown: &str) -> Option<&str> {
    let markdown = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let rest = markdown
        .strip_prefix("---\r\n")
        .or_else(|| markdown.strip_prefix("---\n"))?;

    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            return Some(rest[..offset].trim());
        }
        offset += line.len();
    }

    None
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_paths() -> SkillPaths {
        let tmp = TempDir::new().unwrap();
        let paths = SkillPaths {
            data_dir: tmp.path().to_path_buf(),
            user_skills_dir: tmp.path().join("skills"),
            cron_skills_dir: tmp.path().join("cron").join("skills"),
            builtin_skills_dir: tmp.path().join("builtin-skills"),
            builtin_rules_dir: tmp.path().join("builtin-rules"),
            preset_rules_dir: tmp.path().join("preset-rules"),
            preset_skills_dir: tmp.path().join("preset-skills"),
        };
        std::mem::forget(tmp);
        paths
    }

    #[test]
    fn build_skillhub_package_response_uses_real_child_skills() {
        let package = serde_json::json!({
            "slug": "tech-test-automation",
            "displayName": "Test Automation",
            "summary": "End-to-end automated testing workflow.",
            "skillSlugs": ["name", "superpowers-tdd", "description", "superpowers-tdd"],
            "content": "---\nname: tech-test-automation\ndescription: Test package\nmetadata:\n  author: SkillHub\norchestration:\n  children:\n    - test-case-generator\n    - metadata\n---\n# Test Automation\nUse this package."
        });

        let response = build_skillhub_package_response(&package, "tech-test-automation").unwrap();

        assert_eq!(response.skill_slugs, vec!["superpowers-tdd", "test-case-generator"]);
        assert!(response.instructions.starts_with("---\nname: tech-test-automation"));
        assert!(response.instructions.contains("metadata:"));
        assert!(response.instructions.contains("# Test Automation"));
    }

    #[test]
    fn package_detail_url_targets_one_validated_slug() {
        assert_eq!(
            skillhub_package_detail_url("tech-test-automation")
                .unwrap()
                .as_str(),
            "https://api.skillhub.cn/api/v1/skillsets/tech-test-automation"
        );
        assert!(skillhub_package_detail_url("../tech-test-automation").is_err());
    }

    #[test]
    fn package_slug_recovers_one_stale_field_but_rejects_conflicts() {
        let valid = SkillMarketPackageRequest {
            source: SKILLHUB_PACKAGES_SOURCE.into(),
            id: "skillhub_packages:tech-test-automation".into(),
            url: "https://skillhub.cn/skillspackage/tech-test-automation".into(),
        };
        assert_eq!(
            resolve_skillhub_package_slug(&valid).unwrap(),
            "tech-test-automation"
        );

        let stale_id = SkillMarketPackageRequest {
            id: "skillhub_packages:invalid slug".into(),
            ..valid.clone()
        };
        assert_eq!(
            resolve_skillhub_package_slug(&stale_id).unwrap(),
            "tech-test-automation"
        );

        let conflicting = SkillMarketPackageRequest {
            url: "https://skillhub.cn/skillspackage/another-package".into(),
            ..valid
        };
        assert!(resolve_skillhub_package_slug(&conflicting).is_err());
    }

    #[test]
    fn package_detail_parser_requires_matching_slug_and_content() {
        let body = serde_json::json!({
            "slug": "tech-test-automation",
            "displayName": "Test Automation",
            "summary": "End-to-end automated testing workflow.",
            "skillSlugs": ["superpowers-tdd"],
            "content": "# Test Automation\nUse this package."
        })
        .to_string();

        let package = parse_skillhub_package_detail(&body, "tech-test-automation").unwrap();
        assert_eq!(package.name, "Test Automation");
        assert_eq!(package.skill_slugs, vec!["superpowers-tdd"]);
        assert!(parse_skillhub_package_detail(&body, "another-package").is_err());

        let missing_content = serde_json::json!({ "slug": "tech-test-automation" }).to_string();
        assert!(parse_skillhub_package_detail(&missing_content, "tech-test-automation").is_err());
    }

    #[test]
    fn package_child_slug_preserves_the_full_backend_limit() {
        let max_slug = format!("a{}z", "b".repeat(94));
        let over_limit = format!("a{}z", "b".repeat(95));
        let package = serde_json::json!({ "skillSlugs": [max_slug, over_limit.clone()] });

        let slugs = package_skill_slugs(&package, "# Package");

        assert_eq!(slugs, vec![format!("a{}z", "b".repeat(94))]);

        let invalid_for_reporting = normalize_package_skill_install_slugs(vec![over_limit.clone()]);
        assert_eq!(invalid_for_reporting, vec![over_limit]);
        assert!(!is_market_slug(&invalid_for_reporting[0]));
    }

    /// Manual smoke test for the exact lightweight endpoint used by Add.
    /// Ignored in normal test runs because SkillHub is outside NomiFun's
    /// availability control.
    #[tokio::test]
    #[ignore = "requires public SkillHub access"]
    async fn live_skillhub_package_detail_contract() {
        let package = resolve_market_package(SkillMarketPackageRequest {
            source: SKILLHUB_PACKAGES_SOURCE.into(),
            id: "skillhub_packages:tech-test-automation".into(),
            url: "https://skillhub.cn/skillspackage/tech-test-automation".into(),
        })
        .await
        .unwrap();

        assert!(!package.name.is_empty());
        assert!(!package.instructions.is_empty());
        assert!(!package.skill_slugs.is_empty());
    }

    /// Manual smoke test for the official API -> Tencent COS redirect used by
    /// child-skill archives. The redirect target remains exact-allowlisted.
    #[tokio::test]
    #[ignore = "requires public SkillHub and Tencent COS access"]
    async fn live_skillhub_child_archive_redirect_contract() {
        let client = build_market_client().unwrap();
        let archive = request_skillhub_skill_zip(&client, "superpowers-tdd")
            .await
            .unwrap();

        assert!(archive.starts_with(b"PK"));
    }

    #[test]
    fn markdown_frontmatter_requires_closed_leading_block() {
        let doc = "---\nname: x\norchestration:\n  children:\n    - a\n---\nbody";
        assert_eq!(
            markdown_frontmatter(doc),
            Some("name: x\norchestration:\n  children:\n    - a")
        );
        // CRLF and `...` terminator variants.
        assert_eq!(markdown_frontmatter("---\r\nname: y\r\n---\r\nbody"), Some("name: y"));
        assert_eq!(markdown_frontmatter("---\nname: z\n...\n"), Some("name: z"));
        // No frontmatter / unterminated block.
        assert_eq!(markdown_frontmatter("# heading"), None);
        assert_eq!(markdown_frontmatter("---\nname: never closed"), None);
    }

    #[test]
    fn skillhub_skill_download_url_rejects_unsafe_slug() {
        assert!(skillhub_skill_download_url("superpowers-tdd").is_ok());
        assert!(skillhub_skill_download_url("../superpowers-tdd").is_err());
        assert!(skillhub_skill_download_url("owner/skill").is_err());
    }

    #[test]
    fn select_skillhub_search_slug_requires_exact_safe_slug() {
        let root = serde_json::json!({
            "results": [
                { "slug": "superpowers-tdd-extra", "displayName": "Superpowers TDD Extra" },
                { "skill": { "slug": "superpowers-tdd" }, "displayName": "Superpowers TDD" },
                { "slug": "../bad", "displayName": "Bad" }
            ]
        });

        assert_eq!(
            select_skillhub_search_slug(&root, "superpowers-tdd"),
            Some("superpowers-tdd".into())
        );
        assert_eq!(select_skillhub_search_slug(&root, "missing"), None);
    }

    #[tokio::test]
    async fn install_skillhub_package_skills_uses_existing_available_skill() {
        let paths = make_paths();
        let skill_dir = paths.user_skills_dir.join("superpowers-tdd");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: superpowers-tdd\ndescription: TDD workflow\n---\n# Superpowers TDD",
        )
        .await
        .unwrap();

        let installed = install_skillhub_package_skills(&paths, &["superpowers-tdd".into()])
            .await
            .unwrap();

        assert_eq!(installed.installed_skill_names, vec!["superpowers-tdd"]);
        assert!(installed.errors.is_empty());
    }

    #[tokio::test]
    async fn install_skillhub_package_skills_keeps_successes_when_one_child_fails() {
        let paths = make_paths();
        let skill_dir = paths.user_skills_dir.join("superpowers-tdd");
        tokio::fs::create_dir_all(&skill_dir).await.unwrap();
        tokio::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: superpowers-tdd\ndescription: TDD workflow\n---\n# Superpowers TDD",
        )
        .await
        .unwrap();

        let installed = install_skillhub_package_skills(
            &paths,
            &["../missing-child".into(), "superpowers-tdd".into()],
        )
        .await
        .unwrap();

        assert_eq!(installed.installed_skill_names, vec!["superpowers-tdd"]);
        assert_eq!(installed.errors.len(), 1);
        assert_eq!(installed.errors[0].skill_slug, "../missing-child");
        assert!(installed.errors[0].error.contains("invalid SkillHub skill slug"));
    }
}
