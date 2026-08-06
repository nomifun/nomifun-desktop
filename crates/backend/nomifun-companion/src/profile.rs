//! Multi-companion configuration split: a per-companion profile (`companion/companions/{companion_id}/config.json`)
//! holding identity/persona/model/window settings **plus that companion's own
//! learning and skill-evolution settings**, and a shared config
//! (`companion/shared/config.json`) holding the machine-level collection
//! switches, the session archiver and the default-companion pointer. Both reuse
//! the shared config value types from [`crate::config`] and the same atomic
//! temp+rename write pattern.
//!
//! 学习 / 进化 used to live on the shared config, so one schedule, one model and
//! one cursor served every companion. They are per companion since 2026-08; the
//! retired install-wide blocks are still read off disk exactly once by
//! [`SharedCompanionConfig::load_migrating`] so the boot migration can seed them
//! onto the companions that have none of their own.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use nomifun_common::{CompanionId, FigureId, ProviderWithModel, now_ms};
use serde::{Deserialize, Serialize};

use crate::config::{
    CollectConfig, DEFAULT_CHARACTER, PersonaConfig, deserialize_optional_model,
    serialize_optional_model,
};

/// Desktop-companion window settings for one companion. `character` lives
/// directly on [`CompanionProfileConfig`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompanionWindowConfig {
    /// Whether this companion's desktop window should be visible.
    pub companion_enabled: bool,
    /// Saved companion window position (physical px), if the user dragged it.
    pub companion_x: Option<i32>,
    pub companion_y: Option<i32>,
    /// Quiet hours "HH:mm" — within this window the companion only accrues badges
    /// and never pops bubbles, and its 学习 / 进化 ticks are skipped
    /// ([`CompanionWindowConfig::in_quiet_hours_now`]). Empty strings disable
    /// quiet hours.
    pub quiet_start: String,
    pub quiet_end: String,
    /// DIY single-image figure metadata (character == "custom"). Absent for
    /// roster characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_figure: Option<CustomFigureMeta>,
}

/// Minutes of the local day parsed from an `"HH:mm"` quiet-hours bound.
/// Accepts exactly the renderer's `^(\d{1,2}):(\d{2})$`; anything else (empty,
/// `"9:5"`, `"24:00"`, junk) is `None` and therefore disables the window.
fn parse_hhmm(value: &str) -> Option<u32> {
    let (hours, minutes) = value.trim().split_once(':')?;
    if !matches!(hours.len(), 1 | 2) || minutes.len() != 2 {
        return None;
    }
    if !hours.bytes().all(|b| b.is_ascii_digit()) || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: u32 = hours.parse().ok()?;
    let minutes: u32 = minutes.parse().ok()?;
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

impl CompanionWindowConfig {
    /// True when `minute_of_day` (0..1440, local time) falls inside the
    /// configured 休眠时段.
    ///
    /// This is the single definition of 休眠 for the backend loops. It mirrors the
    /// renderer's `inQuietHours` (`pages/companion/index.tsx`) bound for bound:
    /// an empty or unparseable bound disables the window; `start <= end` is a
    /// same-day `[start, end)` window; anything else wraps midnight
    /// (`22:00`–`08:00`); `start == end` is an empty window, i.e. disabled.
    /// The half-open upper bound keeps a `08:00` end from muting 08:00 itself.
    pub fn in_quiet_hours_at(&self, minute_of_day: u32) -> bool {
        let (Some(start), Some(end)) =
            (parse_hhmm(&self.quiet_start), parse_hhmm(&self.quiet_end))
        else {
            return false;
        };
        if start <= end {
            minute_of_day >= start && minute_of_day < end
        } else {
            minute_of_day >= start || minute_of_day < end
        }
    }

    /// [`Self::in_quiet_hours_at`] against the machine's local wall clock — the
    /// same clock the collector partitions its `events/YYYYMMDD.jsonl` day files
    /// by, and the same one the renderer reads.
    pub fn in_quiet_hours_now(&self) -> bool {
        use chrono::Timelike;
        let now = chrono::Local::now();
        self.in_quiet_hours_at(now.hour() * 60 + now.minute())
    }
}

/// Head-and-shoulders crop over the figure image in image-fraction coordinates:/// left `x` and width `w` are fractions of image WIDTH; top `y` and height `h`
/// are fractions of image HEIGHT. `h == 0` means a square box; the frontend
/// resolves it to `w * aspect`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeadBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    /// Box height as a fraction of image height. `0` means a square crop,
    /// resolved frontend-side to `w * aspect`.
    pub h: f32,
}

/// Metadata for a user-supplied single-image figure (`character == "custom"`),
/// mirrored by `CustomFigureMeta` in the UI (`characters/types.ts`). The image
/// bytes themselves live next to the profile as
/// `{companions_dir}/{companion_id}/{FIGURE_FILE}` (see [`crate::figure`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CustomFigureMeta {
    /// width / height of the cutout image.
    pub aspect: f32,
    pub head_box: HeadBox,
    /// Desk size tier: "s" | "m" | "l".
    pub size_tier: String,
    /// Per-companion continuous figure-height override (logical px). When set it
    /// supersedes `size_tier` for THIS companion's desktop window (the 总览 size
    /// slider writes it); absent ⇒ fall back to the tier's height. The frontend
    /// clamps it to its [SIZE_MIN, SIZE_MAX] range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_px: Option<f32>,
    /// Library figure this companion draws from (a bare UUIDv7). When set, the image is
    /// served from the shared figure library (`/api/companion/figures/{figure_id}/image`),
    /// so one figure can back many companions. When absent, the companion-owned
    /// figure endpoint is used.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_figure_id"
    )]
    pub figure_id: Option<String>,
}

/// General-purpose skills explicitly configured for one companion.
///
/// This intent is separate from the companion's self-evolved skills. `enabled`
/// contains opt-in catalog skills, while `disabled_auto` records auto-injected
/// built-ins that this companion has explicitly opted out of.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CompanionSkillConfig {
    pub enabled: Vec<String>,
    pub disabled_auto: Vec<String>,
}

/// Merge global auto-injected skills with one companion's explicit intent.
/// Values are normalized at the trusted profile boundary because profiles can
/// be patched by API callers other than the desktop UI.
pub(crate) fn normalized_effective_skill_names(
    auto_names: impl IntoIterator<Item = String>,
    config: &CompanionSkillConfig,
) -> Vec<String> {
    let disabled: HashSet<String> = config
        .disabled_auto
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    auto_names
        .into_iter()
        .chain(config.enabled.iter().map(|name| name.trim().to_owned()))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty() && !disabled.contains(name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Per-companion periodic-learning settings (定时学习).
///
/// Install-wide until 2026-08 (`SharedCompanionConfig::learn`), so one schedule
/// and one model distilled events for the whole roster. Now every companion runs
/// its own schedule, from its own cursor, writing memories it owns.
/// `#[serde(default)]` is what lets a pre-migration profile — and an RFC 7396
/// merge patch that names a single field — deserialize.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CompanionLearnConfig {
    pub enabled: bool,
    /// Minutes between learning runs, clamped to
    /// [`MIN_LEARN_INTERVAL_MINUTES`]..=[`MAX_LEARN_INTERVAL_MINUTES`].
    pub interval_minutes: u32,
    #[serde(
        deserialize_with = "deserialize_optional_model",
        serialize_with = "serialize_optional_model"
    )]
    pub model: Option<ProviderWithModel>,
}

