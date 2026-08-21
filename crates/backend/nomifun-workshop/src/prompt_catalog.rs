//! Auditable, offline-first Creative Studio prompt catalog.
//!
//! Catalog entries are synchronized from a fixed allow-list of upstream
//! repositories. They are cached as one versioned JSON snapshot under the
//! Workshop data directory; they are deliberately not inserted into the
//! user's asset library. Every item carries its repository and license so the
//! UI can preserve attribution when a prompt is copied or saved as an asset.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nomifun_common::{AppError, now_ms};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{WORKSHOP_REL_DIR, fsio};

const PROMPT_CATALOG_SCHEMA: &str = "nomifun.creative-studio.prompt-catalog";
const PROMPT_CATALOG_VERSION: u32 = 1;
const PROMPT_CATALOG_FILE: &str = "catalog-v1.json";
const PROMPT_CATALOG_STALE_AFTER_MS: i64 = 24 * 60 * 60 * 1_000;
const MAX_PROMPT_CATALOG_BYTES: usize = 64 * 1024 * 1024;
const MAX_PROMPT_SOURCE_BYTES: usize = 24 * 1024 * 1024;
const MAX_PROMPTS: usize = 20_000;
const MAX_PROMPTS_PER_SOURCE: usize = 5_000;

const GPT_IMAGE_2_RAW_BASE: &str =
    "https://raw.githubusercontent.com/tigerowo/awesome-gpt-image-2-prompts/main";
const AWESOME_GPT_IMAGE_RAW_BASE: &str =
    "https://raw.githubusercontent.com/ZeroLu/awesome-gpt-image/main";
const AWESOME_GPT4O_IMAGE_PROMPTS_BASE: &str =
    "https://raw.githubusercontent.com/ImgEdify/Awesome-GPT4o-Image-Prompts/main";
const XIAN_YU_AWESOME_GPT_IMAGE_2_RAW_BASE: &str =
    "https://raw.githubusercontent.com/xianyu110/awesome-gptimage2/main";
const YOU_MIND_GPT_IMAGE_2_RAW_BASE: &str =
    "https://raw.githubusercontent.com/YouMind-OpenLab/awesome-gpt-image-2/main";
const YOU_MIND_NANO_BANANA_PRO_RAW_BASE: &str =
    "https://raw.githubusercontent.com/YouMind-OpenLab/awesome-nano-banana-pro-prompts/main";
const DAVID_WU_GPT_IMAGE_2_RAW_BASE: &str =
    "https://raw.githubusercontent.com/davidwuw0811-boop/awesome-gpt-image2-prompts/main";

const GPT_IMAGE_2_CASE_FILES: &[&str] = &[
    "README.md",
    "cases/ad-creative.md",
    "cases/character.md",
    "cases/comparison.md",
    "cases/ecommerce.md",
    "cases/portrait.md",
    "cases/poster.md",
    "cases/ui.md",
];

#[derive(Clone, Copy)]
struct PromptSourceDefinition {
    code: &'static str,
    name: &'static str,
    description: &'static str,
    repository_url: &'static str,
    license: &'static str,
    license_url: &'static str,
}

