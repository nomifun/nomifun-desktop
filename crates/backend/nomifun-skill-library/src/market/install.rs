//! Controlled skill-market installation.
//!
//! This module deliberately accepts only a structured market identity
//! (`source` + `id` + canonical page URL). It never accepts or executes the
//! ranking entry's display-only `install_command`. Download URLs are either
//! constructed from a validated market slug or resolved from a freshly
//! fetched allowlisted market response.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nomifun_api_types::{SkillMarketInstallRequest, SkillMarketInstallResponse};
use nomifun_common::AppError;
use reqwest::header::ACCEPT;
use tracing::{error, warn};

use crate::skill_service::{self, SkillPaths};

use super::client::{
    MAX_MARKET_SKILL_ZIP_BYTES, build_market_client, map_market_fetch_error,
    read_market_body, read_market_bytes,
};
use super::parse::{is_market_slug, market_ref_suffix};
use super::{CLAWHUB_SOURCE, LOOPHUB_RANKING_URL, LOOPHUB_SOURCE, SKILLHUB_SOURCE};

const CLAWHUB_DOWNLOAD_URL: &str = "https://clawhub.ai/api/v1/download";
const SKILLHUB_DOWNLOAD_URL: &str = "https://api.skillhub.cn/api/v1/download";
const LOOPHUB_DETAIL_BASE_URL: &str = "https://api.cocoloop.cn/api/v1/store/skills/";
const LOOPHUB_DOWNLOAD_BASE_URL: &str = "https://dl.cocoloop.cn/bss/skills/";
const MARKET_IMPORT_DIR: &str = ".market-import";
const MARKET_ARCHIVE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

static MARKET_INSTALL_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static MARKET_INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug)]
enum DownloadPlan {
    Direct(reqwest::Url),
    LoopHubItem(u64),
}

#[derive(Debug)]
struct ResolvedMarketSkill {
    source: &'static str,
    identity: String,
    download: DownloadPlan,
}

/// Download and install one skill-market entry without involving an Agent
/// session or executing a marketplace-provided command.
pub async fn install_market_skill(
    paths: &SkillPaths,
    request: SkillMarketInstallRequest,
) -> Result<SkillMarketInstallResponse, AppError> {
    let resolved = resolve_market_skill(&request)?;
    let client = build_market_client()?;
    let download_url = match resolved.download {
        DownloadPlan::Direct(url) => url,
        DownloadPlan::LoopHubItem(item_id) => {
            let detail_url = loophub_detail_url(item_id)?;
            let detail_result = match read_market_body(&client, detail_url.as_str()).await {
                Ok(body) => resolve_loophub_download_url(&body, item_id),
                Err(error) => Err(error),
            };
            match detail_result {
                Ok(url) => url,
                Err(detail_error) => {
                    warn!(%item_id, error = %detail_error, "LoopHub detail did not contain a usable archive; trying the current ranking feed");
                    let ranking = read_market_body(&client, LOOPHUB_RANKING_URL).await?;
                    resolve_loophub_download_url(&ranking, item_id)?
                }
            }
        }
    };
    let label = format!("{} skill '{}' archive", resolved.source, resolved.identity);
    let archive = download_market_archive(&client, download_url, &label).await?;

    // Serialize the conflict preflight and publish phase for installs coming
    // through this endpoint. Staging/download happens outside the lock.
    let _guard = MARKET_INSTALL_LOCK.lock().await;
    let skill_names = install_archive_without_overwrite(paths, &archive).await?;
    Ok(SkillMarketInstallResponse { skill_names })
}

fn resolve_market_skill(request: &SkillMarketInstallRequest) -> Result<ResolvedMarketSkill, AppError> {
    match request.source.trim().to_ascii_lowercase().as_str() {
        CLAWHUB_SOURCE => resolve_clawhub_skill(request),
        SKILLHUB_SOURCE => resolve_skillhub_skill(request),
        LOOPHUB_SOURCE => resolve_loophub_skill(request),
        other => Err(AppError::BadRequest(format!(
            "unsupported installable skill market source: {other}"
        ))),
    }
}