/// Lower bound for [`CompanionLearnConfig::interval_minutes`] — a tighter loop
/// would burn tokens faster than events accumulate.
pub const MIN_LEARN_INTERVAL_MINUTES: u32 = 5;
/// Upper bound (24 h) for [`CompanionLearnConfig::interval_minutes`].
pub const MAX_LEARN_INTERVAL_MINUTES: u32 = 1440;

impl Default for CompanionLearnConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 60,
            model: None,
        }
    }
}

impl CompanionLearnConfig {
    /// The interval the loop actually uses. Clamped rather than validated at the
    /// read site so a profile written by an older build can never wedge the
    /// scheduler; durable writes are range-checked by
    /// [`CompanionProfileConfig::save`].
    pub fn effective_interval_minutes(&self) -> u32 {
        self.interval_minutes
            .clamp(MIN_LEARN_INTERVAL_MINUTES, MAX_LEARN_INTERVAL_MINUTES)
    }

    fn validate(&self) -> Result<(), String> {
        if !(MIN_LEARN_INTERVAL_MINUTES..=MAX_LEARN_INTERVAL_MINUTES)
            .contains(&self.interval_minutes)
        {
            return Err(format!(
                "learn.interval_minutes must be between {MIN_LEARN_INTERVAL_MINUTES} and {MAX_LEARN_INTERVAL_MINUTES}"
            ));
        }
        Ok(())
    }
}

/// Per-companion skill-evolution settings (design §6): the background
/// EvolutionEngine mines repeated multi-step tool sequences from real work and
/// drafts them into reviewable skills for THIS companion. Independent
/// schedule/model from the lightweight learner.
///
/// Only `enabled`, the 保守/激进 preference (`auto_activate`) and
/// `min_distinct_sessions` are surfaced in the UI. `min_pattern_count`,
/// `auto_threshold`, `skill_half_life_days` and `skill_archive_threshold` stay
/// Rust-configurable tuning knobs at their historical defaults — exposing four
/// more numeric dials buys nothing a user can reason about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CompanionEvolveConfig {
    pub enabled: bool,
    /// Minutes between evolution runs.
    pub interval_minutes: u32,
    #[serde(
        deserialize_with = "deserialize_optional_model",
        serialize_with = "serialize_optional_model"
    )]
    pub model: Option<ProviderWithModel>,
    /// A pattern must occur at least this many times total to be drafted.
    pub min_pattern_count: i64,
    /// A pattern must appear across at least this many distinct sessions.
    pub min_distinct_sessions: usize,
    /// 激进 (`true`): auto-activate a drafted skill — skip human review — when
    /// confidence ≥ `auto_threshold`. 保守 (`false`, the default): every draft
    /// waits for the owner. This bool IS the UI's two-mode preference.
    pub auto_activate: bool,
    /// Confidence cutoff for `auto_activate`.
    pub auto_threshold: f64,
    /// Skill strength half-life in days (decay clock = time since last use). Used skills reinforce.
    pub skill_half_life_days: f64,
    /// Below this strength a mined skill is auto-archived (restorable; manual skills never decay).
    pub skill_archive_threshold: f64,
}

impl Default for CompanionEvolveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 30,
            model: None,
            min_pattern_count: 3,
            min_distinct_sessions: 2,
            auto_activate: false,
            auto_threshold: 0.85,
            skill_half_life_days: 45.0,
            skill_archive_threshold: 0.05,
        }
    }
}

impl CompanionEvolveConfig {
    /// See [`CompanionLearnConfig::effective_interval_minutes`].
    pub fn effective_interval_minutes(&self) -> u32 {
        self.interval_minutes
            .clamp(MIN_LEARN_INTERVAL_MINUTES, MAX_LEARN_INTERVAL_MINUTES)
    }
}

/// The built-in local VAD engine name. Plan A's robot gateway recognises ONLY
/// this value; anything else falls back to its energy VAD. Kept as a string
/// rather than an enum so a future engine needs no profile migration.
pub const DEFAULT_VAD_ENGINE: &str = "silero";
/// Default speech-probability threshold for the VAD (0 = trigger on anything,
/// 1 = never trigger).
pub const DEFAULT_VAD_SENSITIVITY: f32 = 0.5;
/// Default trailing silence that ends one utterance.
pub const DEFAULT_VAD_MIN_SILENCE_MS: u32 = 700;
pub const MIN_VAD_MIN_SILENCE_MS: u32 = 200;
pub const MAX_VAD_MIN_SILENCE_MS: u32 = 3000;

/// Per-companion voice-activity detection settings (语音活动检测).
///
/// The engine runs locally (no Provider, no credential), so this block holds
/// tuning only. Values are clamped at the READ site
/// ([`Self::effective_sensitivity`] / [`Self::effective_min_silence_ms`]) and
/// range-checked on the way out ([`CompanionProfileConfig::save`]) — the same
/// split [`CompanionLearnConfig`] uses, so a profile written by another build
/// can never wedge the voice pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CompanionVadConfig {
    /// Detection engine id; `"silero"` is the only one implemented.
    pub engine: String,
    /// Speech probability threshold, 0.0..=1.0.
    pub sensitivity: f32,
    /// Trailing silence (ms) that closes one utterance, 200..=3000.
    pub min_silence_ms: u32,
}

impl Default for CompanionVadConfig {
    fn default() -> Self {
        Self {
            engine: DEFAULT_VAD_ENGINE.to_owned(),
            sensitivity: DEFAULT_VAD_SENSITIVITY,
            min_silence_ms: DEFAULT_VAD_MIN_SILENCE_MS,
        }
    }
}

impl CompanionVadConfig {
    /// The threshold the pipeline actually uses. A non-finite or out-of-range
    /// durable value resolves to the default rather than disabling detection.
    pub fn effective_sensitivity(&self) -> f32 {
        if self.sensitivity.is_finite() {
            self.sensitivity.clamp(0.0, 1.0)
        } else {
            DEFAULT_VAD_SENSITIVITY
        }
    }

    /// The pause the pipeline actually uses, clamped to the documented range.
    pub fn effective_min_silence_ms(&self) -> u32 {
        self.min_silence_ms
            .clamp(MIN_VAD_MIN_SILENCE_MS, MAX_VAD_MIN_SILENCE_MS)
    }

    fn validate(&self) -> Result<(), String> {
        if self.engine.trim().is_empty() {
            return Err("voice.vad.engine must not be empty".into());
        }
        if !self.sensitivity.is_finite() || !(0.0..=1.0).contains(&self.sensitivity) {
            return Err("voice.vad.sensitivity must be between 0.0 and 1.0".into());
        }
        if !(MIN_VAD_MIN_SILENCE_MS..=MAX_VAD_MIN_SILENCE_MS).contains(&self.min_silence_ms) {
            return Err(format!(
                "voice.vad.min_silence_ms must be between {MIN_VAD_MIN_SILENCE_MS} and {MAX_VAD_MIN_SILENCE_MS}"
            ));
        }
        Ok(())
    }
}