const PROMPT_SOURCES: &[PromptSourceDefinition] = &[
    PromptSourceDefinition {
        code: "gpt-image-2-prompts",
        name: "GPT Image 2 Prompts",
        description: "GPT Image 2 案例提示词",
        repository_url: "https://github.com/tigerowo/awesome-gpt-image-2-prompts",
        license: "CC0-1.0",
        license_url: "https://github.com/tigerowo/awesome-gpt-image-2-prompts/blob/main/LICENSE",
    },
    PromptSourceDefinition {
        code: "awesome-gpt-image",
        name: "Awesome GPT Image",
        description: "中文 GPT Image 提示词",
        repository_url: "https://github.com/ZeroLu/awesome-gpt-image",
        license: "MIT",
        license_url: "https://github.com/ZeroLu/awesome-gpt-image/blob/main/LICENSE",
    },
    PromptSourceDefinition {
        code: "awesome-gpt4o-image-prompts",
        name: "Awesome GPT-4o Image Prompts",
        description: "GPT-4o 图像提示词",
        repository_url: "https://github.com/ImgEdify/Awesome-GPT4o-Image-Prompts",
        license: "MIT",
        license_url: "https://github.com/ImgEdify/Awesome-GPT4o-Image-Prompts/blob/main/LICENSE",
    },
    PromptSourceDefinition {
        code: "xianyu-awesome-gptimage2",
        name: "Xianyu Awesome GPT Image 2",
        description: "GPT Image 2 中文案例与来源链接",
        repository_url: "https://github.com/xianyu110/awesome-gptimage2",
        license: "Authorized upstream collection",
        license_url: "https://github.com/xianyu110/awesome-gptimage2",
    },
    PromptSourceDefinition {
        code: "youmind-gpt-image-2",
        name: "YouMind GPT Image 2",
        description: "YouMind OpenLab GPT Image 2 提示词",
        repository_url: "https://github.com/YouMind-OpenLab/awesome-gpt-image-2",
        license: "CC-BY-4.0",
        license_url: "https://github.com/YouMind-OpenLab/awesome-gpt-image-2/blob/main/LICENSE",
    },
    PromptSourceDefinition {
        code: "youmind-nano-banana-pro",
        name: "YouMind Nano Banana Pro",
        description: "YouMind OpenLab Nano Banana Pro 提示词",
        repository_url: "https://github.com/YouMind-OpenLab/awesome-nano-banana-pro-prompts",
        license: "CC-BY-4.0",
        license_url: "https://github.com/YouMind-OpenLab/awesome-nano-banana-pro-prompts/blob/main/LICENSE",
    },
    PromptSourceDefinition {
        code: "davidwu-gpt-image2-prompts",
        name: "Awesome GPT Image 2 Prompts",
        description: "结构化 GPT Image 2 提示词目录",
        repository_url: "https://github.com/davidwuw0811-boop/awesome-gpt-image2-prompts",
        license: "MIT",
        license_url: "https://github.com/davidwuw0811-boop/awesome-gpt-image2-prompts",
    },
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativePromptCatalogItem {
    pub id: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub prompt: String,
    pub tags: Vec<String>,
    pub category: String,
    pub source_url: String,
    pub license: String,
    pub license_url: String,
    pub preview: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativePromptCatalogSource {
    pub code: String,
    pub name: String,
    pub description: String,
    pub repository_url: String,
    pub license: String,
    pub license_url: String,
    pub item_count: usize,
    pub last_synced_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativePromptCatalogPage {
    pub items: Vec<CreativePromptCatalogItem>,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub total: usize,
    pub synced_at: Option<i64>,
    pub stale: bool,
    pub sources: Vec<CreativePromptCatalogSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PromptCatalogSnapshot {
    schema: String,
    version: u32,
    synced_at: i64,
    items: Vec<CreativePromptCatalogItem>,
    sources: Vec<CreativePromptCatalogSource>,
}

impl PromptCatalogSnapshot {
    fn empty() -> Self {
        Self {
            schema: PROMPT_CATALOG_SCHEMA.to_owned(),
            version: PROMPT_CATALOG_VERSION,
            synced_at: 0,
            items: Vec::new(),
            sources: PROMPT_SOURCES
                .iter()
                .map(|source| source.status(0, None, None))
                .collect(),
        }
    }

    fn stale(&self, current_ms: i64) -> bool {
        self.synced_at <= 0
            || current_ms.saturating_sub(self.synced_at) >= PROMPT_CATALOG_STALE_AFTER_MS
    }

    fn into_page(self, current_ms: i64) -> CreativePromptCatalogPage {
        let stale = self.stale(current_ms);
        let mut tags = BTreeSet::new();
        let mut categories = BTreeSet::new();
        for item in &self.items {
            categories.insert(item.category.clone());
            tags.extend(item.tags.iter().cloned());
        }
        let total = self.items.len();
        CreativePromptCatalogPage {
            items: self.items,
            tags: tags.into_iter().collect(),
            categories: categories.into_iter().collect(),
            total,
            synced_at: (self.synced_at > 0).then_some(self.synced_at),
            stale,
            sources: self.sources,
        }
    }
}

impl PromptSourceDefinition {
    fn status(
        self,
        item_count: usize,
        last_synced_at: Option<i64>,
        last_error: Option<String>,
    ) -> CreativePromptCatalogSource {
        CreativePromptCatalogSource {
            code: self.code.to_owned(),
            name: self.name.to_owned(),
            description: self.description.to_owned(),
            repository_url: self.repository_url.to_owned(),
            license: self.license.to_owned(),
            license_url: self.license_url.to_owned(),
            item_count,
            last_synced_at,
            last_error,
        }
    }
}

pub(crate) struct PromptCatalogService {
    cache_dir: PathBuf,
    client: Client,
    sync_lock: Mutex<()>,
}

impl PromptCatalogService {
    pub(crate) fn start(data_dir: &Path) -> Self {
        let client = Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent("NomiFun-Creative-Studio-Prompt-Catalog/1")
            .build()
            .expect("static Creative Studio prompt HTTP client configuration must be valid");
        Self {
            cache_dir: data_dir.join(WORKSHOP_REL_DIR).join("prompts"),
            client,
            sync_lock: Mutex::new(()),
        }
    }

    pub(crate) async fn list(&self) -> Result<CreativePromptCatalogPage, AppError> {
        let snapshot = self.load_snapshot().await?;
        Ok(snapshot.into_page(now_ms()))
    }

    pub(crate) async fn sync(
        &self,
        force: bool,
    ) -> Result<CreativePromptCatalogPage, AppError> {
        let _guard = self.sync_lock.lock().await;
        let previous = self.load_snapshot().await?;
        let current_ms = now_ms();
        if !force && !previous.items.is_empty() && !previous.stale(current_ms) {
            return Ok(previous.into_page(current_ms));
        }

        let (
            gpt_image_2,
            awesome_gpt_image,
            awesome_gpt4o,
            xianyu,
            youmind_gpt_image_2,
            youmind_nano_banana,
            david_wu,
        ) = tokio::join!(
            build_gpt_image_2_prompts(&self.client),
            build_awesome_gpt_image_prompts(&self.client),
            build_awesome_gpt4o_image_prompts(&self.client),
            build_xianyu_awesome_gpt_image_2_prompts(&self.client),
            build_youmind_prompts(
                &self.client,
                YOU_MIND_GPT_IMAGE_2_RAW_BASE,
                "youmind-gpt-image-2",
                "gpt-image-2",
            ),
            build_youmind_prompts(
                &self.client,
                YOU_MIND_NANO_BANANA_PRO_RAW_BASE,
                "youmind-nano-banana-pro",
                "nano-banana-pro",
            ),
            build_david_wu_gpt_image_2_prompts(&self.client),
        );

        let results = vec![
            gpt_image_2,
            awesome_gpt_image,
            awesome_gpt4o,
            xianyu,
            youmind_gpt_image_2,
            youmind_nano_banana,
            david_wu,
        ];
        let mut previous_by_source: BTreeMap<String, Vec<CreativePromptCatalogItem>> =
            BTreeMap::new();
        for item in previous.items {
            previous_by_source
                .entry(item.category.clone())
                .or_default()
                .push(item);
        }

        let mut items = Vec::new();
        let mut sources = Vec::with_capacity(PROMPT_SOURCES.len());
        let mut successful_sources = 0usize;
        let mut failures = Vec::new();
        for (source, result) in PROMPT_SOURCES.iter().copied().zip(results) {
            match result {
                Ok(fresh) if !fresh.is_empty() => {
                    successful_sources += 1;
                    sources.push(source.status(fresh.len(), Some(current_ms), None));
                    items.extend(fresh);
                }
                Ok(_) => {
                    let message = "upstream returned no valid prompts".to_owned();
                    failures.push(format!("{}: {message}", source.code));
                    let cached = previous_by_source.remove(source.code).unwrap_or_default();
                    sources.push(source.status(cached.len(), None, Some(message)));
                    items.extend(cached);
                }
                Err(error) => {
                    let message = truncate_error(&error);
                    failures.push(format!("{}: {message}", source.code));
                    let cached = previous_by_source.remove(source.code).unwrap_or_default();
                    let last_synced_at = previous
                        .sources
                        .iter()
                        .find(|status| status.code == source.code)
                        .and_then(|status| status.last_synced_at);
                    sources.push(source.status(cached.len(), last_synced_at, Some(message)));
                    items.extend(cached);
                }
            }
        }

        if successful_sources == 0 {
            if items.is_empty() {
                return Err(AppError::BadGateway(format!(
                    "prompt catalog synchronization failed: {}",
                    failures.join("; ")
                )));
            }
            return Ok(PromptCatalogSnapshot {
                schema: PROMPT_CATALOG_SCHEMA.to_owned(),
                version: PROMPT_CATALOG_VERSION,
                synced_at: previous.synced_at,
                items,
                sources,
            }
            .into_page(current_ms));
        }

        if items.len() > MAX_PROMPTS {
            return Err(AppError::BadGateway(format!(
                "prompt catalog contains {} items; maximum is {MAX_PROMPTS}",
                items.len()
            )));
        }
        let snapshot = PromptCatalogSnapshot {
            schema: PROMPT_CATALOG_SCHEMA.to_owned(),
            version: PROMPT_CATALOG_VERSION,
            synced_at: current_ms,
            items,
            sources,
        };
        validate_snapshot(&snapshot)?;
        self.save_snapshot(&snapshot).await?;
        Ok(snapshot.into_page(current_ms))
    }

    async fn load_snapshot(&self) -> Result<PromptCatalogSnapshot, AppError> {
        let path = self.cache_dir.join(PROMPT_CATALOG_FILE);
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PromptCatalogSnapshot::empty());
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "read prompt catalog cache {}: {error}",
                    path.display()
                )));
            }
        };
        if bytes.len() > MAX_PROMPT_CATALOG_BYTES {
            return Err(AppError::Internal(format!(
                "prompt catalog cache exceeds {MAX_PROMPT_CATALOG_BYTES} bytes"
            )));
        }
        let snapshot: PromptCatalogSnapshot = serde_json::from_slice(&bytes)
            .map_err(|error| AppError::Internal(format!("parse prompt catalog cache: {error}")))?;
        validate_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    async fn save_snapshot(&self, snapshot: &PromptCatalogSnapshot) -> Result<(), AppError> {
        let bytes = serde_json::to_vec(snapshot)
            .map_err(|error| AppError::Internal(format!("serialize prompt catalog: {error}")))?;
        if bytes.len() > MAX_PROMPT_CATALOG_BYTES {
            return Err(AppError::Internal(format!(
                "serialized prompt catalog exceeds {MAX_PROMPT_CATALOG_BYTES} bytes"
            )));
        }
        fsio::save_bytes_atomic(&self.cache_dir, PROMPT_CATALOG_FILE, &bytes)
            .await
            .map_err(|error| AppError::Internal(format!("save prompt catalog cache: {error}")))
    }
}