fn resolve_clawhub_skill(request: &SkillMarketInstallRequest) -> Result<ResolvedMarketSkill, AppError> {
    let suffix = market_ref_suffix(&request.id, CLAWHUB_SOURCE)
        .ok_or_else(|| AppError::BadRequest("invalid ClawHub skill id".into()))?;
    let (id_owner, id_slug) = parse_owner_slug(&suffix, false)
        .ok_or_else(|| AppError::BadRequest("invalid ClawHub skill id".into()))?;
    let segments = canonical_https_segments(&request.url, &["clawhub.ai"])?;
    let (url_owner, url_slug) = match segments.as_slice() {
        [owner, kind, slug] if kind == "skills" => (owner.as_str(), slug.as_str()),
        _ => return Err(AppError::BadRequest("invalid ClawHub skill URL".into())),
    };
    require_same_market_identity(&id_owner, &id_slug, url_owner, url_slug, "ClawHub")?;

    // ClawHub slugs are not globally unique. The owner handle is mandatory;
    // omitting it makes the public API return 409 for ambiguous slugs.
    let download = reqwest::Url::parse_with_params(
        CLAWHUB_DOWNLOAD_URL,
        &[("slug", id_slug.as_str()), ("ownerHandle", id_owner.as_str())],
    )
    .map_err(|error| AppError::Internal(format!("invalid ClawHub download URL: {error}")))?;
    Ok(ResolvedMarketSkill {
        source: CLAWHUB_SOURCE,
        identity: format!("{id_owner}/{id_slug}"),
        download: DownloadPlan::Direct(download),
    })
}

fn resolve_skillhub_skill(request: &SkillMarketInstallRequest) -> Result<ResolvedMarketSkill, AppError> {
    let suffix = market_ref_suffix(&request.id, SKILLHUB_SOURCE)
        .ok_or_else(|| AppError::BadRequest("invalid SkillHub skill id".into()))?;
    let (id_owner, id_slug) = parse_owner_slug(&suffix, true)
        .ok_or_else(|| AppError::BadRequest("invalid SkillHub skill id".into()))?;
    let parsed = reqwest::Url::parse(request.url.trim())
        .map_err(|_| AppError::BadRequest("invalid SkillHub skill URL".into()))?;
    require_plain_https_url(&parsed, &["skillhub.cn"])?;
    let segments = parsed
        .path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    let (url_owner, url_slug) = match (parsed.host_str(), segments.as_slice()) {
        (Some("skillhub.cn"), [kind, owner, slug]) if kind == "skills" => (owner.as_str(), slug.as_str()),
        _ => return Err(AppError::BadRequest("invalid SkillHub skill URL".into())),
    };
    require_same_market_identity(&id_owner, &id_slug, url_owner, url_slug, "SkillHub")?;

    let qualified_slug = format!("@{id_owner}/{id_slug}");
    let download = reqwest::Url::parse_with_params(SKILLHUB_DOWNLOAD_URL, &[("slug", qualified_slug.as_str())])
        .map_err(|error| AppError::Internal(format!("invalid SkillHub download URL: {error}")))?;
    Ok(ResolvedMarketSkill {
        source: SKILLHUB_SOURCE,
        identity: format!("{id_owner}/{id_slug}"),
        download: DownloadPlan::Direct(download),
    })
}

fn resolve_loophub_skill(request: &SkillMarketInstallRequest) -> Result<ResolvedMarketSkill, AppError> {
    let suffix = market_ref_suffix(&request.id, LOOPHUB_SOURCE)
        .ok_or_else(|| AppError::BadRequest("invalid LoopHub skill id".into()))?;
    let item_id = suffix
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && id.to_string() == suffix)
        .ok_or_else(|| AppError::BadRequest("invalid LoopHub skill id".into()))?;
    let segments = canonical_https_segments(&request.url, &["hub.cocoloop.cn"])?;
    match segments.as_slice() {
        [kind, url_id] if kind == "skills" && url_id == &suffix => {}
        _ => return Err(AppError::BadRequest("invalid LoopHub skill URL".into())),
    }
    Ok(ResolvedMarketSkill {
        source: LOOPHUB_SOURCE,
        identity: suffix,
        download: DownloadPlan::LoopHubItem(item_id),
    })
}