/// One companion's speech-synthesis 选择: which catalog model speaks, and in
/// which provider voice.
///
/// Deliberately NOT a [`ProviderWithModel`]: the voice id is part of the
/// selection, and the side store's model shape is fixed at
/// `{provider_id, model}`. [`Self::as_provider_model`] projects it back for the
/// Provider-reference validators.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompanionTtsSelection {
    #[serde(deserialize_with = "crate::config::deserialize_provider_id")]
    pub provider_id: String,
    pub model: String,
    /// Provider voice id (free text). `None` = the provider's own default voice.
    #[serde(default)]
    pub voice: Option<String>,
}

impl CompanionTtsSelection {
    /// The `(provider, model)` pair this selection points at — the shape the
    /// profile's Provider-reference validators and the deletion-usage scan read.
    pub fn as_provider_model(&self) -> ProviderWithModel {
        ProviderWithModel {
            provider_id: self.provider_id.clone(),
            model: self.model.clone(),
            use_model: None,
        }
    }
}

/// One companion's voice stack: 语音识别 (ASR), 语音合成 (TTS) and 语音活动检测
/// (VAD). ASR/TTS absent = fall back to the install-wide preferences
/// (`tools.speechToText` / `tools.textToSpeech`); VAD always has values because
/// the engine is local and needs no configuration to run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CompanionVoiceConfig {
    #[serde(
        deserialize_with = "deserialize_optional_model",
        serialize_with = "serialize_optional_model"
    )]
    pub asr: Option<ProviderWithModel>,
    pub tts: Option<CompanionTtsSelection>,
    pub vad: CompanionVadConfig,
}

/// Per-companion profile persisted as `companion/companions/{companion_id}/config.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompanionProfileConfig {
    /// Stable canonical bare UUIDv7 companion ID. [`Self::load`] returns `None` only
    /// when the profile file is absent; corrupt or non-canonical data is an
    /// error.
    #[serde(deserialize_with = "deserialize_companion_profile_id")]
    pub companion_id: String,
    /// Display-only short number (`#1`, `#2`, …) for companion lists. Monotonic
    /// within this machine, allocated by the registry from its private
    /// high-watermark state file (`companion/shared/companion_seq.json`) so a
    /// deleted companion's number is never reused.
    pub seq: u64,
    /// Display name chosen by the user.
    pub name: String,
    /// Which character renders in the companion window (mochi/ink/roux/pixel/bolt/boo).
    pub character: String,
    pub persona: PersonaConfig,
    /// Per-companion companion-chat model (定时学习 / 进化 carry their own).
    #[serde(
        deserialize_with = "deserialize_optional_model",
        serialize_with = "serialize_optional_model"
    )]
    pub model: Option<ProviderWithModel>,
    /// 备用对话模型: replayed once when the main model's turn fails, and used by
    /// the model-resolution chain when the main reference goes stale. Absent =
    /// no fallback, a failure is a failure.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_model",
        serialize_with = "serialize_optional_model"
    )]
    pub fallback_model: Option<ProviderWithModel>,
    /// 视觉大模型 for one-shot image understanding. Absent = use the main chat
    /// model when it carries the `vision_input` trait.
    #[serde(
        default,
        deserialize_with = "deserialize_optional_model",
        serialize_with = "serialize_optional_model"
    )]
    pub vision_model: Option<ProviderWithModel>,
    /// This companion's voice stack (ASR / TTS / VAD). `#[serde(default)]` so a
    /// profile written before the slots existed still loads.
    #[serde(default)]
    pub voice: CompanionVoiceConfig,
    /// This companion's own 定时学习 loop. `#[serde(default)]` so a profile
    /// written before the settings moved off the shared config still loads; the
    /// boot migration then seeds it from the retired install-wide values.
    #[serde(default)]
    pub learn: CompanionLearnConfig,
    /// This companion's own 技能进化 loop. Same defaulting rationale as `learn`.
    #[serde(default)]
    pub evolve: CompanionEvolveConfig,
    /// General-purpose skills assigned from the global skill catalog.
    #[serde(default)]
    pub skills: CompanionSkillConfig,
    pub appearance: CompanionWindowConfig,
    /// Frozen reusable configuration applied to this companion. Identity,
    /// memories, evolved skills, window state and channel credentials remain
    /// companion-owned; this snapshot only supplies execution preferences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_preset: Option<nomifun_api_types::ResolvedPresetSnapshot>,
    /// User-chosen sidebar position. `None` = never reordered; such companions
    /// sort after every explicitly ordered one, by `created_at`. Distinct from
    /// [`Self::seq`], which is a registry-owned never-reused display ordinal and
    /// therefore cannot express a user's ordering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_index: Option<i64>,
    pub created_at: i64,
}

impl CompanionProfileConfig {
    /// Fresh profile with a generated companion ID. An empty `character` falls back to
    /// the default roster character.
    pub fn new(name: &str, character: &str, seq: u64) -> Self {
        assert!(seq > 0, "companion display sequence must be positive");
        let character = if character.is_empty() { DEFAULT_CHARACTER } else { character };
        Self {
            companion_id: CompanionId::new().into_string(),
            seq,
            name: name.to_owned(),
            character: character.to_owned(),
            persona: PersonaConfig::default(),
            model: None,
            fallback_model: None,
            vision_model: None,
            voice: CompanionVoiceConfig::default(),
            learn: CompanionLearnConfig::default(),
            evolve: CompanionEvolveConfig::default(),
            skills: CompanionSkillConfig::default(),
            appearance: CompanionWindowConfig::default(),
            applied_preset: None,
            order_index: None,
            created_at: now_ms(),
        }
    }

    /// Every Provider reference this profile holds, labelled for diagnostics.
    ///
    /// ONE list so [`Self::load`], [`Self::save`], the startup audit
    /// (`CompanionRegistry::validate_provider_references_under_guard`) and the
    /// Provider-deletion usage scan (`CompanionService::providers_in_use`) can
    /// never disagree about which slots are hard references. Adding a slot means
    /// editing exactly this function.
    pub fn provider_model_slots(&self) -> Vec<(&'static str, ProviderWithModel)> {
        let mut slots = Vec::new();
        for (label, model) in [
            ("chat", self.model.as_ref()),
            ("learn", self.learn.model.as_ref()),
            ("evolve", self.evolve.model.as_ref()),
            ("fallback", self.fallback_model.as_ref()),
            ("vision", self.vision_model.as_ref()),
            ("asr", self.voice.asr.as_ref()),
        ] {
            if let Some(model) = model {
                slots.push((label, model.clone()));
            }
        }
        if let Some(tts) = self.voice.tts.as_ref() {
            slots.push(("tts", tts.as_provider_model()));
        }
        slots
    }