fn truncate_error(error: &str) -> String {
    error.chars().take(240).collect()
}

fn validate_snapshot(snapshot: &PromptCatalogSnapshot) -> Result<(), AppError> {
    if snapshot.schema != PROMPT_CATALOG_SCHEMA || snapshot.version != PROMPT_CATALOG_VERSION {
        return Err(AppError::Internal(
            "prompt catalog cache schema/version is unsupported".into(),
        ));
    }
    if snapshot.synced_at < 0 || snapshot.items.len() > MAX_PROMPTS {
        return Err(AppError::Internal(
            "prompt catalog cache has invalid bounds".into(),
        ));
    }
    if snapshot.sources.len() != PROMPT_SOURCES.len() {
        return Err(AppError::Internal(
            "prompt catalog cache has an invalid source set".into(),
        ));
    }
    let mut item_counts = HashMap::new();
    for item in &snapshot.items {
        *item_counts.entry(item.category.as_str()).or_insert(0usize) += 1;
    }
    for (status, expected) in snapshot.sources.iter().zip(PROMPT_SOURCES) {
        let valid_error = status.last_error.as_deref().is_none_or(|error| {
            !error.trim().is_empty()
                && error.chars().count() <= 240
                && !error.chars().any(char::is_control)
        });
        if status.code != expected.code
            || status.name != expected.name
            || status.description != expected.description
            || status.repository_url != expected.repository_url
            || status.license != expected.license
            || status.license_url != expected.license_url
            || status.item_count != item_counts.get(expected.code).copied().unwrap_or_default()
            || status.last_synced_at.is_some_and(|value| value <= 0)
            || !valid_error
        {
            return Err(AppError::Internal(format!(
                "prompt catalog source '{}' has invalid metadata",
                status.code
            )));
        }
    }

    let source_codes: HashSet<&str> = PROMPT_SOURCES.iter().map(|source| source.code).collect();
    let mut ids = HashSet::new();
    for item in &snapshot.items {
        let field = if item.id.trim().is_empty() || item.id.len() > 255 {
            Some("id")
        } else if !ids.insert(item.id.as_str()) {
            Some("duplicate id")
        } else if item.title.trim().is_empty() || item.title.len() > 1_000 {
            Some("title")
        } else if item.prompt.trim().is_empty() || item.prompt.len() > 1_000_000 {
            Some("prompt")
        } else if !source_codes.contains(item.category.as_str()) {
            Some("category")
        } else if item.tags.len() > 64
            || item
                .tags
                .iter()
                .any(|tag| tag.trim().is_empty() || tag.len() > 128)
        {
            Some("tags")
        } else if item.preview.len() > 100_000 {
            Some("preview")
        } else if !valid_https_url(&item.source_url) {
            Some("sourceUrl")
        } else if !valid_https_url(&item.license_url) {
            Some("licenseUrl")
        } else if item
            .cover_url
            .as_deref()
            .is_some_and(|url| !valid_https_url(url))
        {
            Some("coverUrl")
        } else {
            None
        };
        if let Some(field) = field {
            return Err(AppError::Internal(format!(
                "prompt catalog item '{}' has invalid {field}",
                item.id
            )));
        }
    }
    Ok(())
}