fn parse_owner_slug(value: &str, includes_skills_segment: bool) -> Option<(String, String)> {
    let parts = value.split('/').collect::<Vec<_>>();
    let (owner, slug) = match (includes_skills_segment, parts.as_slice()) {
        (false, [owner, slug]) => (*owner, *slug),
        (true, [owner, "skills", slug]) => (*owner, *slug),
        _ => return None,
    };
    if is_market_slug(owner) && is_market_slug(slug) {
        Some((owner.to_string(), slug.to_string()))
    } else {
        None
    }
}

fn canonical_https_segments(url: &str, hosts: &[&str]) -> Result<Vec<String>, AppError> {
    let parsed = reqwest::Url::parse(url.trim())
        .map_err(|_| AppError::BadRequest("invalid skill market URL".into()))?;
    require_plain_https_url(&parsed, hosts)?;
    Ok(parsed
        .path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).map(str::to_string).collect())
        .unwrap_or_default())
}

fn require_plain_https_url(url: &reqwest::Url, hosts: &[&str]) -> Result<(), AppError> {
    let host_allowed = url
        .host_str()
        .is_some_and(|host| hosts.iter().any(|allowed| host.eq_ignore_ascii_case(allowed)));
    if url.scheme() != "https"
        || !host_allowed
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::BadRequest("untrusted skill market URL".into()));
    }
    Ok(())
}

fn require_same_market_identity(
    id_owner: &str,
    id_slug: &str,
    url_owner: &str,
    url_slug: &str,
    source: &str,
) -> Result<(), AppError> {
    if !is_market_slug(url_owner)
        || !is_market_slug(url_slug)
        || !id_owner.eq_ignore_ascii_case(url_owner)
        || !id_slug.eq_ignore_ascii_case(url_slug)
    {
        return Err(AppError::BadRequest(format!(
            "{source} skill id and URL refer to different skills"
        )));
    }
    Ok(())
}

fn resolve_loophub_download_url(body: &str, item_id: u64) -> Result<reqwest::Url, AppError> {
    let root = serde_json::from_str::<serde_json::Value>(body)
        .map_err(|error| AppError::BadGateway(format!("LoopHub ranking JSON parse failed: {error}")))?;
    let item = if let Some(items) = root.pointer("/data/items").and_then(serde_json::Value::as_array) {
        items
            .iter()
            .find(|item| item.get("id").and_then(serde_json::Value::as_u64) == Some(item_id))
            .ok_or_else(|| AppError::NotFound(format!("LoopHub skill '{item_id}' not found")))?
    } else {
        root.get("data").filter(|value| value.is_object()).unwrap_or(&root)
    };
    let returned_id = item
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| AppError::BadGateway("LoopHub skill response missing its item id".into()))?;
    if returned_id != item_id {
        return Err(AppError::BadGateway(format!(
            "LoopHub skill response id mismatch: expected '{item_id}', got '{returned_id}'"
        )));
    }

    let url = if let Some(raw) = item
        .get("download_url")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 2_048)
    {
        reqwest::Url::parse(raw)
            .map_err(|_| AppError::BadGateway("LoopHub returned an invalid download URL".into()))?
    } else {
        let asset_name = item
            .get("asset_name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| is_safe_zip_asset_name(value))
            .ok_or_else(|| AppError::BadGateway("LoopHub skill response missing a safe ZIP asset name".into()))?;
        let mut url = reqwest::Url::parse(LOOPHUB_DOWNLOAD_BASE_URL)
            .map_err(|error| AppError::Internal(format!("invalid LoopHub download URL base: {error}")))?;
        url.path_segments_mut()
            .map_err(|_| AppError::Internal("invalid LoopHub download URL base".into()))?
            .pop_if_empty()
            .push(asset_name);
        url
    };
    let trusted = url.scheme() == "https"
        && url.host_str() == Some("dl.cocoloop.cn")
        && url.path().starts_with("/bss/skills/")
        && url.path().len() > "/bss/skills/".len()
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none();
    if !trusted {
        return Err(AppError::BadGateway(
            "LoopHub returned an untrusted download URL".into(),
        ));
    }
    Ok(url)
}