    pub fn config_path(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    /// Load and validate `{dir}/config.json`. Only a missing file is absent;
    /// malformed or non-canonical durable data fails closed.
    pub fn load(dir: &Path) -> Result<Option<Self>, nomifun_common::AppError> {
        let path = Self::config_path(dir);
        let Some(profile): Option<Self> = crate::fsio::load_json_optional(&path)
            .map_err(|error| {
                nomifun_common::AppError::Internal(format!(
                    "load companion profile {}: {error}",
                    path.display()
                ))
            })?
        else {
            return Ok(None);
        };
        CompanionId::try_from(profile.companion_id.as_str()).map_err(|error| {
            nomifun_common::AppError::Internal(format!(
                "companion profile {} has invalid companion_id: {error}",
                path.display()
            ))
        })?;
        if profile.seq == 0 {
            return Err(nomifun_common::AppError::Internal(format!(
                "companion profile {} has invalid zero sequence",
                path.display()
            )));
        }
        for (label, model) in profile.provider_model_slots() {
            validate_persisted_model(Some(&model)).map_err(|error| {
                nomifun_common::AppError::Internal(format!(
                    "companion profile {} has invalid {label} model: {error}",
                    path.display()
                ))
            })?;
        }
        validate_persisted_appearance(&profile.appearance).map_err(|error| {
            nomifun_common::AppError::Internal(format!(
                "companion profile {} has invalid custom figure: {error}",
                path.display()
            ))
        })?;
        if profile
            .applied_preset
            .as_ref()
            .is_some_and(|snapshot| snapshot.resolved_model.is_some())
        {
            return Err(nomifun_common::AppError::Internal(format!(
                "companion profile {} duplicates a Provider reference inside applied_preset",
                path.display()
            )));
        }
        Ok(Some(profile))
    }

    /// Whether `{dir}/config.json` already carries a `learn` or an `evolve`
    /// block of its own.
    ///
    /// Read off the RAW json, not the parsed struct: both fields are
    /// `#[serde(default)]`, so a profile written before 学习/进化 became
    /// per-companion and a profile whose owner deliberately picked the defaults
    /// deserialize to exactly the same value — and only the first may be
    /// overwritten by the boot seed. A missing or unreadable file answers
    /// `false`: seeding a profile that cannot be read fails later anyway, loudly.
    pub fn has_persisted_learn_or_evolve(dir: &Path) -> bool {
        let Ok(Some(value)) =
            crate::fsio::load_json_optional::<serde_json::Value>(&Self::config_path(dir))
        else {
            return false;
        };
        value.get("learn").is_some() || value.get("evolve").is_some()
    }

    /// Atomically persist to `{dir}/config.json` (unique temp file + rename,
    /// so two concurrent saves can never rename each other's half-written
    /// temp into place).
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        for (label, model) in self.provider_model_slots() {
            validate_persisted_model(Some(&model))
                .map_err(|error| std::io::Error::other(format!("{label} model: {error}")))?;
        }
        self.voice.vad.validate().map_err(std::io::Error::other)?;
        // Range-checked on the way OUT only. `load` deliberately accepts whatever
        // is on disk (and the loops clamp), because hard-failing a boot on a
        // legacy interval would take the whole install down over a schedule.
        self.learn.validate().map_err(std::io::Error::other)?;
        validate_persisted_appearance(&self.appearance).map_err(std::io::Error::other)?;
        if self
            .applied_preset
            .as_ref()
            .is_some_and(|snapshot| snapshot.resolved_model.is_some())
        {
            return Err(std::io::Error::other(
                "companion side store keeps Provider references only in the fixed model field",
            ));
        }
        crate::fsio::save_json_atomic(dir, "config.json", self)
    }
}

/// Session-window archiving settings (伙伴会话窗口归档): when a companion's chat
/// window goes idle for `idle_minutes`, compress it into a day-partitioned
/// digest, then reset the live engine context so the next window starts small.
/// Default OFF (opt-in), mirroring the learn loop — these background LLM loops
/// cost tokens and (here) reset live context, so the user opts in explicitly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SharedArchiveConfig {
    /// Master switch. Off = the archiver is a complete no-op (companion behaves
    /// exactly as before this feature).
    pub enabled: bool,
    /// Close & archive a window after this many minutes with no activity.
    pub idle_minutes: u32,
    /// Skip summarizing (roll boundary only, no digest, no reset) windows whose
    /// total content is shorter than this many chars — avoids burning tokens on
    /// trivial "hi/bye" sessions.
    pub min_chars: usize,
    /// How many recent day-digests to inject into a new window's system prompt.
    pub inject_recent_days: u32,
}

impl Default for SharedArchiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_minutes: 30,
            min_chars: 60,
            inject_recent_days: 3,
        }
    }
}

/// Cross-companion shared configuration persisted as `companion/shared/config.json`.
/// Deliberately user-writable (`PATCH /api/companion/config` merges arbitrary
/// user JSON over it), so nothing registry-owned (e.g. the companion-seq
/// watermark, which lives in `companion/shared/companion_seq.json`) may be
/// carried here.
///
/// What is left here is genuinely machine-level: WHICH events this device
/// records, the session archiver, and the default-companion pointer. 学习 and
/// 进化 moved onto [`CompanionProfileConfig`] — see [`Self::load_migrating`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SharedCompanionConfig {
    pub collect: CollectConfig,
    pub archive: SharedArchiveConfig,
    /// 智能协作（默认 OFF）：开启后，本地伙伴会话可通过
    /// `nomi_delegate` 把复杂工作交给多个 Agent，并在当前会话汇总结果。
    /// 能力由桌面网关的 Agent Execution 域提供，远程 IM 会话不注入。
    pub smart_collaboration: bool,
    /// Which companion new/unattributed activity defaults to.
    #[serde(deserialize_with = "deserialize_optional_companion_id")]
    pub default_companion_id: Option<String>,
    /// Opt-in (default None = off): when set to a directory path, companion
    /// `save` memories are ALSO mirrored into the nomi agent's file-memory there
    /// (the §3.4 "消两库割裂" bridge), so the agent recalls companion-learned
    /// facts. Enabling it intentionally surfaces companion memories in agent
    /// sessions — that is the feature; default-off keeps the libraries separate.
    pub bridge_to_memory_dir: Option<String>,
}

fn deserialize_optional_companion_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|raw| {
            CompanionId::try_from(raw.as_str())
                .map(CompanionId::into_string)
                .map_err(serde::de::Error::custom)
        })
        .transpose()
}

fn deserialize_optional_figure_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|raw| {
            FigureId::try_from(raw.as_str())
                .map(FigureId::into_string)
                .map_err(serde::de::Error::custom)
        })
        .transpose()
}

fn deserialize_companion_profile_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    CompanionId::try_from(raw.as_str())
        .map(CompanionId::into_string)
        .map_err(serde::de::Error::custom)
}

/// The install-wide `learn` / `evolve` blocks that a pre-2026-08
/// `shared/config.json` still carries, lifted off the file verbatim so the boot
/// migration can seed them onto the companions that have none of their own.
///
/// This type exists because [`SharedCompanionConfig`] is
/// `#[serde(deny_unknown_fields)]`: simply deleting the two fields would make
/// every existing `config.json` fail to parse, i.e. fail to boot. They are
/// therefore stripped during load — and, crucially, the stripped file is only
/// written back AFTER the seeding succeeded, so a crash in between re-reads the
/// same values next boot instead of losing them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetiredSharedLearnEvolve {
    pub learn: Option<serde_json::Value>,
    pub evolve: Option<serde_json::Value>,
}

impl RetiredSharedLearnEvolve {
    /// True once the file no longer carries either block — which is exactly what
    /// makes the migration idempotent, with no extra marker to keep in sync.
    pub fn is_empty(&self) -> bool {
        self.learn.is_none() && self.evolve.is_none()
    }
}