fn valid_https_url(value: &str) -> bool {
    if value.len() > 4_096 || value.contains(char::is_whitespace) {
        return false;
    }
    reqwest::Url::parse(value).is_ok_and(|url| url.scheme() == "https" && url.host().is_some())
}

async fn fetch_text(client: &Client, base_url: &str, file: &str) -> Result<String, String> {
    let url = format!("{}/{}", base_url.trim_end_matches('/'), file.trim_start_matches('/'));
    let mut response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("fetch {file}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("fetch {file}: HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROMPT_SOURCE_BYTES as u64)
    {
        return Err(format!(
            "fetch {file}: response exceeds {MAX_PROMPT_SOURCE_BYTES} bytes"
        ));
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_PROMPT_SOURCE_BYTES as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("read {file}: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_PROMPT_SOURCE_BYTES {
            return Err(format!(
                "fetch {file}: response exceeds {MAX_PROMPT_SOURCE_BYTES} bytes"
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|error| format!("decode {file}: {error}"))
}

fn source_definition(code: &str) -> &'static PromptSourceDefinition {
    PROMPT_SOURCES
        .iter()
        .find(|source| source.code == code)
        .expect("prompt parser source must be registered")
}

fn prompt_item(
    category: &str,
    id: String,
    title: String,
    cover_url: String,
    prompt: String,
    tags: Vec<String>,
    preview: String,
    created_at: String,
    updated_at: String,
) -> CreativePromptCatalogItem {
    let source = source_definition(category);
    CreativePromptCatalogItem {
        id,
        title: title.trim().chars().take(240).collect(),
        cover_url: safe_remote_url(&cover_url),
        prompt: prompt.trim().to_owned(),
        tags: dedupe_tags(tags),
        category: category.to_owned(),
        source_url: source.repository_url.to_owned(),
        license: source.license.to_owned(),
        license_url: source.license_url.to_owned(),
        preview: preview.trim().to_owned(),
        created_at: optional_text(created_at),
        updated_at: optional_text(updated_at),
    }
}

fn optional_text(value: String) -> Option<String> {
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn safe_remote_url(value: &str) -> Option<String> {
    let value = value.trim();
    valid_https_url(value).then(|| value.to_owned())
}

fn dedupe_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty() && tag.len() <= 128 && seen.insert(tag.clone()))
        .take(64)
        .collect()
}

#[derive(Deserialize)]
struct GptImage2Data {
    records: Vec<GptImage2Record>,
}

#[derive(Deserialize)]
struct GptImage2Record {
    #[serde(default)]
    title: String,
    #[serde(default)]
    tweet_url: String,
    #[serde(default)]
    image_dir: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    added_at: String,
}

#[derive(Clone, Default)]
struct GptImage2Case {
    prompt: String,
    image: String,
}

async fn build_gpt_image_2_prompts(
    client: &Client,
) -> Result<Vec<CreativePromptCatalogItem>, String> {
    let raw = fetch_text(client, GPT_IMAGE_2_RAW_BASE, "data/ingested_tweets.json").await?;
    let data: GptImage2Data =
        serde_json::from_str(&raw).map_err(|error| format!("parse ingested_tweets.json: {error}"))?;
    let (
        readme,
        ad_creative,
        character,
        comparison,
        ecommerce,
        portrait,
        poster,
        ui,
    ) = tokio::join!(
        fetch_text(client, GPT_IMAGE_2_RAW_BASE, GPT_IMAGE_2_CASE_FILES[0]),
        fetch_text(client, GPT_IMAGE_2_RAW_BASE, GPT_IMAGE_2_CASE_FILES[1]),
        fetch_text(client, GPT_IMAGE_2_RAW_BASE, GPT_IMAGE_2_CASE_FILES[2]),
        fetch_text(client, GPT_IMAGE_2_RAW_BASE, GPT_IMAGE_2_CASE_FILES[3]),
        fetch_text(client, GPT_IMAGE_2_RAW_BASE, GPT_IMAGE_2_CASE_FILES[4]),
        fetch_text(client, GPT_IMAGE_2_RAW_BASE, GPT_IMAGE_2_CASE_FILES[5]),
        fetch_text(client, GPT_IMAGE_2_RAW_BASE, GPT_IMAGE_2_CASE_FILES[6]),
        fetch_text(client, GPT_IMAGE_2_RAW_BASE, GPT_IMAGE_2_CASE_FILES[7]),
    );
    let markdown_files = [
        readme?,
        ad_creative?,
        character?,
        comparison?,
        ecommerce?,
        portrait?,
        poster?,
        ui?,
    ];
    let mut cases = HashMap::new();
    for markdown in markdown_files {
        collect_gpt_image_2_cases(&mut cases, &markdown);
    }
    let mut items = Vec::new();
    for record in data.records {
        let case = cases
            .get(&record.tweet_url)
            .or_else(|| cases.get(&record.image_dir));
        let Some(case) = case else { continue };
        if case.prompt.trim().is_empty() {
            continue;
        }
        let date = record.added_at.trim().to_owned();
        let title = first_non_empty(&[
            &record.title,
            &record.category,
            "GPT Image 2 Prompt",
        ]);
        items.push(prompt_item(
            "gpt-image-2-prompts",
            format!("gpt-image-2-prompts-{}", left_pad(items.len() + 1)),
            title,
            case.image.clone(),
            case.prompt.clone(),
            tags_from_category(&record.category),
            markdown_preview(std::slice::from_ref(&case.image)),
            date.clone(),
            date,
        ));
        if items.len() >= MAX_PROMPTS_PER_SOURCE {
            break;
        }
    }
    Ok(items)
}

fn collect_gpt_image_2_cases(cases: &mut HashMap<String, GptImage2Case>, markdown: &str) {
    let case_re = Regex::new(
        r"(?s)### Case \d+: \[[^\]]+\]\(([^)]+)\).*?\*\*Prompt:\*\*\s*\r?\n\s*```[\w-]*\r?\n(.*?)\r?\n```",
    )
    .expect("static case regex");
    let image_dir_re = Regex::new(r"images/\w+_case\d+").expect("static image-dir regex");
    for capture in case_re.captures_iter(markdown) {
        let whole = capture.get(0).map_or("", |value| value.as_str());
        let prompt = capture.get(2).map_or("", |value| value.as_str()).trim();
        if prompt.is_empty() {
            continue;
        }
        let image = extract_markdown_images(GPT_IMAGE_2_RAW_BASE, whole)
            .into_iter()
            .next()
            .unwrap_or_default();
        let item = GptImage2Case {
            prompt: prompt.to_owned(),
            image,
        };
        if let Some(key) = capture.get(1) {
            cases.insert(key.as_str().to_owned(), item.clone());
        }
        if let Some(key) = image_dir_re.find(whole) {
            cases.insert(key.as_str().to_owned(), item);
        }
    }
}

async fn build_awesome_gpt_image_prompts(
    client: &Client,
) -> Result<Vec<CreativePromptCatalogItem>, String> {
    let markdown = fetch_text(client, AWESOME_GPT_IMAGE_RAW_BASE, "README.md").await?;
    Ok(parse_awesome_gpt_image_prompts(&markdown))
}

fn parse_awesome_gpt_image_prompts(markdown: &str) -> Vec<CreativePromptCatalogItem> {
    let title_link_re = Regex::new(r"\[([^\]]+)\]\([^)]+\)").expect("static title regex");
    let mut items = Vec::new();
    for section in split_before_heading(markdown, "## ") {
        let tags = tags_from_heading(&first_match(&section, r"(?m)^##\s+(.+)$"));
        for block in split_before_heading(&section, "### ") {
            let raw_title = first_match(&block, r"(?m)^###\s+(.+)$");
            let title = title_link_re.replace_all(&raw_title, "$1").trim().to_owned();
            let prompt = first_match(
                &block,
                r"(?s)\*\*Prompt:\*\*\s*\r?\n\s*```[\w-]*\r?\n(.*?)\r?\n```",
            );
            if title.is_empty() || prompt.trim().is_empty() {
                continue;
            }
            let images = extract_markdown_images(AWESOME_GPT_IMAGE_RAW_BASE, &block);
            items.push(prompt_item(
                "awesome-gpt-image",
                format!("awesome-gpt-image-{}", left_pad(items.len() + 1)),
                title,
                images.first().cloned().unwrap_or_default(),
                prompt,
                tags.clone(),
                markdown_preview(&images),
                String::new(),
                String::new(),
            ));
            if items.len() >= MAX_PROMPTS_PER_SOURCE {
                return items;
            }
        }
    }
    items
}

async fn build_awesome_gpt4o_image_prompts(
    client: &Client,
) -> Result<Vec<CreativePromptCatalogItem>, String> {
    let markdown = fetch_text(
        client,
        AWESOME_GPT4O_IMAGE_PROMPTS_BASE,
        "README.zh-CN.md",
    )
    .await?;
    Ok(parse_awesome_gpt4o_image_prompts(&markdown))
}

fn parse_awesome_gpt4o_image_prompts(markdown: &str) -> Vec<CreativePromptCatalogItem> {
    let mut items = Vec::new();
    for block in split_before_heading(markdown, "### ") {
        let title = first_match(&block, r"(?m)^###\s+(.+)$").trim().to_owned();
        let prompt = first_match(&block, r"(?s)- \*\*提示词文本：\*\*\s*`(.*?)`");
        if title.is_empty() || prompt.trim().is_empty() {
            continue;
        }
        let images = extract_markdown_images(AWESOME_GPT4O_IMAGE_PROMPTS_BASE, &block);
        items.push(prompt_item(
            "awesome-gpt4o-image-prompts",
            format!(
                "awesome-gpt4o-image-prompts-{}",
                left_pad(items.len() + 1)
            ),
            title,
            images.first().cloned().unwrap_or_default(),
            prompt,
            vec!["gpt4o".into()],
            markdown_preview(&images),
            String::new(),
            String::new(),
        ));
        if items.len() >= MAX_PROMPTS_PER_SOURCE {
            break;
        }
    }
    items
}

#[derive(Default, Deserialize)]
struct XianyuLatestPromptData {
    #[serde(default)]
    dates: Vec<XianyuLatestPromptGroup>,
    #[serde(default)]
    items: Vec<XianyuLatestPrompt>,
}

#[derive(Default, Deserialize)]
struct XianyuLatestPromptGroup {
    #[serde(default)]
    items: Vec<XianyuLatestPrompt>,
}

#[derive(Clone, Default, Deserialize)]
struct XianyuLatestPrompt {
    #[serde(default)]
    x_url: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    image_urls: Vec<String>,
    #[serde(default)]
    primary_image_url: String,
}

async fn build_xianyu_awesome_gpt_image_2_prompts(
    client: &Client,
) -> Result<Vec<CreativePromptCatalogItem>, String> {
    let (markdown, latest) = tokio::join!(
        fetch_text(client, XIAN_YU_AWESOME_GPT_IMAGE_2_RAW_BASE, "README.md"),
        fetch_text(
            client,
            XIAN_YU_AWESOME_GPT_IMAGE_2_RAW_BASE,
            "data/latest-prompts.json"
        ),
    );
    let mut items = parse_xianyu_prompt_collection(&markdown?);
    items.extend(parse_xianyu_latest_prompts(&latest?, items.len())?);
    items.truncate(MAX_PROMPTS_PER_SOURCE);
    Ok(items)
}

fn parse_xianyu_prompt_collection(markdown: &str) -> Vec<CreativePromptCatalogItem> {
    let section = markdown_section(markdown, "## 提示词合集", "## 高级技巧");
    let mut items = Vec::new();
    let mut current_category = String::new();
    let mut current_title = String::new();
    let mut current_lines = Vec::new();
    let finish = |items: &mut Vec<CreativePromptCatalogItem>,
                  category: &str,
                  title: &str,
                  lines: &[String]| {
        if title.is_empty() || category == "补充案例提示词" {
            return;
        }
        let block = lines.join("\n");
        let prompt = {
            let code = xianyu_code_block_text(&block);
            if code.is_empty() {
                xianyu_fallback_prompt_text(&block)
            } else {
                code
            }
        };
        if prompt.is_empty() {
            return;
        }
        let images = extract_markdown_images(XIAN_YU_AWESOME_GPT_IMAGE_2_RAW_BASE, &block);
        items.push(prompt_item(
            "xianyu-awesome-gptimage2",
            format!("xianyu-awesome-gptimage2-{}", left_pad(items.len() + 1)),
            title.to_owned(),
            images.first().cloned().unwrap_or_default(),
            prompt,
            xianyu_prompt_tags(category),
            markdown_preview(&images),
            String::new(),
            String::new(),
        ));
    };
    for line in section.lines() {
        if line.starts_with("### ") && !line.starts_with("#### ") {
            finish(
                &mut items,
                &current_category,
                &current_title,
                &current_lines,
            );
            current_title.clear();
            current_lines.clear();
            current_category = clean_xianyu_category(line.trim_start_matches("### ").trim());
        } else if line.starts_with("#### ") {
            finish(
                &mut items,
                &current_category,
                &current_title,
                &current_lines,
            );
            current_title = clean_xianyu_prompt_title(line.trim_start_matches("#### ").trim());
            current_lines.clear();
        } else if !current_title.is_empty() {
            current_lines.push(line.to_owned());
        }
    }
    finish(
        &mut items,
        &current_category,
        &current_title,
        &current_lines,
    );
    items
}

fn parse_xianyu_latest_prompts(
    raw: &str,
    offset: usize,
) -> Result<Vec<CreativePromptCatalogItem>, String> {
    let data: XianyuLatestPromptData = serde_json::from_str(raw)
        .map_err(|error| format!("parse xianyu latest-prompts.json: {error}"))?;
    let mut values = Vec::new();
    for group in data.dates {
        values.extend(group.items);
    }
    values.extend(data.items);
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for value in values {
        let prompt = value.prompt.trim();
        if prompt.is_empty() {
            continue;
        }
        let key = first_non_empty(&[
            &value.x_url,
            &value.url,
            &format!("{}{}{}", value.author, value.created_at, prompt),
        ]);
        if !seen.insert(key) {
            continue;
        }
        let image = first_non_empty(&[
            &value.primary_image_url,
            value.image_urls.first().map(String::as_str).unwrap_or(""),
        ]);
        let title = first_non_empty(&[&value.reason, &value.author, "X Prompt"]);
        let preview = xianyu_latest_preview(&value, &image);
        items.push(prompt_item(
            "xianyu-awesome-gptimage2",
            format!(
                "xianyu-awesome-gptimage2-{}",
                left_pad(offset + items.len() + 1)
            ),
            title,
            image,
            prompt.to_owned(),
            vec!["x".into()],
            preview,
            value.created_at.clone(),
            value.created_at,
        ));
    }
    Ok(items)
}

async fn build_youmind_prompts(
    client: &Client,
    base_url: &str,
    id_prefix: &str,
    model_tag: &str,
) -> Result<Vec<CreativePromptCatalogItem>, String> {
    let markdown = fetch_text(client, base_url, "README_zh.md").await?;
    Ok(parse_youmind_prompts(
        &markdown, base_url, id_prefix, model_tag,
    ))
}

fn parse_youmind_prompts(
    markdown: &str,
    base_url: &str,
    id_prefix: &str,
    model_tag: &str,
) -> Vec<CreativePromptCatalogItem> {
    let mut items = Vec::new();
    for block in split_before_heading(markdown, "### ") {
        let title = first_match(&block, r"(?m)^###\s+No\.\s*\d+:\s*(.+)$")
            .trim()
            .to_owned();
        let prompt = first_match(
            &block,
            r"(?s)#### .*?提示词\s*\r?\n\s*```[\w-]*\r?\n(.*?)\r?\n```",
        );
        if title.is_empty() || prompt.trim().is_empty() {
            continue;
        }
        let images = extract_markdown_images(base_url, &block);
        items.push(prompt_item(
            id_prefix,
            format!("{}-{}", id_prefix, left_pad(items.len() + 1)),
            title.clone(),
            images.first().cloned().unwrap_or_default(),
            prompt,
            youmind_tags(&title, model_tag),
            markdown_preview(&images),
            String::new(),
            String::new(),
        ));
        if items.len() >= MAX_PROMPTS_PER_SOURCE {
            break;
        }
    }
    items
}

#[derive(Deserialize)]
struct DavidWuPrompt {
    id: usize,
    #[serde(default)]
    title_en: String,
    #[serde(default)]
    title_cn: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    category_cn: String,
    prompt: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    needs_ref: bool,
    #[serde(default)]
    image: String,
}

async fn build_david_wu_gpt_image_2_prompts(
    client: &Client,
) -> Result<Vec<CreativePromptCatalogItem>, String> {
    let raw = fetch_text(client, DAVID_WU_GPT_IMAGE_2_RAW_BASE, "prompts.json").await?;
    parse_david_wu_prompts(&raw)
}

fn parse_david_wu_prompts(raw: &str) -> Result<Vec<CreativePromptCatalogItem>, String> {
    let data: Vec<DavidWuPrompt> =
        serde_json::from_str(raw).map_err(|error| format!("parse prompts.json: {error}"))?;
    let mut items = Vec::new();
    for value in data {
        let title = first_non_empty(&[&value.title_cn, &value.title_en]);
        if title.is_empty() || value.prompt.trim().is_empty() {
            continue;
        }
        let image = absolute_image(DAVID_WU_GPT_IMAGE_2_RAW_BASE, &value.image);
        let tags = david_wu_tags(&value);
        let preview = david_wu_preview(&value, &image);
        items.push(prompt_item(
            "davidwu-gpt-image2-prompts",
            format!("davidwu-gpt-image2-prompts-{}", left_pad(value.id)),
            title,
            image,
            value.prompt,
            tags,
            preview,
            String::new(),
            String::new(),
        ));
        if items.len() >= MAX_PROMPTS_PER_SOURCE {
            break;
        }
    }
    Ok(items)
}

fn split_before_heading(markdown: &str, prefix: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in markdown.lines() {
        if line.starts_with(prefix) && !current.is_empty() {
            blocks.push(current.join("\n"));
            current.clear();
        }
        current.push(line);
    }
    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }
    blocks
}