fn loophub_detail_url(item_id: u64) -> Result<reqwest::Url, AppError> {
    let mut url = reqwest::Url::parse(LOOPHUB_DETAIL_BASE_URL)
        .map_err(|error| AppError::Internal(format!("invalid LoopHub detail URL base: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| AppError::Internal("invalid LoopHub detail URL base".into()))?
        .pop_if_empty()
        .push(&item_id.to_string());
    Ok(url)
}

fn is_safe_zip_asset_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !value.is_empty()
        && value.len() <= 255
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && value.to_ascii_lowercase().ends_with(".zip")
        && !value.contains("..")
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
}

async fn download_market_archive(
    client: &reqwest::Client,
    url: reqwest::Url,
    label: &str,
) -> Result<Vec<u8>, AppError> {
    let mut response = client
        .get(url)
        .timeout(MARKET_ARCHIVE_REQUEST_TIMEOUT)
        .header(ACCEPT, "application/zip,application/octet-stream,*/*")
        .send()
        .await
        .map_err(map_market_fetch_error)?;
    let archive = read_market_bytes(&mut response, MAX_MARKET_SKILL_ZIP_BYTES, label).await?;
    if !looks_like_zip(&archive) {
        return Err(AppError::BadGateway(format!(
            "{label} did not return a ZIP archive"
        )));
    }
    Ok(archive)
}

fn looks_like_zip(bytes: &[u8]) -> bool {
    matches!(bytes.get(..4), Some(b"PK\x03\x04" | b"PK\x05\x06" | b"PK\x07\x08"))
}

/// Import into an isolated library first, then publish validated skill
/// directories with same-filesystem renames. Existing library entries are
/// rejected before any publish; the ordinary import API's replace behavior is
/// intentionally not used for marketplace installs.
async fn install_archive_without_overwrite(
    paths: &SkillPaths,
    archive: &[u8],
) -> Result<Vec<String>, AppError> {
    tokio::fs::create_dir_all(&paths.user_skills_dir)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let temp_root = paths.user_skills_dir.join(MARKET_IMPORT_DIR);
    tokio::fs::create_dir_all(&temp_root)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let job_dir = match create_market_job_dir(&temp_root).await {
        Ok(job_dir) => job_dir,
        Err(error) => {
            let _ = tokio::fs::remove_dir(&temp_root).await;
            return Err(error);
        }
    };

    let result = async {
        let archive_path = job_dir.join("download.zip");
        tokio::fs::write(&archive_path, archive)
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;

        let staged_library = job_dir.join("staged-skills");
        let mut staged_paths = paths.clone();
        staged_paths.user_skills_dir = staged_library.clone();
        let mut names = skill_service::import_skills_with_symlink(&staged_paths, &archive_path)
            .await
            .map_err(AppError::from)?;
        names.sort();
        names.dedup();
        if names.is_empty() {
            return Err(AppError::BadRequest(
                "market archive did not contain an importable skill".into(),
            ));
        }

        preflight_market_publish(paths, &staged_library, &names).await?;
        publish_staged_skills(paths, &staged_library, &names).await?;
        Ok(names)
    }
    .await;

    cleanup_market_job(&job_dir, &temp_root).await;
    result
}

async fn create_market_job_dir(temp_root: &Path) -> Result<PathBuf, AppError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = MARKET_INSTALL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let job_dir = temp_root.join(format!(
        "install-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    tokio::fs::create_dir(&job_dir)
        .await
        .map_err(|error| AppError::Internal(format!("create market import staging directory: {error}")))?;
    Ok(job_dir)
}

async fn preflight_market_publish(
    paths: &SkillPaths,
    staged_library: &Path,
    names: &[String],
) -> Result<(), AppError> {
    let available_names = skill_service::list_available_skills(paths)
        .await?
        .into_iter()
        .map(|skill| skill.name.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();

    for name in names {
        let staged = staged_library.join(name);
        if !path_entry_exists(&staged).await?
            || !tokio::fs::metadata(staged.join("SKILL.md"))
                .await
                .is_ok_and(|metadata| metadata.is_file())
        {
            return Err(AppError::Internal(format!(
                "validated staged skill '{name}' is missing SKILL.md"
            )));
        }
        let target = paths.user_skills_dir.join(name);
        if available_names.contains(&name.to_ascii_lowercase()) || path_entry_exists(&target).await? {
            return Err(AppError::Conflict(format!(
                "skill '{name}' is already installed; marketplace install will not overwrite it"
            )));
        }
    }
    Ok(())
}

async fn publish_staged_skills(
    paths: &SkillPaths,
    staged_library: &Path,
    names: &[String],
) -> Result<(), AppError> {
    let mut published = Vec::<(PathBuf, PathBuf)>::new();
    for name in names {
        let staged = staged_library.join(name);
        let target = paths.user_skills_dir.join(name);
        if path_entry_exists(&target).await? {
            rollback_market_publish(&mut published).await;
            return Err(AppError::Conflict(format!(
                "skill '{name}' was installed concurrently; marketplace install did not overwrite it"
            )));
        }
        if let Err(error) = tokio::fs::rename(&staged, &target).await {
            rollback_market_publish(&mut published).await;
            return Err(AppError::Internal(format!(
                "publish market skill '{name}': {error}"
            )));
        }
        published.push((target, staged));
        let published_manifest = published
            .last()
            .expect("published entry was just pushed")
            .0
            .join("SKILL.md");
        if !tokio::fs::metadata(&published_manifest)
            .await
            .is_ok_and(|metadata| metadata.is_file())
        {
            rollback_market_publish(&mut published).await;
            return Err(AppError::Internal(format!(
                "published market skill '{name}' is missing SKILL.md"
            )));
        }
    }
    Ok(())
}

async fn rollback_market_publish(published: &mut Vec<(PathBuf, PathBuf)>) {
    while let Some((target, staged)) = published.pop() {
        if let Err(error) = tokio::fs::rename(&target, &staged).await {
            error!(
                target = %target.display(),
                staged = %staged.display(),
                %error,
                "failed to roll back partially published market skill"
            );
        }
    }
}

async fn path_entry_exists(path: &Path) -> Result<bool, AppError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AppError::Internal(error.to_string())),
    }
}

async fn cleanup_market_job(job_dir: &Path, temp_root: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(job_dir).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!(path = %job_dir.display(), %error, "failed to clean market skill import staging directory");
    }
    if let Err(error) = tokio::fs::remove_dir(temp_root).await
        && !matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
        )
    {
        warn!(path = %temp_root.display(), %error, "failed to remove empty market import root");
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    use super::*;

    fn make_paths(temp: &TempDir) -> SkillPaths {
        SkillPaths {
            data_dir: temp.path().to_path_buf(),
            user_skills_dir: temp.path().join("skills"),
            cron_skills_dir: temp.path().join("cron").join("skills"),
            builtin_skills_dir: temp.path().join("builtin-skills"),
            builtin_rules_dir: temp.path().join("builtin-rules"),
        }
    }

    fn skill_archive(name: &str, marker: &str) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            zip.start_file(format!("bundle/{name}/SKILL.md"), SimpleFileOptions::default())
                .unwrap();
            write!(
                zip,
                "---\nname: {name}\ndescription: Test skill\n---\n# {marker}\n"
            )
            .unwrap();
            zip.finish().unwrap();
        }
        bytes.into_inner()
    }

    fn archive_without_manifest() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            zip.start_file("bundle/README.md", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"not a skill").unwrap();
            zip.finish().unwrap();
        }
        bytes.into_inner()
    }

    fn request(source: &str, id: &str, url: &str) -> SkillMarketInstallRequest {
        SkillMarketInstallRequest {
            source: source.into(),
            id: id.into(),
            url: url.into(),
        }
    }

    #[test]
    fn validates_all_three_market_identity_shapes() {
        let clawhub = resolve_market_skill(&request(
            "clawhub",
            "clawhub:owner/demo",
            "https://clawhub.ai/owner/skills/demo",
        ))
        .unwrap();
        let DownloadPlan::Direct(url) = clawhub.download else {
            panic!("ClawHub must use a backend-constructed direct URL");
        };
        assert_eq!(
            url.as_str(),
            "https://clawhub.ai/api/v1/download?slug=demo&ownerHandle=owner"
        );

        let skillhub = resolve_market_skill(&request(
            "skillhub",
            "skillhub:owner/skills/demo",
            "https://skillhub.cn/skills/owner/demo",
        ))
        .unwrap();
        let DownloadPlan::Direct(url) = skillhub.download else {
            panic!("SkillHub must use a backend-constructed direct URL");
        };
        assert_eq!(url.host_str(), Some("api.skillhub.cn"));
        assert_eq!(
            url.query_pairs().find(|(key, _)| key == "slug").map(|(_, value)| value.into_owned()),
            Some("@owner/demo".into())
        );

        let loophub = resolve_market_skill(&request(
            "loophub",
            "loophub:42",
            "https://hub.cocoloop.cn/skills/42",
        ))
        .unwrap();
        assert!(matches!(loophub.download, DownloadPlan::LoopHubItem(42)));
    }

    #[test]
    fn rejects_cross_source_off_host_and_mismatched_identities() {
        for invalid in [
            request(
                "skillhub",
                "skillhub:owner/skills/demo",
                "https://evil.example/skills/owner/demo",
            ),
            request(
                "skillhub",
                "skillhub:owner/skills/demo",
                "https://skillhub.cn/skills/owner/other",
            ),
            request(
                "skillhub",
                "skillhub:owner/skills/demo",
                "https://www.skills.sh/owner/skills/demo",
            ),
            request(
                "clawhub",
                "skillhub:owner/skills/demo",
                "https://clawhub.ai/owner/skills/demo",
            ),
            request(
                "loophub",
                "loophub:042",
                "https://hub.cocoloop.cn/skills/042",
            ),
        ] {
            assert!(resolve_market_skill(&invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn loophub_download_is_resolved_only_from_the_trusted_fresh_feed() {
        let trusted = serde_json::json!({
            "data": { "items": [
                { "id": 42, "download_url": "https://dl.cocoloop.cn/bss/skills/demo.zip?version=1" }
            ] }
        })
        .to_string();
        assert_eq!(
            resolve_loophub_download_url(&trusted, 42).unwrap().as_str(),
            "https://dl.cocoloop.cn/bss/skills/demo.zip?version=1"
        );

        let untrusted = serde_json::json!({
            "data": { "items": [
                { "id": 42, "download_url": "https://dl.cocoloop.cn.evil.example/bss/skills/demo.zip" }
            ] }
        })
        .to_string();
        assert!(matches!(
            resolve_loophub_download_url(&untrusted, 42),
            Err(AppError::BadGateway(_))
        ));

        let missing_identity = serde_json::json!({
            "data": {
                "asset_name": "demo.zip",
                "download_url": ""
            }
        })
        .to_string();
        assert!(matches!(
            resolve_loophub_download_url(&missing_identity, 42),
            Err(AppError::BadGateway(_))
        ));

        let detail = serde_json::json!({
            "data": {
                "id": 42,
                "asset_name": "demo-v1.2.3.zip",
                "download_url": ""
            }
        })
        .to_string();
        assert_eq!(
            resolve_loophub_download_url(&detail, 42).unwrap().as_str(),
            "https://dl.cocoloop.cn/bss/skills/demo-v1.2.3.zip"
        );

        for unsafe_name in ["../demo.zip", "folder/demo.zip", "demo.exe", "demo..zip"] {
            let body = serde_json::json!({
                "data": { "id": 42, "asset_name": unsafe_name, "download_url": "" }
            })
            .to_string();
            assert!(resolve_loophub_download_url(&body, 42).is_err(), "{unsafe_name}");
        }
    }

    #[test]
    fn archive_signature_rejects_json_handoffs_and_html_errors() {
        assert!(looks_like_zip(b"PK\x03\x04rest"));
        assert!(!looks_like_zip(br#"{"sourceRef":"public-github"}"#));
        assert!(!looks_like_zip(b"<html>upstream error</html>"));
    }

    #[tokio::test]
    async fn staged_market_install_publishes_and_cleans_temp_files() {
        let temp = TempDir::new().unwrap();
        let paths = make_paths(&temp);
        let names = install_archive_without_overwrite(&paths, &skill_archive("demo", "installed"))
            .await
            .unwrap();
        assert_eq!(names, vec!["demo"]);
        assert!(paths.user_skills_dir.join("demo/SKILL.md").is_file());
        assert!(!paths.user_skills_dir.join(MARKET_IMPORT_DIR).exists());
    }

    #[tokio::test]
    async fn conflict_preserves_existing_skill_and_cleans_staging() {
        let temp = TempDir::new().unwrap();
        let paths = make_paths(&temp);
        let existing = paths.user_skills_dir.join("demo");
        tokio::fs::create_dir_all(&existing).await.unwrap();
        tokio::fs::write(
            existing.join("SKILL.md"),
            "---\nname: demo\ndescription: Original\n---\n# original\n",
        )
        .await
        .unwrap();

        let error = install_archive_without_overwrite(&paths, &skill_archive("demo", "replacement"))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)), "{error}");
        let content = tokio::fs::read_to_string(existing.join("SKILL.md")).await.unwrap();
        assert!(content.contains("# original"));
        assert!(!content.contains("# replacement"));
        assert!(!paths.user_skills_dir.join(MARKET_IMPORT_DIR).exists());
    }

    #[tokio::test]
    async fn invalid_archive_failure_cleans_download_and_extraction_staging() {
        let temp = TempDir::new().unwrap();
        let paths = make_paths(&temp);
        assert!(install_archive_without_overwrite(&paths, b"not a zip").await.is_err());
        assert!(!paths.user_skills_dir.join(MARKET_IMPORT_DIR).exists());

        assert!(
            install_archive_without_overwrite(&paths, &archive_without_manifest())
                .await
                .is_err()
        );
        assert!(!paths.user_skills_dir.join(MARKET_IMPORT_DIR).exists());
        assert!(
            tokio::fs::read_dir(&paths.user_skills_dir)
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_none(),
            "a rejected archive must not leave a partial skill directory"
        );
    }

    #[tokio::test]
    #[ignore = "requires the public SkillHub download service"]
    async fn live_skillhub_dev_expert_installs_into_the_managed_library() {
        let temp = TempDir::new().unwrap();
        let paths = make_paths(&temp);
        let response = install_market_skill(
            &paths,
            request(
                "skillhub",
                "skillhub:indiv-ebandao/skills/dev-expert",
                "https://skillhub.cn/skills/indiv-ebandao/dev-expert",
            ),
        )
        .await
        .unwrap();

        assert!(!response.skill_names.is_empty());
        for name in response.skill_names {
            assert!(
                paths.user_skills_dir.join(name).join("SKILL.md").is_file(),
                "every returned skill name must point to a canonical installed SKILL.md"
            );
        }
        assert!(!paths.user_skills_dir.join(MARKET_IMPORT_DIR).exists());
    }
}