/// A freshly loaded shared config plus whatever retired install-wide state came
/// with it.
pub struct LoadedSharedConfig {
    pub config: SharedCompanionConfig,
    pub retired_learn_evolve: RetiredSharedLearnEvolve,
    /// True when the on-disk file still carries retired keys, so the caller must
    /// [`SharedCompanionConfig::save`] it once the migration has consumed them.
    pub needs_rewrite: bool,
}

impl SharedCompanionConfig {
    pub fn config_path(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    /// Load from `{dir}/config.json` (dir is the shared dir), dropping any
    /// retired setting on the way in. Only a missing file uses defaults;
    /// unreadable or malformed data fails closed.
    ///
    /// Retired keys with no successor (`evolve.reflect_enabled`, three old
    /// `collect.*` switches) are stripped and the file is rewritten immediately —
    /// nothing depends on their values. `learn` / `evolve`, which DID move
    /// somewhere, are handed to the caller instead and the rewrite is deferred
    /// until the caller has durably seeded them; see [`Self::load`] for the
    /// non-migrating shorthand.
    pub fn load_migrating(dir: &Path) -> Result<LoadedSharedConfig, nomifun_common::AppError> {
        let path = Self::config_path(dir);
        let loaded = crate::fsio::load_json_optional::<serde_json::Value>(&path).map_err(|error| {
            nomifun_common::AppError::Internal(format!(
                "load shared companion config {}: {error}",
                path.display()
            ))
        })?;
        let Some(mut value) = loaded else {
            return Ok(LoadedSharedConfig {
                config: Self::default(),
                retired_learn_evolve: RetiredSharedLearnEvolve::default(),
                needs_rewrite: false,
            });
        };
        let mut removed_legacy_settings = value
            .get_mut("evolve")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|evolve| evolve.remove("reflect_enabled"))
            .is_some();
        if let Some(collect) = value
            .get_mut("collect")
            .and_then(serde_json::Value::as_object_mut)
        {
            for key in [
                "chat_assistant_replies",
                "cron_runs",
                "conversation_lifecycle",
            ] {
                removed_legacy_settings |= collect.remove(key).is_some();
            }
        }
        let retired_learn_evolve = match value.as_object_mut() {
            Some(object) => RetiredSharedLearnEvolve {
                learn: object.remove("learn"),
                evolve: object.remove("evolve"),
            },
            None => RetiredSharedLearnEvolve::default(),
        };
        let config: Self = serde_json::from_value(value).map_err(|error| {
            nomifun_common::AppError::Internal(format!(
                "load shared companion config {}: {error}",
                path.display()
            ))
        })?;
        config.collect.validate_storage_policy().map_err(|error| {
            nomifun_common::AppError::Internal(format!(
                "load shared companion config {}: {error}",
                path.display()
            ))
        })?;
        // Retired-with-no-successor keys are rewritten away right here; the
        // moved ones wait for their migration.
        if removed_legacy_settings && retired_learn_evolve.is_empty() {
            config.save(dir).map_err(|error| {
                nomifun_common::AppError::Internal(format!(
                    "migrate shared companion config {}: {error}",
                    path.display()
                ))
            })?;
        }
        let needs_rewrite = !retired_learn_evolve.is_empty();
        Ok(LoadedSharedConfig {
            config,
            retired_learn_evolve,
            needs_rewrite,
        })
    }

    /// [`Self::load_migrating`] for callers that do not run the migration (tests,
    /// read-only inspection). Retired install-wide learn/evolve values are
    /// dropped on the floor rather than seeded, so never use this on the boot
    /// path.
    pub fn load(dir: &Path) -> Result<Self, nomifun_common::AppError> {
        Self::load_migrating(dir).map(|loaded| loaded.config)
    }

    /// Atomically persist to `{dir}/config.json` (unique temp file + rename).
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        self.collect
            .validate_storage_policy()
            .map_err(std::io::Error::other)?;
        crate::fsio::save_json_atomic(dir, "config.json", self)
    }
}

fn validate_persisted_model(model: Option<&ProviderWithModel>) -> Result<(), String> {
    let Some(model) = model else {
        return Ok(());
    };
    model.validate()?;
    if model.use_model.is_some() {
        return Err(
            "companion side-store model must use exactly {provider_id, model}".into(),
        );
    }
    Ok(())
}