fn first_match(value: &str, pattern: &str) -> String {
    Regex::new(pattern)
        .expect("static prompt parser regex")
        .captures(value)
        .and_then(|capture| capture.get(1))
        .map_or_else(String::new, |value| value.as_str().to_owned())
}

fn tags_from_category(category: &str) -> Vec<String> {
    let cleaned = Regex::new(r"(?i)\s+Cases$")
        .expect("static category regex")
        .replace(category, "");
    split_tags(&cleaned, r"\s*(&|and)\s*")
}

fn tags_from_heading(heading: &str) -> Vec<String> {
    let cleaned = Regex::new(r"[^\p{L}\p{N}/&、与 ]")
        .expect("static heading regex")
        .replace_all(heading, "");
    split_tags(&cleaned, r"\s*(/|&|、|与)\s*")
}

fn split_tags(value: &str, pattern: &str) -> Vec<String> {
    Regex::new(pattern)
        .expect("static tag regex")
        .split(value)
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn youmind_tags(title: &str, model_tag: &str) -> Vec<String> {
    let mut tags = vec![model_tag.to_owned()];
    if let Some((prefix, _)) = title.split_once(" - ") {
        tags.extend(tags_from_heading(prefix));
    }
    tags
}

fn david_wu_tags(item: &DavidWuPrompt) -> Vec<String> {
    let mut tags = split_tags(
        &[
            item.category_cn.as_str(),
            item.category.as_str(),
            item.author.as_str(),
            item.source.as_str(),
        ]
        .join("/"),
        "/",
    );
    if item.needs_ref {
        tags.push("需要参考图".into());
    }
    tags
}

fn david_wu_preview(item: &DavidWuPrompt, image: &str) -> String {
    let mut lines = Vec::new();
    if !item.title_en.trim().is_empty() {
        lines.push(item.title_en.trim().to_owned());
    }
    if !item.note.trim().is_empty() {
        lines.push(item.note.trim().to_owned());
    }
    if !image.is_empty() {
        lines.push(format!("![]({image})"));
    }
    lines.join("\n\n")
}

fn markdown_preview(images: &[String]) -> String {
    images
        .iter()
        .filter(|image| !image.is_empty())
        .map(|image| format!("![]({image})"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn extract_markdown_images(base_url: &str, block: &str) -> Vec<String> {
    let html_re = Regex::new(r#"<img[^>]+src="([^"]+)""#).expect("static HTML image regex");
    let markdown_re =
        Regex::new(r"!\[[^\]]*\]\(([^)]+)\)").expect("static Markdown image regex");
    let mut seen = HashSet::new();
    let mut images = Vec::new();
    for capture in html_re.captures_iter(block).chain(markdown_re.captures_iter(block)) {
        let Some(value) = capture.get(1) else { continue };
        let image = absolute_image(base_url, value.as_str());
        if safe_remote_url(&image).is_some() && seen.insert(image.clone()) {
            images.push(image);
        }
    }
    images
}

fn absolute_image(base_url: &str, image: &str) -> String {
    let image = image.trim();
    if image.is_empty() || image.starts_with("https://") || image.starts_with("http://") {
        return image.to_owned();
    }
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        image.trim_start_matches('.').trim_start_matches('/')
    )
}

fn left_pad(value: usize) -> String {
    if value >= 1_000 {
        value.to_string()
    } else {
        format!("{value:03}")
    }
}

fn markdown_section(markdown: &str, start_heading: &str, end_heading: &str) -> String {
    let Some(start) = markdown.find(start_heading) else {
        return String::new();
    };
    let rest_start = start + start_heading.len();
    let rest = &markdown[rest_start..];
    let end = rest.find(end_heading).unwrap_or(rest.len());
    markdown[start..rest_start + end].to_owned()
}

fn clean_xianyu_category(value: &str) -> String {
    let mut value = value.trim().to_owned();
    for separator in ["、", ".", "．", " "] {
        if let Some(index) = value.find(separator) {
            let prefix = value[..index].trim();
            if !prefix.is_empty() && prefix.chars().count() <= 4 {
                value = value[index + separator.len()..].trim().to_owned();
            }
            break;
        }
    }
    value
}

fn clean_xianyu_prompt_title(value: &str) -> String {
    let value = value.trim();
    if let Some(index) = value.find(' ') {
        let prefix = &value[..index];
        if prefix.contains('.') || prefix.contains('．') {
            return value[index + 1..].trim().to_owned();
        }
    }
    value.to_owned()
}

fn xianyu_code_block_text(block: &str) -> String {
    let mut lines = Vec::new();
    let mut in_code = false;
    for line in block.lines() {
        let text = line.trim();
        if text.starts_with("```") {
            if in_code {
                break;
            }
            in_code = true;
            continue;
        }
        if in_code {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_owned()
}

fn xianyu_fallback_prompt_text(block: &str) -> String {
    let mut lines = Vec::new();
    for line in block.lines() {
        let mut text = line.trim();
        if text.is_empty()
            || text.starts_with('#')
            || text.starts_with("---")
            || text.starts_with("![")
            || text.starts_with('|')
            || text.starts_with('>')
            || text.starts_with("```")
            || ["- 原文链接", "- 公众号", "- 作者", "- 本次补充", "- 说明"]
                .iter()
                .any(|prefix| text.starts_with(prefix))
        {
            continue;
        }
        text = text.trim_start_matches(['-', '*']).trim();
        text = text.strip_prefix("提示词：").unwrap_or(text).trim();
        if !text.is_empty() && !text.starts_with("http") {
            lines.push(text);
        }
    }
    lines.join("\n")
}

fn xianyu_prompt_tags(category: &str) -> Vec<String> {
    let mut tags = vec!["gpt-image-2".into()];
    if !category.trim().is_empty() {
        tags.extend(split_tags(category, r"\s*(/|&|、|与)\s*"));
    }
    tags
}

fn xianyu_latest_preview(item: &XianyuLatestPrompt, image: &str) -> String {
    let mut lines = Vec::new();
    let link = first_non_empty(&[&item.x_url, &item.url]);
    if !link.is_empty() {
        lines.push(link);
    }
    for url in &item.image_urls {
        if !url.trim().is_empty() {
            lines.push(url.trim().to_owned());
        }
    }
    if lines.len() == 1 && !image.is_empty() {
        lines.push(image.to_owned());
    }
    if !item.text.trim().is_empty() {
        lines.push(item.text.trim().to_owned());
    }
    lines.join("\n")
}

fn first_non_empty(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_parsers_preserve_prompt_media_and_attribution() {
        let awesome = parse_awesome_gpt_image_prompts(
            "## Poster & UI\n### [Paper](https://example.test)\n**Prompt:**\n```text\nDraw a paper poster\n```\n![](images/paper.png)",
        );
        assert_eq!(awesome.len(), 1);
        assert_eq!(awesome[0].prompt, "Draw a paper poster");
        assert_eq!(awesome[0].tags, vec!["poster", "ui"]);
        assert_eq!(
            awesome[0].cover_url.as_deref(),
            Some("https://raw.githubusercontent.com/ZeroLu/awesome-gpt-image/main/images/paper.png")
        );
        assert_eq!(awesome[0].license, "MIT");

        let gpt4o = parse_awesome_gpt4o_image_prompts(
            "### 纸雕\n- **提示词文本：** `Create a paper sculpture`\n<img src=\"assets/paper.jpg\">",
        );
        assert_eq!(gpt4o.len(), 1);
        assert_eq!(gpt4o[0].title, "纸雕");
        assert_eq!(gpt4o[0].tags, vec!["gpt4o"]);
    }

    #[test]
    fn david_wu_json_is_mapped_without_losing_source_metadata() {
        let items = parse_david_wu_prompts(
            r#"[{"id":7,"title_en":"Poster","title_cn":"海报","category":"Poster","category_cn":"海报设计","prompt":"Make a poster","note":"Use a reference","author":"Ada","source":"X","needs_ref":true,"image":"images/7.jpg"}]"#,
        )
        .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "davidwu-gpt-image2-prompts-007");
        assert!(items[0].tags.contains(&"需要参考图".to_owned()));
        assert!(items[0].preview.contains("Use a reference"));
        assert!(items[0].source_url.contains("davidwuw0811-boop"));
    }

    #[test]
    fn xianyu_parser_uses_code_block_and_category() {
        let items = parse_xianyu_prompt_collection(
            "## 提示词合集\n### 一、电商与产品\n#### 1. 商品海报\n```text\nCreate a product poster\n```\n![](images/product.png)\n## 高级技巧",
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "商品海报");
        assert_eq!(items[0].prompt, "Create a product poster");
        assert!(items[0].tags.contains(&"电商".to_owned()));
    }

    #[test]
    fn snapshot_validation_rejects_duplicates_and_non_https_media() {
        let mut item = prompt_item(
            "awesome-gpt-image",
            "same".into(),
            "Title".into(),
            String::new(),
            "Prompt".into(),
            vec![],
            String::new(),
            String::new(),
            String::new(),
        );
        let snapshot = PromptCatalogSnapshot {
            schema: PROMPT_CATALOG_SCHEMA.into(),
            version: PROMPT_CATALOG_VERSION,
            synced_at: 1,
            items: vec![item.clone(), item.clone()],
            sources: PROMPT_SOURCES
                .iter()
                .map(|source| {
                    source.status(
                        if source.code == "awesome-gpt-image" {
                            2
                        } else {
                            0
                        },
                        Some(1),
                        None,
                    )
                })
                .collect(),
        };
        assert!(validate_snapshot(&snapshot).is_err());

        item.cover_url = Some("http://example.test/image.jpg".into());
        let snapshot = PromptCatalogSnapshot {
            schema: PROMPT_CATALOG_SCHEMA.into(),
            version: PROMPT_CATALOG_VERSION,
            synced_at: 1,
            items: vec![item],
            sources: PROMPT_SOURCES
                .iter()
                .map(|source| {
                    source.status(
                        usize::from(source.code == "awesome-gpt-image"),
                        Some(1),
                        None,
                    )
                })
                .collect(),
        };
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[test]
    fn snapshot_validation_rejects_source_metadata_or_count_drift() {
        let item = prompt_item(
            "awesome-gpt-image",
            "prompt-1".into(),
            "Title".into(),
            String::new(),
            "Prompt".into(),
            vec![],
            String::new(),
            String::new(),
            String::new(),
        );
        let mut snapshot = PromptCatalogSnapshot {
            schema: PROMPT_CATALOG_SCHEMA.into(),
            version: PROMPT_CATALOG_VERSION,
            synced_at: 1,
            items: vec![item],
            sources: PROMPT_SOURCES
                .iter()
                .map(|source| {
                    source.status(
                        usize::from(source.code == "awesome-gpt-image"),
                        Some(1),
                        None,
                    )
                })
                .collect(),
        };
        validate_snapshot(&snapshot).unwrap();

        snapshot.sources[1].item_count = 0;
        assert!(validate_snapshot(&snapshot).is_err());
        snapshot.sources[1].item_count = 1;
        snapshot.sources[1].repository_url = "https://example.test/impostor".into();
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[tokio::test]
    async fn cache_roundtrip_is_versioned_and_offline() {
        let temp = tempfile::tempdir().unwrap();
        let service = PromptCatalogService::start(temp.path());
        let item = prompt_item(
            "awesome-gpt-image",
            "prompt-1".into(),
            "Title".into(),
            String::new(),
            "Prompt".into(),
            vec!["poster".into()],
            String::new(),
            String::new(),
            String::new(),
        );
        let snapshot = PromptCatalogSnapshot {
            schema: PROMPT_CATALOG_SCHEMA.into(),
            version: PROMPT_CATALOG_VERSION,
            synced_at: now_ms(),
            items: vec![item],
            sources: PROMPT_SOURCES
                .iter()
                .map(|source| {
                    source.status(
                        usize::from(source.code == "awesome-gpt-image"),
                        Some(now_ms()),
                        None,
                    )
                })
                .collect(),
        };
        service.save_snapshot(&snapshot).await.unwrap();
        let page = service.list().await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.categories, vec!["awesome-gpt-image"]);
        assert_eq!(page.tags, vec!["poster"]);
        assert!(!page.stale);
    }

}