fn validate_persisted_appearance(appearance: &CompanionWindowConfig) -> Result<(), String> {
    let Some(figure) = appearance.custom_figure.as_ref() else {
        return Ok(());
    };
    if !figure.aspect.is_finite() || figure.aspect <= 0.0 {
        return Err("custom figure aspect must be finite and greater than zero".into());
    }
    let values = [
        figure.head_box.x,
        figure.head_box.y,
        figure.head_box.w,
        figure.head_box.h,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err("custom figure head_box values must be finite".into());
    }
    if figure.head_box.x < 0.0
        || figure.head_box.y < 0.0
        || figure.head_box.w <= 0.0
        || figure.head_box.h < 0.0
        || figure.head_box.x + figure.head_box.w > 1.0
        || figure.head_box.y + figure.head_box.h > 1.0
    {
        return Err("custom figure head_box must fit inside normalized image bounds".into());
    }
    if !matches!(figure.size_tier.as_str(), "s" | "m" | "l") {
        return Err("custom figure size_tier must be one of s, m, l".into());
    }
    if figure
        .size_px
        .is_some_and(|size| !size.is_finite() || size <= 0.0)
    {
        return Err("custom figure size_px must be finite and greater than zero".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_roundtrip_and_default_on_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = CompanionProfileConfig::load(dir.path()).unwrap();
        assert_eq!(loaded, None);

        let mut profile = CompanionProfileConfig::new("毛球", "ink", 1);
        profile.model = Some(ProviderWithModel {
            provider_id: nomifun_common::ProviderId::new().into_string(),
            model: "claude-fable-5".into(),
            use_model: None,
        });
        profile.skills.enabled = vec!["mermaid".into()];
        profile.skills.disabled_auto = vec!["cron".into()];
        profile.appearance.companion_enabled = true;
        profile.save(dir.path()).unwrap();

        let again = CompanionProfileConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(again, profile);
        assert!(CompanionId::parse(&again.companion_id).is_ok());
        assert!(again.created_at > 0);
    }

    #[test]
    fn old_profile_without_skills_defaults_to_empty_configuration() {
        let companion_id = CompanionId::new().into_string();
        let raw = serde_json::json!({
            "companion_id": companion_id,
            "seq": 1,
            "name": "Old",
            "character": "ink",
            "persona": PersonaConfig::default(),
            "model": null,
            "appearance": CompanionWindowConfig::default(),
            "created_at": 1
        });
        let profile: CompanionProfileConfig = serde_json::from_value(raw).unwrap();
        assert!(profile.skills.enabled.is_empty());
        assert!(profile.skills.disabled_auto.is_empty());
    }

    #[test]
    fn effective_skill_names_trim_deduplicate_and_exclude_auto() {
        let config = CompanionSkillConfig {
            enabled: vec![" mermaid ".into(), "mermaid".into(), " ".into()],
            disabled_auto: vec![" cron ".into()],
        };

        assert_eq!(
            normalized_effective_skill_names(vec!["cron".into(), "todo".into()], &config),
            vec!["mermaid", "todo"]
        );
    }

    #[test]
    fn profile_new_falls_back_to_default_character() {
        let p = CompanionProfileConfig::new("无名", "", 1);
        assert_eq!(p.character, "mochi");
        let q = CompanionProfileConfig::new("有名", "boo", 1);
        assert_eq!(q.character, "boo");
    }

    #[test]
    fn corrupt_profile_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(CompanionProfileConfig::config_path(dir.path()), "{not json").unwrap();
        assert!(CompanionProfileConfig::load(dir.path()).is_err());
    }

    #[test]
    fn custom_figure_roundtrips_and_omits_absent_fields() {
        let dir = tempfile::tempdir().unwrap();

        // A profile with no custom_figure key deserializes to None and
        // serializes without the key (skip_serializing_if).
        let mut profile = CompanionProfileConfig::new("自定", "custom", 1);
        assert_eq!(profile.appearance.custom_figure, None);
        profile.save(dir.path()).unwrap();
        let raw = std::fs::read_to_string(CompanionProfileConfig::config_path(dir.path())).unwrap();
        assert!(!raw.contains("custom_figure"));

        let figure_id = FigureId::new().into_string();
        profile.appearance.custom_figure = Some(CustomFigureMeta {
            aspect: 0.9444,
            head_box: HeadBox { x: 0.321, y: 0.0, w: 0.281, h: 0.3 },
            size_tier: "m".into(),
            size_px: None,
            figure_id: None,
        });
        profile.save(dir.path()).unwrap();
        // A None figure_id / size_px must not appear in the JSON.
        let raw_none = std::fs::read_to_string(CompanionProfileConfig::config_path(dir.path())).unwrap();
        assert!(!raw_none.contains("figure_id"));
        assert!(!raw_none.contains("size_px"));
        let again = CompanionProfileConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(again, profile);
        let meta = again.appearance.custom_figure.unwrap();
        assert_eq!(meta.size_tier, "m");
        assert_eq!(meta.size_px, None);
        assert!((meta.head_box.w - 0.281).abs() < f32::EPSILON);

        // A library-linked figure_id + a per-companion size_px override round-trip.
        profile.appearance.custom_figure = Some(CustomFigureMeta {
            aspect: 0.9444,
            head_box: HeadBox { x: 0.321, y: 0.0, w: 0.281, h: 0.3 },
            size_tier: "m".into(),
            size_px: Some(333.0),
            figure_id: Some(figure_id.clone()),
        });
        profile.save(dir.path()).unwrap();
        let linked = CompanionProfileConfig::load(dir.path()).unwrap().unwrap();
        let linked_cf = linked.appearance.custom_figure.unwrap();
        assert_eq!(linked_cf.figure_id.as_deref(), Some(figure_id.as_str()));
        assert_eq!(linked_cf.size_px, Some(333.0));
    }

    #[test]
    fn shared_roundtrip_and_default_on_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = SharedCompanionConfig::load(dir.path()).unwrap();
        assert_eq!(loaded, SharedCompanionConfig::default());
        assert!(!loaded.collect.any_enabled());

        let mut cfg = SharedCompanionConfig::default();
        cfg.collect.chat_user_messages = true;
        cfg.archive.enabled = true;
        cfg.bridge_to_memory_dir = Some("/tmp/bridge".into());
        cfg.default_companion_id = Some(nomifun_common::CompanionId::new().into_string());
        cfg.save(dir.path()).unwrap();

        let again = SharedCompanionConfig::load(dir.path()).unwrap();
        assert_eq!(again, cfg);
    }

    /// 学习/进化 moved onto the profile; the shared config must no longer carry
    /// them, and a fresh profile must default to "off" exactly as the shared
    /// blocks used to.
    #[test]
    fn learn_and_evolve_live_on_the_profile_not_the_shared_config() {
        let wire = serde_json::to_value(SharedCompanionConfig::default()).unwrap();
        assert!(wire.get("learn").is_none());
        assert!(wire.get("evolve").is_none());

        let profile = CompanionProfileConfig::new("新宠", "ink", 1);
        assert!(!profile.learn.enabled);
        assert_eq!(profile.learn.interval_minutes, 60);
        assert!(profile.learn.model.is_none());
        assert!(!profile.evolve.enabled);
        assert!(!profile.evolve.auto_activate, "保守 is the default preference");
        assert_eq!(profile.evolve.min_distinct_sessions, 2);
        // The four tuning knobs stay Rust-side at their historical defaults.
        assert_eq!(profile.evolve.min_pattern_count, 3);
        assert!((profile.evolve.auto_threshold - 0.85).abs() < f64::EPSILON);
        assert!((profile.evolve.skill_half_life_days - 45.0).abs() < f64::EPSILON);
        assert!((profile.evolve.skill_archive_threshold - 0.05).abs() < f64::EPSILON);
    }

    /// A profile written before the move has no `learn`/`evolve` key at all. It
    /// must still load (both are `#[serde(default)]`) and must be reported as
    /// un-seeded so the boot migration can fill it in.
    #[test]
    fn a_pre_migration_profile_loads_and_reports_itself_unseeded() {
        let dir = tempfile::tempdir().unwrap();
        let profile = CompanionProfileConfig::new("老宠", "ink", 1);
        let mut raw = serde_json::to_value(&profile).unwrap();
        let object = raw.as_object_mut().unwrap();
        object.remove("learn");
        object.remove("evolve");
        std::fs::write(
            CompanionProfileConfig::config_path(dir.path()),
            serde_json::to_vec_pretty(&raw).unwrap(),
        )
        .unwrap();

        assert!(!CompanionProfileConfig::has_persisted_learn_or_evolve(dir.path()));
        let loaded = CompanionProfileConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.learn, CompanionLearnConfig::default());
        assert_eq!(loaded.evolve, CompanionEvolveConfig::default());

        // Once saved with the fields present, it is never re-seeded — even though
        // the values happen to equal the defaults.
        loaded.save(dir.path()).unwrap();
        assert!(CompanionProfileConfig::has_persisted_learn_or_evolve(dir.path()));
    }

    /// `deny_unknown_fields` + a removed field = every existing `config.json`
    /// fails to parse, i.e. fails to boot. The retired blocks must therefore come
    /// back to the caller instead of exploding, and the file must NOT be rewritten
    /// yet (the migration has not consumed them).
    #[test]
    fn retired_install_wide_learn_and_evolve_are_handed_to_the_migration() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(SharedCompanionConfig::default()).unwrap();
        value["learn"] = serde_json::json!({
            "enabled": true, "interval_minutes": 30, "model": null
        });
        value["evolve"] = serde_json::json!({
            "enabled": true, "interval_minutes": 45, "model": null,
            "min_pattern_count": 4, "min_distinct_sessions": 3,
            "auto_activate": true, "auto_threshold": 0.7,
            "skill_half_life_days": 30.0, "skill_archive_threshold": 0.1
        });
        std::fs::write(
            SharedCompanionConfig::config_path(dir.path()),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();

        let loaded = SharedCompanionConfig::load_migrating(dir.path()).unwrap();
        assert_eq!(loaded.config, SharedCompanionConfig::default());
        assert!(loaded.needs_rewrite);
        assert!(!loaded.retired_learn_evolve.is_empty());
        let learn: CompanionLearnConfig =
            serde_json::from_value(loaded.retired_learn_evolve.learn.clone().unwrap()).unwrap();
        assert!(learn.enabled);
        assert_eq!(learn.interval_minutes, 30);
        let evolve: CompanionEvolveConfig =
            serde_json::from_value(loaded.retired_learn_evolve.evolve.clone().unwrap()).unwrap();
        assert!(evolve.enabled);
        assert!(evolve.auto_activate);
        assert_eq!(evolve.min_distinct_sessions, 3);

        // Nothing rewritten yet: a crash here must find the same values next boot.
        let still: serde_json::Value = serde_json::from_slice(
            &std::fs::read(SharedCompanionConfig::config_path(dir.path())).unwrap(),
        )
        .unwrap();
        assert!(still.get("learn").is_some());

        // After the caller saves, the blocks are gone and the next load is a no-op.
        loaded.config.save(dir.path()).unwrap();
        let again = SharedCompanionConfig::load_migrating(dir.path()).unwrap();
        assert!(again.retired_learn_evolve.is_empty());
        assert!(!again.needs_rewrite);
    }

    #[test]
    fn quiet_hours_cover_same_day_overnight_and_disabled_windows() {
        let at = |start: &str, end: &str, hhmm: (u32, u32)| {
            CompanionWindowConfig {
                quiet_start: start.into(),
                quiet_end: end.into(),
                ..Default::default()
            }
            .in_quiet_hours_at(hhmm.0 * 60 + hhmm.1)
        };
        // Same-day window, half-open: the end minute is already awake.
        assert!(at("09:00", "17:00", (12, 0)));
        assert!(at("09:00", "17:00", (9, 0)));
        assert!(!at("09:00", "17:00", (17, 0)));
        assert!(!at("09:00", "17:00", (8, 59)));
        // Overnight window wraps midnight.
        assert!(at("22:00", "08:00", (23, 30)));
        assert!(at("22:00", "08:00", (0, 0)));
        assert!(at("22:00", "08:00", (7, 59)));
        assert!(!at("22:00", "08:00", (8, 0)));
        assert!(!at("22:00", "08:00", (12, 0)));
        // Empty / equal / malformed bounds all disable the window rather than
        // muting the companion forever.
        for (start, end) in [
            ("", ""),
            ("22:00", ""),
            ("", "08:00"),
            ("12:00", "12:00"),
            ("9:5", "17:00"),
            ("24:00", "08:00"),
            ("22:60", "08:00"),
            ("nope", "08:00"),
            ("2200", "0800"),
        ] {
            assert!(!at(start, end, (13, 0)), "{start:?}..{end:?} must disable quiet hours");
            assert!(!at(start, end, (2, 0)), "{start:?}..{end:?} must disable quiet hours");
        }
        // A one-digit hour is legal (matches the renderer's regex).
        assert!(at("9:00", "17:00", (10, 0)));
    }

    #[test]
    fn shared_load_removes_retired_collect_settings_and_rewrites_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(SharedCompanionConfig::default()).unwrap();
        value["collect"]["chat_assistant_replies"] = serde_json::json!(true);
        value["collect"]["cron_runs"] = serde_json::json!(true);
        value["collect"]["conversation_lifecycle"] = serde_json::json!(true);
        std::fs::write(
            SharedCompanionConfig::config_path(dir.path()),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();

        let loaded = SharedCompanionConfig::load(dir.path()).unwrap();
        assert_eq!(loaded, SharedCompanionConfig::default());
        let migrated: serde_json::Value = serde_json::from_slice(
            &std::fs::read(SharedCompanionConfig::config_path(dir.path())).unwrap(),
        )
        .unwrap();
        assert!(migrated["collect"].get("chat_assistant_replies").is_none());
        assert!(migrated["collect"].get("cron_runs").is_none());
        assert!(migrated["collect"].get("conversation_lifecycle").is_none());
    }

    /// `evolve.reflect_enabled` was retired long before the whole `evolve` block
    /// moved onto the profile. The nested strip must still run BEFORE the retired
    /// blob reaches `CompanionEvolveConfig` (`deny_unknown_fields`), or an install
    /// that still carries the key fails its boot migration.
    #[test]
    fn a_retired_key_inside_the_retired_evolve_block_never_reaches_the_strict_parser() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(SharedCompanionConfig::default()).unwrap();
        value["evolve"] = serde_json::json!({"enabled": true, "reflect_enabled": true});
        std::fs::write(
            SharedCompanionConfig::config_path(dir.path()),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();

        let loaded = SharedCompanionConfig::load_migrating(dir.path()).unwrap();
        let retired = loaded.retired_learn_evolve.evolve.clone().unwrap();
        assert!(retired.get("reflect_enabled").is_none());
        let evolve: CompanionEvolveConfig = serde_json::from_value(retired).unwrap();
        assert!(evolve.enabled, "the surviving setting still carries over");
    }

    #[test]
    fn shared_load_backfills_event_storage_policy_for_legacy_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(SharedCompanionConfig::default()).unwrap();
        value["collect"]["chat_user_messages"] = serde_json::json!(true);
        value["collect"].as_object_mut().unwrap().remove("event_retention_days");
        value["collect"].as_object_mut().unwrap().remove("event_max_storage_mb");
        std::fs::write(
            SharedCompanionConfig::config_path(dir.path()),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();

        let loaded = SharedCompanionConfig::load(dir.path()).unwrap();
        assert!(loaded.collect.chat_user_messages);
        assert_eq!(
            loaded.collect.event_retention_days,
            crate::config::DEFAULT_EVENT_RETENTION_DAYS
        );
        assert_eq!(
            loaded.collect.event_max_storage_mb,
            crate::config::DEFAULT_EVENT_MAX_STORAGE_MB
        );
    }

    #[test]
    fn shared_load_rejects_an_out_of_range_event_storage_policy() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(SharedCompanionConfig::default()).unwrap();
        value["collect"]["event_max_storage_mb"] = serde_json::json!(15);
        std::fs::write(
            SharedCompanionConfig::config_path(dir.path()),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();

        assert!(SharedCompanionConfig::load(dir.path()).is_err());
    }

    #[test]
    fn per_companion_evolve_config_still_rejects_unknown_settings() {
        let result = serde_json::from_value::<CompanionEvolveConfig>(serde_json::json!({
            "unknown_setting": true
        }));
        assert!(result.is_err());
    }

    #[test]
    fn shared_config_still_rejects_unknown_collection_settings() {
        let result = serde_json::from_value::<SharedCompanionConfig>(serde_json::json!({
            "collect": {"unknown_setting": true}
        }));
        assert!(result.is_err());
    }

    #[test]
    fn shared_config_rejects_retired_smart_orchestration_key() {
        let result = serde_json::from_value::<SharedCompanionConfig>(serde_json::json!({
            "smart_orchestration": true
        }));
        assert!(result.is_err());
    }

    #[test]
    fn shared_config_rejects_empty_or_malformed_default_companion_id() {
        for default_companion_id in ["", "not-a-companion-id"] {
            let result = serde_json::from_value::<SharedCompanionConfig>(serde_json::json!({
                "default_companion_id": default_companion_id
            }));
            assert!(result.is_err());
        }
    }

    #[test]
    fn profile_and_shared_models_persist_exact_provider_id_and_model_shape() {
        let canonical_provider = nomifun_common::ProviderId::new().into_string();
        let model = ProviderWithModel {
            provider_id: canonical_provider.clone(),
            model: "chat".into(),
            use_model: None,
        };

        let mut profile = CompanionProfileConfig::new("严格模型", "ink", 1);
        profile.model = Some(model.clone());
        let profile_json = serde_json::to_value(&profile).unwrap();
        assert_eq!(
            profile_json["model"],
            serde_json::json!({
                "provider_id": canonical_provider.clone(),
                "model": "chat"
            })
        );

        let mut owner = CompanionProfileConfig::new("双模型", "ink", 1);
        owner.learn.model = Some(model.clone());
        owner.evolve.model = Some(model);
        let owner_json = serde_json::to_value(owner).unwrap();
        for persisted in [&owner_json["learn"]["model"], &owner_json["evolve"]["model"]] {
            assert_eq!(
                persisted
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>(),
                ["model", "provider_id"].into_iter().collect()
            );
        }

        for invalid in [
            serde_json::json!({"provider_id": "", "model": "chat"}),
            serde_json::json!({"provider_id": "not-a-provider-id", "model": "chat"}),
            serde_json::json!({"provider_id": canonical_provider, "model": " "}),
            serde_json::json!({
                "provider_id": canonical_provider,
                "model": "chat",
                "use_model": "chat"
            }),
            serde_json::json!({
                "provider_id": canonical_provider,
                "model": "chat",
                "backend": "openai"
            }),
        ] {
            let result = serde_json::from_value::<CompanionLearnConfig>(serde_json::json!({
                "model": invalid
            }));
            assert!(
                result.is_err(),
                "non-v3 companion side-store model must be rejected"
            );
        }
    }

    #[test]
    fn legacy_profile_without_voice_slots_gets_documented_defaults() {
        let companion_id = CompanionId::new().into_string();
        let raw = serde_json::json!({
            "companion_id": companion_id,
            "seq": 1,
            "name": "Old",
            "character": "ink",
            "persona": PersonaConfig::default(),
            "model": null,
            "appearance": CompanionWindowConfig::default(),
            "created_at": 1
        });
        let profile: CompanionProfileConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(profile.fallback_model, None);
        assert_eq!(profile.vision_model, None);
        assert_eq!(profile.voice.asr, None);
        assert_eq!(profile.voice.tts, None);
        assert_eq!(profile.voice.vad.engine, DEFAULT_VAD_ENGINE);
        assert!((profile.voice.vad.sensitivity - DEFAULT_VAD_SENSITIVITY).abs() < f32::EPSILON);
        assert_eq!(profile.voice.vad.min_silence_ms, DEFAULT_VAD_MIN_SILENCE_MS);
    }

    #[test]
    fn every_model_slot_round_trips_in_the_v3_reference_shape() {
        let dir = tempfile::tempdir().unwrap();
        let provider_id = nomifun_common::ProviderId::new().into_string();
        let model = ProviderWithModel {
            provider_id: provider_id.clone(),
            model: "chat".into(),
            use_model: None,
        };

        let mut profile = CompanionProfileConfig::new("全槽位", "ink", 1);
        profile.model = Some(model.clone());
        profile.fallback_model = Some(model.clone());
        profile.vision_model = Some(model.clone());
        profile.voice.asr = Some(model.clone());
        profile.voice.tts = Some(CompanionTtsSelection {
            provider_id: provider_id.clone(),
            model: "tts-1".into(),
            voice: Some("alloy".into()),
        });
        profile.voice.vad.sensitivity = 0.7;
        profile.voice.vad.min_silence_ms = 900;
        profile.save(dir.path()).unwrap();

        let again = CompanionProfileConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(again, profile);

        // 侧存储只接受 {provider_id, model}；use_model 是运行时 DTO 概念。
        let json = serde_json::to_value(&profile).unwrap();
        for persisted in [&json["fallback_model"], &json["vision_model"], &json["voice"]["asr"]] {
            assert_eq!(
                persisted
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>(),
                ["model", "provider_id"].into_iter().collect()
            );
        }
        assert_eq!(json["voice"]["tts"]["voice"], serde_json::json!("alloy"));
        assert_eq!(json["voice"]["vad"]["engine"], serde_json::json!("silero"));
    }

    #[test]
    fn provider_model_slots_lists_every_hard_reference_exactly_once() {
        let provider_id = nomifun_common::ProviderId::new().into_string();
        let model = ProviderWithModel {
            provider_id: provider_id.clone(),
            model: "m".into(),
            use_model: None,
        };

        let empty = CompanionProfileConfig::new("空", "ink", 1);
        assert!(empty.provider_model_slots().is_empty());

        let mut full = CompanionProfileConfig::new("满", "ink", 1);
        full.model = Some(model.clone());
        full.learn.model = Some(model.clone());
        full.evolve.model = Some(model.clone());
        full.fallback_model = Some(model.clone());
        full.vision_model = Some(model.clone());
        full.voice.asr = Some(model.clone());
        full.voice.tts = Some(CompanionTtsSelection {
            provider_id: provider_id.clone(),
            model: "tts-1".into(),
            voice: None,
        });
        let slots = full.provider_model_slots();
        assert_eq!(
            slots.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
            ["chat", "learn", "evolve", "fallback", "vision", "asr", "tts"]
        );
        assert!(slots.iter().all(|(_, model)| model.provider_id == provider_id));
        assert_eq!(slots.last().unwrap().1.model, "tts-1");
    }

    #[test]
    fn vad_settings_are_clamped_on_read_and_range_checked_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let mut profile = CompanionProfileConfig::new("VAD", "ink", 1);

        profile.voice.vad.sensitivity = 9.0;
        profile.voice.vad.min_silence_ms = 99_999;
        assert!(profile.save(dir.path()).is_err());
        // load 故意宽容（旧档案不能拖垮启动），读取点负责收敛。
        assert!((profile.voice.vad.effective_sensitivity() - 1.0).abs() < f32::EPSILON);
        assert_eq!(
            profile.voice.vad.effective_min_silence_ms(),
            MAX_VAD_MIN_SILENCE_MS
        );

        profile.voice.vad.sensitivity = -1.0;
        profile.voice.vad.min_silence_ms = 1;
        assert!((profile.voice.vad.effective_sensitivity() - 0.0).abs() < f32::EPSILON);
        assert_eq!(
            profile.voice.vad.effective_min_silence_ms(),
            MIN_VAD_MIN_SILENCE_MS
        );

        profile.voice.vad.sensitivity = 0.5;
        profile.voice.vad.min_silence_ms = 700;
        profile.voice.vad.engine = " ".into();
        assert!(profile.save(dir.path()).is_err());
    }

    #[test]
    fn voice_and_new_slots_reject_unknown_fields() {
        assert!(
            serde_json::from_value::<CompanionVoiceConfig>(serde_json::json!({"engine": "x"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<CompanionVadConfig>(
                serde_json::json!({"engine": "silero", "threshold": 1})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<CompanionTtsSelection>(serde_json::json!({
                "provider_id": "not-a-provider-id",
                "model": "tts-1"
            }))
            .is_err()
        );
    }

    #[test]
    fn corrupt_shared_config_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(SharedCompanionConfig::config_path(dir.path()), "[oops").unwrap();
        assert!(SharedCompanionConfig::load(dir.path()).is_err());
    }
}
