# Plan B — 模型体系与 UI Implementation Plan

> [!CAUTION]
> **模型体系与 UI 部分已被 2026-08-11 现行规范替代。** 本文只保留为历史执行计划，不再约束模型管理实现；尤其不得继续执行其中“旧模型表不改”、客户端任务投影、旧选择接口或“不得修复既有债务”等指令。当前单一能力源、九模态统一编辑器、后端协议 manifest、自由模型 ID 和每模态 transport 配置以[《模型供应商 × 模态官方接口核验矩阵（2026-08-11）》](../../specs/2026-08-11-provider-modality-official-matrix.zh.md)为准。本文其余历史正文未重写。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把伙伴档案扩成五类模型槽位、给 TTS 补上全局偏好与配置面、把模型管理页从「供应商为主」翻成「模态为主」，并在伙伴远程 Tab 加出「机器人连接」专项。

**Architecture:** 后端只动两处数据契约——`CompanionProfileConfig` 增加 `fallback_model` / `vision_model` / `voice` 三个 serde-default 字段（`crates/backend/nomifun-companion`），以及新偏好键 `tools.textToSpeech` 的类型 + provider 引用注册（`nomifun-api-types` / `nomifun-db` / `nomifun-shell`）；`provider_models` 表与 invoke 层零改动。前端收敛出一个共享的 `TaskModelSelect`（`(task, traits)` → 供应商+模型二联，TTS 变体带音色第三联），总览页与模型管理页都靠它，模型管理页的模态分区是 `provider_models` 行按 `tasks`/`traits` 的客户端投影。机器人 UI 只消费 Plan A 的 REST + `robot.status` 事件，契约先落 `ipcBridge.ts` 并由 wire 契约测试钉住。

**Tech Stack:** Rust（serde / axum / sqlx，`cargo test -p <crate>`）、TypeScript + React 19 + Arco Design + UnoCSS（`bun test --cwd ui`）、SWR、i18next。

## Global Constraints

- Git 署名：绝不让 AI 出现在 author / committer / co-author；绝不加 `Co-authored-by` / `Generated-by` / `Assisted-by` 等 trailer；不得 `--no-verify`；作者固定 `RiKa0-0 <2206491416@qq.com>`（仓库 `git config` 已是该值，直接 `git commit` 即可）。
- 绝不在 `.github/workflows/` 下创建任何文件。
- commit message 用英文 conventional commits；本计划正文中文，代码 / 标识符 / commit message 英文。
- 本仓库 Linux 全量测试有 14 个既有失败。任务内**只跑目标包 / 目标文件**的测试，绝不跑 `bun run test` / `cargo test` 全量。
- Rust 侧粒度：`cargo test -p <crate> <模块过滤>`（已实测：`cargo test -p nomifun-companion --lib profile::tests` → 21 passed）。
- TS 侧粒度：`bun test --cwd ui <相对 ui/ 的路径>`（已实测：`bun test --cwd ui src/renderer/pages/settings/SshHostSettings/SshHostManagement.structure.test.ts` → 16 pass）。类型检查 `bun run typecheck`。
- UI i18n zh/en 必须成对添加，`bun run check:i18n` 会校验 `localeKeyParity`；改完 locale JSON **必须**跑 `bun run gen:i18n` 重新生成 `ui/src/renderer/services/i18n/i18n-keys.d.ts`，否则 `check:i18n --check` 红。
- 面向用户的 UI 文案一律中文；`en-US` 同名键写英文。
- **`ui/src/renderer/pages/nomi/**` 下所有文件受两个既有结构测试约束**（`workspace/shell.structure.test.ts`、`workspace/rulesOfHooks.test.ts`）：
  - 文件必须以 `/**\n * @license` 开头；
  - 代码行（注释除外）**不得出现 `/suggestion/i`**（所以音色候选列表的 prop 叫 `voiceOptions`，不叫 `voiceSuggestions`）；
  - 禁 `border-border-<数字>`、禁 `border-b-base` / `border-b-light`、禁 `text-[rgb(var(--danger-6))]` 这类裸 ramp 任意值（用 `text-danger-6`）；
  - 图标只能从 `@icon-park/react` 具名导入且**不得别名**；
  - 组件内所有 hook 必须在任何 early `return` **之前**。
- Plan A 侧 VAD 引擎名**只认 `"silero"`**，其他任何值都回落内置能量 VAD。所以模型管理页语音区的 VAD 条目是「内置 Silero VAD（本地）」文字条目 + 两个参数（灵敏度、停顿判停时长），**不是模型选择器**；伙伴级参数写 `voice.vad`。
- §7 提到的既有债务**不在本期修复范围**：P1（`provider_models.protocol` 一列被 new-api 徽章与 invoke 协议 id 两套词表争用）、P2（前端 `MODEL_PROTOCOL_OPTIONS` 相对后端 16 个 adapter 已漂移）、P3（新增模型走整-provider PUT + 行级 upsert 的双写路径）、P4（wire 上的 legacy 投影字段）、P7（`ModelModalContent.tsx` 1308 行）。执行者**不得**顺手重构它们。
- 同理**不在本期范围**：把 `ModelModalContent` 里的 `ModelModalityEditor` / `ModelDescriptionEditor` 抽成公共组件；模态分区里的「任务归属打标」一律跳转到「供应商与密钥」区完成（打标编辑器保持单一权威，避免再造一条双写路径）。
- 会话页 / 知识库 / 引导页 / 创作工坊等其余 8 处模型选择器**本期不迁移**到 `TaskModelSelect`（spec §7 明确「不强求本期完成」）。

## 跨计划依赖

- **Plan A 依赖本计划的 Task 1**（`CompanionProfileConfig` 的 `fallback_model` / `vision_model` / `voice` 字段与 `provider_model_slots()`）。Task 1 必须最先合入 main。
- **本计划的 Task 12-14 依赖 Plan A 的后端**（`/api/robots*` 六条路由 + 用户级 WS 事件 `robot.status`）。Task 12 可以先落地——契约即真相，wire 契约测试只读 `ipcBridge.ts` 源码与 mock 过的 `fetch`，不需要后端在线；Task 14 的 section 在后端缺席时必须渲染成「读取失败」的空态而不是崩。
- 本计划**不实现**任何 `/api/robots*` 路由、不新建 crate、不碰 `nomifun-robot`。

## 文件结构清单

新建：

```
crates/backend/nomifun-companion/src/profile.rs           (改：三个新字段 + CompanionVoiceConfig/TtsSelection/VadConfig + provider_model_slots)
ui/src/renderer/components/model/taskModelSelectState.ts  (新：纯状态函数)
ui/src/renderer/components/model/taskModelSelectState.test.ts
ui/src/renderer/components/model/TaskModelSelect.tsx      (新：共享二/三联选择器)
ui/src/renderer/components/model/TaskModelSelect.structure.test.ts
ui/src/renderer/components/model/ttsVoiceOptions.ts       (新：已知供应商音色候选)
ui/src/renderer/services/textToSpeechConfig.ts            (新：tools.textToSpeech 读写)
ui/src/renderer/services/textToSpeechConfig.test.ts
ui/src/renderer/pages/modelHub/modalityModels.ts          (新：provider_models 模态投影)
ui/src/renderer/pages/modelHub/modalityModels.test.ts
ui/src/renderer/pages/modelHub/ModalityModelsPanel.tsx    (新：模态分区通用面板)
ui/src/renderer/pages/modelHub/ChatModelsContent.tsx      (新)
ui/src/renderer/pages/modelHub/VisionModelsContent.tsx    (新)
ui/src/renderer/pages/modelHub/EmbeddingModelsContent.tsx (新)
ui/src/renderer/pages/modelHub/SpeechModelsContent.tsx    (新：语音区宿主 = ASR + TTS + VAD)
ui/src/renderer/pages/modelHub/TextToSpeechContent.tsx    (新：TTS 全局默认配置面)
ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/RobotConnectSection.tsx      (新)
ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/AddRobotModal.tsx            (新)
ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.ts          (新)
ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/RobotConnectSection.structure.test.ts
ui/src/common/adapter/ipcBridge.robot-status-wire.test.ts (新)
ui/src/renderer/services/i18n/robotLocales.test.ts        (新)
```

修改（关键行区间在各任务里点名）：

```
crates/backend/nomifun-companion/src/config.rs
crates/backend/nomifun-companion/src/registry.rs
crates/backend/nomifun-companion/src/service.rs
crates/backend/nomifun-api-types/src/shell.rs
crates/backend/nomifun-api-types/src/lib.rs
crates/backend/nomifun-db/src/repository/client_preference.rs
crates/backend/nomifun-shell/src/routes.rs
ui/src/common/adapter/ipcBridge.ts
ui/src/common/config/configKeys.ts
ui/src/common/types/provider/speech.ts
ui/src/renderer/pages/nomi/useNomi.ts
ui/src/renderer/pages/nomi/CompanionModelControl.tsx
ui/src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.tsx
ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/index.tsx
ui/src/renderer/pages/modelHub/index.tsx
ui/src/renderer/pages/modelHub/SpeechToTextContent.tsx
ui/src/renderer/pages/modelHub/modelConfigurationPlacement.test.ts
ui/src/renderer/services/speechToTextConfig.ts
ui/src/renderer/components/capability/capabilityStatusColors.ts
ui/src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.ts
ui/src/renderer/services/i18n/locales/{zh-CN,en-US}/nomi.json
ui/src/renderer/services/i18n/locales/{zh-CN,en-US}/settings.json
ui/src/renderer/services/i18n/i18n-keys.d.ts  (生成物，勿手改)
```

---

### Task 1: 伙伴档案模型槽位契约（Rust）

**Files:**
- Modify: `crates/backend/nomifun-companion/src/config.rs`（`deserialize_provider_id` 现为私有 fn，见 `:108`；改成 `pub(crate)`）
- Modify: `crates/backend/nomifun-companion/src/profile.rs`（新类型插在 `CompanionEvolveConfig` 之后即 `:295` 附近；`CompanionProfileConfig` 字段 `:297-346`；`new()` `:351-369`；`load()` `:377-435` 的 3 项模型校验循环 `:407-417`；`save()` `:458-477`）
- Modify: `crates/backend/nomifun-companion/src/registry.rs`（`validate_provider_references_under_guard` `:595-619`）
- Modify: `crates/backend/nomifun-companion/src/service.rs`（`providers_in_use` `:522-556`）
- Test: `crates/backend/nomifun-companion/src/profile.rs`（`mod tests`，`:764` 起）

**Interfaces:**
- Produces（Plan A 与本计划 Task 2 之后的任务都消费）：
  - `pub struct CompanionVoiceConfig { pub asr: Option<ProviderWithModel>, pub tts: Option<CompanionTtsSelection>, pub vad: CompanionVadConfig }`
  - `pub struct CompanionTtsSelection { pub provider_id: String, pub model: String, pub voice: Option<String> }`
  - `pub struct CompanionVadConfig { pub engine: String, pub sensitivity: f32, pub min_silence_ms: u32 }`
  - `CompanionProfileConfig { …, pub fallback_model: Option<ProviderWithModel>, pub vision_model: Option<ProviderWithModel>, pub voice: CompanionVoiceConfig, … }`
  - `impl CompanionTtsSelection { pub fn as_provider_model(&self) -> ProviderWithModel }`
  - `impl CompanionVadConfig { pub fn effective_sensitivity(&self) -> f32; pub fn effective_min_silence_ms(&self) -> u32 }`
  - `impl CompanionProfileConfig { pub fn provider_model_slots(&self) -> Vec<(&'static str, ProviderWithModel)> }` —— slot 标签取值 `"chat" | "learn" | "evolve" | "fallback" | "vision" | "asr" | "tts"`
  - `pub const DEFAULT_VAD_ENGINE: &str = "silero";`、`DEFAULT_VAD_SENSITIVITY: f32 = 0.5`、`DEFAULT_VAD_MIN_SILENCE_MS: u32 = 700`、`MIN_VAD_MIN_SILENCE_MS: u32 = 200`、`MAX_VAD_MIN_SILENCE_MS: u32 = 3000`
- Consumes: 既有 `nomifun_common::ProviderWithModel`、`crate::config::{deserialize_optional_model, serialize_optional_model}`。

- [ ] **Step 1: 写失败测试**

在 `crates/backend/nomifun-companion/src/profile.rs` 的 `mod tests` 里追加（放在 `corrupt_shared_config_fails_closed` 之前）：

```rust
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
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-companion --lib profile::tests`
Expected: FAIL — 编译错误 `cannot find type CompanionVoiceConfig in this scope` / `no field fallback_model on type CompanionProfileConfig` / `cannot find value DEFAULT_VAD_ENGINE`。

- [ ] **Step 3: 写最小实现**

3a. `crates/backend/nomifun-companion/src/config.rs:108` —— 把 `deserialize_provider_id` 开放给同 crate：

```rust
pub(crate) fn deserialize_provider_id<'de, D>(deserializer: D) -> Result<String, D::Error>
```

3b. `crates/backend/nomifun-companion/src/profile.rs` —— 在 `impl CompanionEvolveConfig { … }` 结束（`:295`）之后插入：

```rust
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

/// One companion's speech-synthesis选择: which catalog model speaks, and in
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
```

3c. `CompanionProfileConfig`（`:321` 的 `model` 字段之后）插入三个字段：

```rust
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
```

3d. `CompanionProfileConfig::new`（`:354-368` 的字面量）加三行，紧跟 `model: None,`：

```rust
            fallback_model: None,
            vision_model: None,
            voice: CompanionVoiceConfig::default(),
```

3e. 在 `impl CompanionProfileConfig` 里（`config_path` 之前）加 slot 清单：

```rust
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
```

3f. `load()` —— 把 `:401-417` 的「chat 单独校验 + learn/evolve 循环」整段替换为一个 slot 循环：

```rust
        for (label, model) in profile.provider_model_slots() {
            validate_persisted_model(Some(&model)).map_err(|error| {
                nomifun_common::AppError::Internal(format!(
                    "companion profile {} has invalid {label} model: {error}",
                    path.display()
                ))
            })?;
        }
```

3g. `save()` —— 把 `:459-461` 的三行 `validate_persisted_model` 替换为：

```rust
        for (label, model) in self.provider_model_slots() {
            validate_persisted_model(Some(&model))
                .map_err(|error| std::io::Error::other(format!("{label} model: {error}")))?;
        }
        self.voice.vad.validate().map_err(std::io::Error::other)?;
```

3h. `crates/backend/nomifun-companion/src/registry.rs:598-618` —— 审计循环改读 slot 清单：

```rust
        let profiles: Vec<_> = self.inner.read().await.values().cloned().collect();
        for profile in profiles {
            // Every slot in `provider_model_slots()` is a hard binding: a missing
            // Provider is an orphaned reference and fails startup. Provider
            // deletion is refused while any of them points at it
            // (`CompanionService::providers_in_use`), so an orphan means the
            // durable state was edited behind the app's back.
            for (what, model) in profile.provider_model_slots() {
                validate_provider_model(self.provider_repo.as_ref(), Some(&model))
                    .await
                    .map_err(|error| {
                        AppError::Internal(format!(
                            "companion '{}' has an orphaned {what} provider reference: {error}",
                            profile.companion_id
                        ))
                    })?;
            }
        }
        Ok(())
```

3i. `crates/backend/nomifun-companion/src/registry.rs:560-562`（`patch()` 里的三行 `validate_provider_model`）替换为：

```rust
        for (_, model) in merged.provider_model_slots() {
            validate_provider_model(self.provider_repo.as_ref(), Some(&model)).await?;
        }
```

3j. `crates/backend/nomifun-companion/src/service.rs:538-554`（`providers_in_use` 的内层循环）替换为：

```rust
        let mut out = Vec::new();
        for p in self.list_companions().await {
            for (slot, model) in p.provider_model_slots() {
                if model.provider_id != provider_id.as_str() {
                    continue;
                }
                let suffix = slot_display_label(slot);
                out.push(ProviderUsage {
                    feature: ProviderUsageFeature::DesktopCompanion,
                    label: if suffix.is_empty() {
                        p.name.clone()
                    } else {
                        format!("{}·{suffix}", p.name)
                    },
                    target_id: Some(p.companion_id.clone()),
                });
            }
        }
        out
```

并在 `crates/backend/nomifun-companion/src/service.rs` 文件末尾（`mod tests` 之前）加：

```rust
/// Display suffix for one profile Provider slot in the deletion-blocking usage
/// report. Empty = the companion's own chat model, which needs no suffix.
fn slot_display_label(slot: &str) -> &'static str {
    match slot {
        "learn" => "学习模型",
        "evolve" => "进化模型",
        "fallback" => "备用模型",
        "vision" => "视觉模型",
        "asr" => "语音识别模型",
        "tts" => "语音合成模型",
        _ => "",
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-companion --lib profile::tests registry::tests`
Expected: PASS
Run: `cargo test -p nomifun-companion --lib service::tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-companion/src/config.rs \
        crates/backend/nomifun-companion/src/profile.rs \
        crates/backend/nomifun-companion/src/registry.rs \
        crates/backend/nomifun-companion/src/service.rs
git commit -m "feat(companion): add fallback, vision and voice model slots to the profile"
```

---

### Task 2: `tools.textToSpeech` 全局偏好的后端支撑

**Files:**
- Modify: `crates/backend/nomifun-api-types/src/shell.rs`（TTS 段 `:51-70` 之后加类型；`mod tests` `:189` 起加测试）
- Modify: `crates/backend/nomifun-api-types/src/lib.rs:209-211`（shell 的 re-export 清单）
- Modify: `crates/backend/nomifun-db/src/repository/client_preference.rs`（键常量 `:4-10`、`provider_preference_kind` `:69-83`、`mod provider_reference_tests` `:370` 起）
- Modify: `crates/backend/nomifun-shell/src/routes.rs`（`speech_to_text_config_from_preferences` `:266` 旁加 TTS 读取；`mod tests` `:375` 起）
- Test: 同上三个文件的 `mod tests`

**Interfaces:**
- Produces:
  - `nomifun_api_types::TextToSpeechConfig { pub provider_id: String, pub model: String, pub voice: Option<String> }`（`Deserialize`，`deny_unknown_fields`，`provider_id` 走 `deserialize_provider_id`，`model` 走 `deserialize_model_name`）
  - `nomifun_api_types::TEXT_TO_SPEECH_PREFERENCE_KEY: &str = "tools.textToSpeech"`
  - `nomifun_api_types::TextToSpeechConfig::from_preferences(prefs: &ClientPreferencesResponse) -> Option<TextToSpeechConfig>`
  - `nomifun-db` 侧：键 `tools.textToSpeech` 注册为 `ProviderPreferenceKind::RequiredModelObject`，于是写入时校验 provider 存在、provider 删除时该键被删除
- Consumes: 既有 `ClientPreferencesResponse = HashMap<String, Value>`（`nomifun-api-types/src/system.rs:61`）、`crate::serde_util::{deserialize_provider_id, deserialize_model_name}`。

- [ ] **Step 1: 写失败测试**

1a. `crates/backend/nomifun-api-types/src/shell.rs` 的 `mod tests` 末尾追加：

```rust
    // -- TextToSpeechConfig --

    #[test]
    fn text_to_speech_config_reads_the_tools_preference_key() {
        let provider_id = "0190f5fe-7c00-7a00-8000-0000000000aa";
        let prefs = ClientPreferencesResponse::from([(
            TEXT_TO_SPEECH_PREFERENCE_KEY.to_owned(),
            json!({ "provider_id": provider_id, "model": "tts-1", "voice": "alloy" }),
        )]);
        let config = TextToSpeechConfig::from_preferences(&prefs).unwrap();
        assert_eq!(config.provider_id, provider_id);
        assert_eq!(config.model, "tts-1");
        assert_eq!(config.voice.as_deref(), Some("alloy"));
    }

    #[test]
    fn text_to_speech_config_has_no_enabled_switch_and_fails_closed() {
        let provider_id = "0190f5fe-7c00-7a00-8000-0000000000aa";
        // Presence of the key IS the configuration — an `enabled` field would be
        // a second source of truth and is rejected outright.
        assert!(
            serde_json::from_value::<TextToSpeechConfig>(json!({
                "provider_id": provider_id,
                "model": "tts-1",
                "voice": null,
                "enabled": true
            }))
            .is_err()
        );
        for invalid in [
            json!({ "provider_id": "prov_legacy", "model": "tts-1", "voice": null }),
            json!({ "provider_id": provider_id, "model": " ", "voice": null }),
            json!({ "model": "tts-1", "voice": null }),
        ] {
            assert!(serde_json::from_value::<TextToSpeechConfig>(invalid).is_err());
        }
        // An absent or malformed preference is "no global default", never a panic.
        assert!(TextToSpeechConfig::from_preferences(&ClientPreferencesResponse::new()).is_none());
        let broken = ClientPreferencesResponse::from([(
            TEXT_TO_SPEECH_PREFERENCE_KEY.to_owned(),
            json!("nonsense"),
        )]);
        assert!(TextToSpeechConfig::from_preferences(&broken).is_none());
    }
```

并在该 `mod tests` 顶部的 `use super::*;` 之后确认 `use serde_json::json;` 已存在（`:192` 已有）。

1b. `crates/backend/nomifun-db/src/repository/client_preference.rs` 的 `mod provider_reference_tests` 里，把 `registry_extracts_every_supported_provider_reference_shape` 的 `cases` 数组补一项（放在 `SPEECH_TO_TEXT_KEY` 之后）：

```rust
            (
                TEXT_TO_SPEECH_KEY,
                serde_json::json!({"provider_id": PROVIDER_A, "model": "tts-1", "voice": null})
                    .to_string(),
                1,
            ),
```

并追加一个新测试：

```rust
    #[test]
    fn text_to_speech_preference_is_a_required_model_reference() {
        // A malformed global TTS default must be refused at the write boundary,
        // not stored and then discovered by the robot gateway at speak time.
        for value in [
            r#"{"model":"tts-1"}"#,
            r#"{"provider_id":"prov_legacy","model":"tts-1"}"#,
            r#"{"provider_id":"0190f5fe-7c00-7a00-8000-000000000001","model":" "}"#,
        ] {
            assert!(normalize_provider_preference(TEXT_TO_SPEECH_KEY, value).is_err());
        }
        // Deleting the Provider deletes the default outright — a half-broken
        // default would silently pick the wrong voice on the next turn.
        assert_eq!(
            provider_preference_delete_action(
                TEXT_TO_SPEECH_KEY,
                &serde_json::json!({"provider_id": PROVIDER_A, "model": "tts-1", "voice": "alloy"})
                    .to_string(),
                PROVIDER_A,
            )
            .unwrap(),
            ProviderPreferenceDeleteAction::Delete
        );
    }
```

1c. `crates/backend/nomifun-shell/src/routes.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn text_to_speech_preference_is_read_through_the_shared_reader() {
        let provider_id = "0190f5fe-7c00-7a00-8000-0000000000aa";
        let prefs = ClientPreferencesResponse::from([(
            "tools.textToSpeech".into(),
            json!({ "provider_id": provider_id, "model": "tts-1", "voice": "alloy" }),
        )]);
        let config = text_to_speech_config_from_preferences(&prefs).unwrap();
        assert_eq!(config.provider_id, provider_id);
        assert_eq!(config.model, "tts-1");
        assert_eq!(config.voice.as_deref(), Some("alloy"));
        // Unlike STT there is no legacy un-namespaced key and no enabled switch.
        let legacy_only =
            ClientPreferencesResponse::from([("textToSpeech".into(), json!({"model": "tts-1"}))]);
        assert!(text_to_speech_config_from_preferences(&legacy_only).is_none());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p nomifun-api-types --lib shell::tests`
Expected: FAIL with `cannot find type TextToSpeechConfig in this scope`
Run: `cargo test -p nomifun-db --lib client_preference`
Expected: FAIL with `cannot find value TEXT_TO_SPEECH_KEY in this scope`
Run: `cargo test -p nomifun-shell --lib routes::tests`
Expected: FAIL with `cannot find function text_to_speech_config_from_preferences in this scope`

- [ ] **Step 3: 写最小实现**

3a. `crates/backend/nomifun-api-types/src/shell.rs` —— 在 `TtsApiRequest`（`:70`）之后插入：

```rust
/// Preference key holding the install-wide speech-synthesis default.
/// Deliberately parallel to `tools.speechToText`, minus the `enabled` switch:
/// speech synthesis has no input-box affordance to gate, so the key's PRESENCE
/// is the configuration and a second boolean would only be able to disagree
/// with it.
pub const TEXT_TO_SPEECH_PREFERENCE_KEY: &str = "tools.textToSpeech";

/// The install-wide speech-synthesis default: which catalog model speaks and in
/// which provider voice. Every companion whose `voice.tts` slot is empty falls
/// back to this.
///
/// There is no legacy un-namespaced twin (the key is new in this release), so
/// unlike [`SpeechToTextConfig`] there is nothing to migrate and no embedded
/// credential shape to reject.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextToSpeechConfig {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    /// Provider voice id (free text). `None` = the provider's own default voice.
    #[serde(default)]
    pub voice: Option<String>,
}

impl TextToSpeechConfig {
    /// Read the global default out of a preferences snapshot. A missing or
    /// malformed value answers `None` — "no global default" — because a caller
    /// that cannot synthesize must say so, not fail the whole request.
    pub fn from_preferences(
        prefs: &crate::ClientPreferencesResponse,
    ) -> Option<Self> {
        prefs
            .get(TEXT_TO_SPEECH_PREFERENCE_KEY)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
}
```

3b. `crates/backend/nomifun-api-types/src/lib.rs:209-212` 的 shell re-export 里加 `TEXT_TO_SPEECH_PREFERENCE_KEY, TextToSpeechConfig,`（保持字母序，`TextToSpeechConfig` 排在 `SpeechToTextResult` 之后）。

3c. `crates/backend/nomifun-db/src/repository/client_preference.rs:10` 之后加常量并注册：

```rust
const SPEECH_TO_TEXT_KEY: &str = "tools.speechToText";
const TEXT_TO_SPEECH_KEY: &str = "tools.textToSpeech";
```

`provider_preference_kind`（`:74-77`）改为：

```rust
        NOMI_DEFAULT_MODEL_KEY
        | KNOWLEDGE_AUTOGEN_MODEL_KEY
        | IMAGE_GENERATION_MODEL_KEY
        | TEXT_TO_SPEECH_KEY => Some(ProviderPreferenceKind::RequiredModelObject),
        SPEECH_TO_TEXT_KEY => Some(ProviderPreferenceKind::OptionalObjectProviderId),
```

（`RequiredModelObject` 而非 `OptionalObjectProviderId`：TTS 偏好没有 enabled 开关，`provider_id` 与 `model` 都是必填，provider 被删时整键删除 = 「没有全局默认」，比留一个半坏的引用诚实。）

3d. `crates/backend/nomifun-shell/src/routes.rs` —— 在 `speech_to_text_config_from_preferences`（`:281` 结束）之后插入读取函数，并把 import（`:8-12`）补上 `TextToSpeechConfig`：

```rust
/// The install-wide speech-synthesis default, or `None` when the user has not
/// picked one. Mirrors [`speech_to_text_config_from_preferences`] minus the
/// legacy-key fallback: `tools.textToSpeech` has no un-namespaced predecessor.
///
/// Read here rather than inside `/api/tts` on purpose — that route takes its
/// `(provider_id, model)` from the request body. This is the resolver the
/// companion/robot voice paths consult when a companion's own `voice.tts` slot
/// is empty.
pub fn text_to_speech_config_from_preferences(
    prefs: &ClientPreferencesResponse,
) -> Option<TextToSpeechConfig> {
    TextToSpeechConfig::from_preferences(prefs)
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p nomifun-api-types --lib shell::tests`
Expected: PASS
Run: `cargo test -p nomifun-db --lib client_preference`
Expected: PASS
Run: `cargo test -p nomifun-shell --lib routes::tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/backend/nomifun-api-types/src/shell.rs \
        crates/backend/nomifun-api-types/src/lib.rs \
        crates/backend/nomifun-db/src/repository/client_preference.rs \
        crates/backend/nomifun-shell/src/routes.rs
git commit -m "feat(tts): add the tools.textToSpeech install-wide preference"
```

---

### Task 3: 伙伴档案的 TS 镜像与乐观合并

**Files:**
- Modify: `ui/src/common/adapter/ipcBridge.ts`（`ICompanionModelRef` `:4696-4701`；`ICompanionProfile` `:4749-4771`；`ICompanionProfilePatch` `:4842-4852`）
- Modify: `ui/src/renderer/pages/nomi/useNomi.ts:19-27`（`mergeProfile`）
- Test: `ui/src/renderer/pages/nomi/companionVoiceSlots.test.ts`（新建）

**Interfaces:**
- Consumes: Task 1 的 Rust wire 形状（`fallback_model` / `vision_model` / `voice` 恒序列化，不会被省略）。
- Produces:
  - `interface ICompanionTtsSelection { provider_id: ProviderId; model: string; voice: string | null }`
  - `interface ICompanionVadConfig { engine: string; sensitivity: number; min_silence_ms: number }`
  - `interface ICompanionVoiceConfig { asr: ICompanionModelRef | null; tts: ICompanionTtsSelection | null; vad: ICompanionVadConfig }`
  - `ICompanionProfile` 新增 `fallback_model: ICompanionModelRef | null`、`vision_model: ICompanionModelRef | null`、`voice: ICompanionVoiceConfig`
  - `ICompanionProfilePatch` 新增 `fallback_model?: ICompanionModelRef | null`、`vision_model?: ICompanionModelRef | null`、`voice?: { asr?: ICompanionModelRef | null; tts?: ICompanionTtsSelection | null; vad?: Partial<ICompanionVadConfig> }`
  - `mergeProfile` 对 `voice` 做两级合并（`voice` 浅合并 + `voice.vad` 再浅合并）

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/pages/nomi/companionVoiceSlots.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const bridge = readFileSync(new URL('../../common/adapter/ipcBridge.ts', import.meta.url), 'utf8');
const useNomi = readFileSync(new URL('./useNomi.ts', import.meta.url), 'utf8');

describe('companion model-slot wire mirror', () => {
  test('the profile declares every slot the Rust struct serializes', () => {
    // The backend serializes these unconditionally (no skip_serializing_if), so
    // the type must not mark them optional — an optional field here would let a
    // consumer read `undefined` where the wire always sends `null`.
    expect(bridge.includes('fallback_model: ICompanionModelRef | null;')).toBe(true);
    expect(bridge.includes('vision_model: ICompanionModelRef | null;')).toBe(true);
    expect(bridge.includes('voice: ICompanionVoiceConfig;')).toBe(true);
    expect(bridge.includes('export interface ICompanionVoiceConfig')).toBe(true);
    expect(bridge.includes('export interface ICompanionTtsSelection')).toBe(true);
    expect(bridge.includes('export interface ICompanionVadConfig')).toBe(true);
    expect(bridge.includes('min_silence_ms: number;')).toBe(true);
  });

  test('the patch type can address one voice sub-field at a time', () => {
    const start = bridge.indexOf('export type ICompanionProfilePatch');
    const patch = bridge.slice(start, bridge.indexOf('};', start));
    expect(patch.includes('fallback_model?: ICompanionModelRef | null;')).toBe(true);
    expect(patch.includes('vision_model?: ICompanionModelRef | null;')).toBe(true);
    expect(patch.includes('vad?: Partial<ICompanionVadConfig>;')).toBe(true);
  });

  test('the optimistic merge reaches two levels deep into voice', () => {
    // `voice.vad` is nested one level below `voice`; a single spread would
    // replace the whole vad block and blank the untouched parameter, so the
    // slider would visibly snap the other value back to its default.
    const start = useNomi.indexOf('const mergeProfile');
    const merge = useNomi.slice(start, useNomi.indexOf('});', start));
    expect(merge.includes('patch.fallback_model !== undefined')).toBe(true);
    expect(merge.includes('patch.vision_model !== undefined')).toBe(true);
    expect(merge.includes('vad: { ...prev.voice.vad, ...patch.voice.vad }')).toBe(true);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/nomi/companionVoiceSlots.test.ts`
Expected: FAIL — 三个 test 全红（`expect(false).toBe(true)`）。

- [ ] **Step 3: 写最小实现**

3a. `ui/src/common/adapter/ipcBridge.ts` —— 在 `ICompanionModelRef`（`:4701` 结束）之后插入：

```ts
/** One companion's speech-synthesis选择: catalog model + provider voice id. */
export interface ICompanionTtsSelection {
  provider_id: ProviderId;
  model: string;
  /** Provider voice id (free text); `null` = the provider's own default voice. */
  voice: string | null;
}

/**
 * One companion's voice-activity-detection tuning. The engine runs locally, so
 * there is no Provider reference here — only tuning. `engine` is a string
 * rather than a union because the backend recognises exactly `'silero'` today
 * and falls back to its built-in energy detector for anything else; a union
 * would make a future engine a breaking type change.
 */
export interface ICompanionVadConfig {
  engine: string;
  /** Speech-probability threshold, 0..1. */
  sensitivity: number;
  /** Trailing silence (ms) that closes one utterance, 200..3000. */
  min_silence_ms: number;
}

/** One companion's voice stack. `asr`/`tts` null = use the install-wide default. */
export interface ICompanionVoiceConfig {
  asr: ICompanionModelRef | null;
  tts: ICompanionTtsSelection | null;
  vad: ICompanionVadConfig;
}
```

3b. `ICompanionProfile`（`:4756` 的 `model: ICompanionModelRef | null;` 之后）插入：

```ts
  /** 备用对话模型: replayed once when the main model's turn fails. */
  fallback_model: ICompanionModelRef | null;
  /** 视觉大模型; null = use the main chat model when it can see images. */
  vision_model: ICompanionModelRef | null;
  /** ASR / TTS / VAD for this companion. */
  voice: ICompanionVoiceConfig;
```

3c. `ICompanionProfilePatch`（`:4846` 的 `model?: ICompanionModelRef | null;` 之后）插入：

```ts
  fallback_model?: ICompanionModelRef | null;
  vision_model?: ICompanionModelRef | null;
  voice?: {
    asr?: ICompanionModelRef | null;
    tts?: ICompanionTtsSelection | null;
    vad?: Partial<ICompanionVadConfig>;
  };
```

3d. `ui/src/renderer/pages/nomi/useNomi.ts:19-27` 整体替换：

```ts
/** Optimistic RFC 7396-style merge of a companion-profile patch (client mirror). */
const mergeProfile = (prev: ICompanionProfile, patch: ICompanionProfilePatch): ICompanionProfile => ({
  ...prev,
  ...(patch.name !== undefined ? { name: patch.name } : {}),
  ...(patch.character !== undefined ? { character: patch.character } : {}),
  ...(patch.persona ? { persona: { ...prev.persona, ...patch.persona } } : {}),
  ...(patch.model !== undefined ? { model: patch.model } : {}),
  ...(patch.fallback_model !== undefined ? { fallback_model: patch.fallback_model } : {}),
  ...(patch.vision_model !== undefined ? { vision_model: patch.vision_model } : {}),
  // `voice.vad` sits one level below `voice`: a single spread would replace the
  // whole vad block, so patching 灵敏度 alone would snap 停顿判停 back to its
  // default until the server response landed.
  ...(patch.voice
    ? {
        voice: {
          ...prev.voice,
          ...patch.voice,
          ...(patch.voice.vad ? { vad: { ...prev.voice.vad, ...patch.voice.vad } } : {}),
        },
      }
    : {}),
  ...(patch.skills ? { skills: { ...prev.skills, ...patch.skills } } : {}),
  ...(patch.appearance ? { appearance: { ...prev.appearance, ...patch.appearance } } : {}),
});
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun test --cwd ui src/renderer/pages/nomi/companionVoiceSlots.test.ts`
Expected: PASS
Run: `bun run typecheck`
Expected: PASS（无输出即通过）

- [ ] **Step 5: Commit**

```bash
git add ui/src/common/adapter/ipcBridge.ts \
        ui/src/renderer/pages/nomi/useNomi.ts \
        ui/src/renderer/pages/nomi/companionVoiceSlots.test.ts
git commit -m "feat(companion): mirror the new model slots on the profile wire type"
```

---

### Task 4: `TaskModelSelect` 的纯状态函数

**Files:**
- Create: `ui/src/renderer/components/model/taskModelSelectState.ts`
- Test: `ui/src/renderer/components/model/taskModelSelectState.test.ts`

**Interfaces:**
- Consumes: `TaskModelGroup { provider: IProvider; models: string[] }`（`ui/src/renderer/hooks/agent/useModelsForTask.ts:15-18`）。
- Produces:
  - `export interface TaskModelSelection { provider_id: ProviderId; model: string; voice?: string | null }`
  - `export type TaskModelProviderScope = 'task' | 'all-enabled'`
  - `export interface TaskModelSelectState { providers: IProvider[]; models: string[]; providerStale: boolean; modelStale: boolean; anyModel: boolean; configured: boolean }`
  - `export const taskModelSelectState = (input: { groups: readonly TaskModelGroup[]; enabledProviders: readonly IProvider[]; scope: TaskModelProviderScope; value: TaskModelSelection | null; draftProviderId: ProviderId | null; isLoading: boolean }) => TaskModelSelectState`

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/components/model/taskModelSelectState.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import type { TaskModelGroup } from '@/renderer/hooks/agent/useModelsForTask';
import { taskModelSelectState } from './taskModelSelectState';

const providerId = (suffix: string) => `0190f5fe-7c00-7a00-8000-0000000000${suffix}` as ProviderId;
const A = providerId('a1');
const B = providerId('b2');

const provider = (id: ProviderId, name: string): IProvider =>
  ({ id, name, platform: 'custom', enabled: true }) as unknown as IProvider;

const group = (id: ProviderId, name: string, models: string[]): TaskModelGroup => ({
  provider: provider(id, name),
  models,
});

describe('taskModelSelectState', () => {
  test("task scope lists only providers that can do the task; all-enabled lists them all", () => {
    const groups = [group(A, 'A', ['m1', 'm2'])];
    const enabledProviders = [provider(A, 'A'), provider(B, 'B')];

    const task = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'task',
      value: null,
      draftProviderId: null,
      isLoading: false,
    });
    expect(task.providers.map((p) => p.id)).toEqual([A]);

    const all = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'all-enabled',
      value: null,
      draftProviderId: null,
      isLoading: false,
    });
    expect(all.providers.map((p) => p.id)).toEqual([A, B]);
    // A provider with no task-capable model yields an empty model list rather
    // than disappearing — that is what lets the row explain itself.
    expect(
      taskModelSelectState({
        groups,
        enabledProviders,
        scope: 'all-enabled',
        value: null,
        draftProviderId: B,
        isLoading: false,
      }).models
    ).toEqual([]);
  });

  test('a deleted provider is stale, a vanished model is stale, and both are reported separately', () => {
    const groups = [group(A, 'A', ['m1'])];
    const enabledProviders = [provider(A, 'A')];

    const providerGone = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'all-enabled',
      value: { provider_id: B, model: 'm9' },
      draftProviderId: B,
      isLoading: false,
    });
    expect(providerGone.providerStale).toBe(true);
    expect(providerGone.modelStale).toBe(false);
    expect(providerGone.configured).toBe(false);

    const modelGone = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'all-enabled',
      value: { provider_id: A, model: 'retired' },
      draftProviderId: A,
      isLoading: false,
    });
    expect(modelGone.providerStale).toBe(false);
    expect(modelGone.modelStale).toBe(true);
    expect(modelGone.configured).toBe(false);

    const good = taskModelSelectState({
      groups,
      enabledProviders,
      scope: 'all-enabled',
      value: { provider_id: A, model: 'm1' },
      draftProviderId: A,
      isLoading: false,
    });
    expect(good.configured).toBe(true);
    expect(good.modelStale).toBe(false);
    expect(good.anyModel).toBe(true);
  });

  test('nothing is stale while the catalog is still loading', () => {
    // useModelsForTask keeps isLoading true whenever `data` is not an array, so
    // a failed resolve arrives here as "unknown", not as an empty catalog. If
    // this leaked through as staleness the row would tell the user to re-pick a
    // model that is perfectly fine — and the next click would overwrite a good
    // saved reference.
    const loading = taskModelSelectState({
      groups: [],
      enabledProviders: [],
      scope: 'task',
      value: { provider_id: A, model: 'm1' },
      draftProviderId: A,
      isLoading: true,
    });
    expect(loading.providerStale).toBe(false);
    expect(loading.modelStale).toBe(false);
    expect(loading.anyModel).toBe(false);
    expect(loading.configured).toBe(false);
  });

  test('a draft provider switch does not report the saved model as this provider’s', () => {
    const groups = [group(A, 'A', ['m1']), group(B, 'B', ['m2'])];
    const state = taskModelSelectState({
      groups,
      enabledProviders: [provider(A, 'A'), provider(B, 'B')],
      scope: 'task',
      value: { provider_id: A, model: 'm1' },
      draftProviderId: B,
      isLoading: false,
    });
    // The user picked provider B but has not picked a model yet: the model
    // select must be empty, not showing A's saved model under B.
    expect(state.models).toEqual(['m2']);
    expect(state.modelStale).toBe(false);
    expect(state.configured).toBe(false);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/components/model/taskModelSelectState.test.ts`
Expected: FAIL with `Cannot find module './taskModelSelectState'`

- [ ] **Step 3: 写最小实现**

新建 `ui/src/renderer/components/model/taskModelSelectState.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * The decision logic behind {@link TaskModelSelect}, extracted as a pure
 * function.
 *
 * Eight surfaces in this renderer had each grown their own copy of "group by
 * provider, render a stale reference as a disabled (unavailable) option, and
 * explain an empty catalog" — and they disagreed, which is how a perfectly
 * valid model ended up flagged as retired on one page and not another. This is
 * the one answer; the component only renders it.
 */

import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import type { TaskModelGroup } from '@/renderer/hooks/agent/useModelsForTask';

/** A stored `(provider, model)` reference, plus the voice id for TTS slots. */
export interface TaskModelSelection {
  provider_id: ProviderId;
  model: string;
  voice?: string | null;
}

/**
 * Which providers the first select offers.
 *
 * - `'task'`: only providers that actually own a model for this task. Right for
 *   secondary slots (learning, ASR, TTS…) where an empty provider is noise.
 * - `'all-enabled'`: every enabled provider. Right for the companion's main
 *   chat model, where hiding a provider would render the SAVED provider as a
 *   raw uuid and make the user's own configuration look corrupt.
 */
export type TaskModelProviderScope = 'task' | 'all-enabled';

export interface TaskModelSelectState {
  /** Providers for the first select, in selector order. */
  providers: IProvider[];
  /** Task-capable models of the drafted provider, in catalog order. */
  models: string[];
  /** The saved provider is not in `providers` (deleted or disabled). */
  providerStale: boolean;
  /** The saved provider is fine but the saved model is no longer offered. */
  modelStale: boolean;
  /** The catalog holds at least one model for this task, anywhere. */
  anyModel: boolean;
  /** The saved reference resolves to a live provider AND a live model. */
  configured: boolean;
}

export const taskModelSelectState = ({
  groups,
  enabledProviders,
  scope,
  value,
  draftProviderId,
  isLoading,
}: {
  groups: readonly TaskModelGroup[];
  enabledProviders: readonly IProvider[];
  scope: TaskModelProviderScope;
  value: TaskModelSelection | null;
  draftProviderId: ProviderId | null;
  isLoading: boolean;
}): TaskModelSelectState => {
  const providers = scope === 'task' ? groups.map((g) => g.provider) : [...enabledProviders];
  const currentProvider = providers.find((p) => p.id === draftProviderId);
  const models = currentProvider
    ? (groups.find((g) => g.provider.id === currentProvider.id)?.models ?? [])
    : [];

  // The saved model belongs to the DRAFTED provider only; after a provider
  // switch the model select starts empty instead of showing the old pick.
  const savedModel = value != null && value.provider_id === draftProviderId ? value.model : null;
  const modelValid = savedModel != null && models.includes(savedModel);

  // While the catalog is unresolved (loading, or a failed resolve that
  // useModelsForTask deliberately reports as still-loading) nothing may be
  // called stale: a saved reference is unknown, not wrong.
  const providerStale = !isLoading && draftProviderId != null && currentProvider === undefined;
  const modelStale = !isLoading && savedModel != null && !modelValid;

  return {
    providers,
    models,
    providerStale,
    modelStale,
    anyModel: !isLoading && groups.length > 0,
    configured: !isLoading && !providerStale && modelValid,
  };
};
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun test --cwd ui src/renderer/components/model/taskModelSelectState.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/components/model/taskModelSelectState.ts \
        ui/src/renderer/components/model/taskModelSelectState.test.ts
git commit -m "feat(models): extract the shared task-model selector decision logic"
```

---

### Task 5: `TaskModelSelect` 组件与 `CompanionModelControl` 收敛

**Files:**
- Create: `ui/src/renderer/components/model/TaskModelSelect.tsx`
- Create: `ui/src/renderer/components/model/ttsVoiceOptions.ts`
- Create: `ui/src/renderer/components/model/TaskModelSelect.structure.test.ts`
- Modify: `ui/src/renderer/pages/nomi/CompanionModelControl.tsx`（整文件重写为 `TaskModelSelect` 的薄封装，公开 props 不变）
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/settings.json`、`ui/src/renderer/services/i18n/locales/en-US/settings.json`（新增 `settings.taskModel.*`）

**Interfaces:**
- Consumes: Task 4 的 `taskModelSelectState` / `TaskModelSelection` / `TaskModelProviderScope`；`useModelsForTask(task, requiredTraits?)`；`useProvidersQuery()`；`useModelSelectorProviderLabel()`。
- Produces:
  - `TaskModelSelect` 组件，props：`{ task: ModelTask; traits?: ModelTrait[]; value: TaskModelSelection | null; onChange: (next: TaskModelSelection) => void; scope?: TaskModelProviderScope; withVoice?: boolean; voiceOptions?: readonly string[]; size?: 'mini' | 'small' | 'default'; disabled?: boolean; hideHint?: boolean; emptyHint?: string }`
  - `export const TTS_VOICE_OPTIONS_BY_PLATFORM: Record<string, readonly string[]>` 与 `export const ttsVoiceOptionsFor = (platform: string | undefined): readonly string[]`
  - `CompanionModelControl` props 保持 `{ companion: ReturnType<typeof useCompanion>; showLabel?: boolean }`

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/components/model/TaskModelSelect.structure.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { ttsVoiceOptionsFor, TTS_VOICE_OPTIONS_BY_PLATFORM } from './ttsVoiceOptions';

const src = readFileSync(new URL('./TaskModelSelect.tsx', import.meta.url), 'utf8');
const companionControl = readFileSync(
  new URL('../../pages/nomi/CompanionModelControl.tsx', import.meta.url),
  'utf8'
);

describe('TaskModelSelect', () => {
  test('reads the catalog through the one shared hook and the one shared decision', () => {
    expect(src.includes("useModelsForTask(task, traits)")).toBe(true);
    expect(src.includes('taskModelSelectState({')).toBe(true);
    // No local re-derivation of staleness: that is exactly the drift this
    // component exists to remove.
    expect(src.includes('.includes(selectedModel)')).toBe(false);
  });

  test('a stale provider and a stale model are both rendered as disabled options', () => {
    expect(src.match(/disabled\s*>/g)?.length).toBeGreaterThanOrEqual(2);
    expect(src.includes("t('settings.taskModel.unavailableOption'")).toBe(true);
  });

  test('the voice select is free text with a candidate list, and only for the TTS variant', () => {
    expect(src.includes('withVoice')).toBe(true);
    expect(src.includes('showSearch')).toBe(true);
    expect(src.includes('allowCreate')).toBe(true);
    expect(src.includes("t('settings.taskModel.voicePlaceholder')")).toBe(true);
  });

  test('committing a model keeps the voice already chosen for that provider', () => {
    // Re-picking the model must not silently drop the voice: the user would
    // hear the provider default and have no idea why.
    expect(src.includes('voice: value?.provider_id === providerId ? value.voice : null')).toBe(true);
  });

  test('CompanionModelControl is now a thin wrapper, keeping its all-enabled scope', () => {
    expect(companionControl.includes('<TaskModelSelect')).toBe(true);
    expect(companionControl.includes("scope='all-enabled'")).toBe(true);
    expect(companionControl.includes('patchCompanion({ model:')).toBe(true);
    // Its own duplicated select markup is gone.
    expect(companionControl.includes('NomiSelect.Option')).toBe(false);
  });
});

describe('tts voice candidates', () => {
  test('only platforms whose voice ids are documented get a list', () => {
    expect(ttsVoiceOptionsFor('openai')).toEqual(TTS_VOICE_OPTIONS_BY_PLATFORM.openai);
    expect(ttsVoiceOptionsFor('openai').includes('alloy')).toBe(true);
    // Everything else is free text — inventing ids for a provider we have not
    // verified would offer the user values that just fail at synthesis time.
    expect(ttsVoiceOptionsFor('some-gateway')).toEqual([]);
    expect(ttsVoiceOptionsFor(undefined)).toEqual([]);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/components/model/TaskModelSelect.structure.test.ts`
Expected: FAIL with `Cannot find module './ttsVoiceOptions'`

- [ ] **Step 3: 写最小实现**

3a. 新建 `ui/src/renderer/components/model/ttsVoiceOptions.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Candidate voice ids offered by the TTS variant of {@link TaskModelSelect}.
 *
 * The field is free text on purpose — every provider names its voices
 * differently and new ones ship constantly, so a closed list would go stale and
 * block a voice that works. This table therefore holds ONLY platforms whose
 * voice ids are documented and verified; anything else gets an empty candidate
 * list and the user types the id. Offering guessed ids would be worse than
 * offering none: they look authoritative and fail at synthesis time.
 */
export const TTS_VOICE_OPTIONS_BY_PLATFORM: Record<string, readonly string[]> = {
  openai: ['alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer'],
};

export const ttsVoiceOptionsFor = (platform: string | undefined): readonly string[] =>
  (platform && TTS_VOICE_OPTIONS_BY_PLATFORM[platform]) || [];
```

3b. 新建 `ui/src/renderer/components/model/TaskModelSelect.tsx`：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ModelTask, ModelTrait } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import {
  taskModelSelectState,
  type TaskModelProviderScope,
  type TaskModelSelection,
} from './taskModelSelectState';
import { ttsVoiceOptionsFor } from './ttsVoiceOptions';

export type { TaskModelSelection, TaskModelProviderScope } from './taskModelSelectState';

interface TaskModelSelectProps {
  task: ModelTask;
  /** Extra capability the model must carry (e.g. `['vision_input']`). */
  traits?: ModelTrait[];
  value: TaskModelSelection | null;
  /** Fired only with a complete, live selection. */
  onChange: (next: TaskModelSelection) => void;
  scope?: TaskModelProviderScope;
  /** Render the third (voice) select — the speech-synthesis variant. */
  withVoice?: boolean;
  /** Candidate voice ids; the field stays free text regardless. */
  voiceOptions?: readonly string[];
  size?: 'mini' | 'small' | 'default';
  disabled?: boolean;
  /** Suppress the inline warning line (the caller renders its own copy). */
  hideHint?: boolean;
  /** Copy shown when the catalog has no model for this task at all. */
  emptyHint?: string;
}

/**
 * The shared "pick a model for this task" control: provider + model, plus a
 * voice id for speech synthesis.
 *
 * Membership comes from `useModelsForTask` (→ `POST /api/model-profiles/resolve`),
 * the single authority on which models can do which task — no name heuristics
 * here. Every judgement about the SAVED reference lives in
 * `taskModelSelectState`, so a stale provider and a stale model are rendered as
 * explicit disabled "(unavailable)" options rather than silently blanked: the
 * saved value stays visible and the user is told to re-pick.
 */
const TaskModelSelect: React.FC<TaskModelSelectProps> = ({
  task,
  traits,
  value,
  onChange,
  scope = 'task',
  withVoice = false,
  voiceOptions,
  size = 'mini',
  disabled = false,
  hideHint = false,
  emptyHint,
}) => {
  const { t } = useTranslation();
  const { groups, isLoading } = useModelsForTask(task, traits);
  const { data: rawProviders } = useProvidersQuery();
  const providerLabel = useModelSelectorProviderLabel();
  const [draftProviderId, setDraftProviderId] = useState<ProviderId | null>(null);

  useEffect(() => {
    setDraftProviderId(value?.provider_id ?? null);
  }, [value?.provider_id]);

  const enabledProviders = useMemo(
    () => (rawProviders ?? []).filter((p) => p.enabled !== false),
    [rawProviders]
  );

  const state = taskModelSelectState({
    groups,
    enabledProviders,
    scope,
    value,
    draftProviderId,
    isLoading,
  });

  const providerId = draftProviderId;
  const selectedModel = value?.provider_id === providerId ? value.model : null;
  const currentPlatform = state.providers.find((p) => p.id === providerId)?.platform;
  const voices = voiceOptions ?? ttsVoiceOptionsFor(currentPlatform);
  const selectedVoice = value?.provider_id === providerId ? (value.voice ?? null) : null;

  const hint = !state.anyModel && !isLoading
    ? (emptyHint ?? t('settings.taskModel.emptyHint'))
    : state.providerStale
      ? t('settings.taskModel.staleHint', { model: providerId ?? '' })
      : state.modelStale && selectedModel
        ? t('settings.taskModel.staleHint', { model: selectedModel })
        : '';

  return (
    <div className='flex min-w-0 flex-col items-end gap-4px'>
      <div className='flex min-w-0 flex-wrap items-center justify-end gap-6px'>
        <NomiSelect
          size={size}
          contentFit
          contentMaxWidth={220}
          disabled={disabled}
          placeholder={t('settings.taskModel.providerPlaceholder')}
          value={providerId ?? undefined}
          onChange={(next: ProviderId) => setDraftProviderId(next)}
        >
          {state.providerStale && providerId && (
            <NomiSelect.Option key={providerId} value={providerId} disabled>
              {t('settings.taskModel.unavailableOption', { model: providerId })}
            </NomiSelect.Option>
          )}
          {state.providers.map((p) => (
            <NomiSelect.Option key={p.id} value={p.id}>
              {providerLabel(p)}
            </NomiSelect.Option>
          ))}
        </NomiSelect>
        <NomiSelect
          size={size}
          contentFit
          contentMaxWidth={280}
          disabled={disabled || providerId == null || state.providerStale}
          placeholder={t('settings.taskModel.modelPlaceholder')}
          value={selectedModel ?? undefined}
          onChange={(model: string) => {
            if (!providerId) return;
            onChange({
              provider_id: providerId,
              model,
              // Re-picking the model must keep a voice already chosen for this
              // provider; only a provider switch resets it.
              voice: value?.provider_id === providerId ? value.voice : null,
            });
          }}
        >
          {state.modelStale && selectedModel && (
            <NomiSelect.Option key={selectedModel} value={selectedModel} disabled>
              {t('settings.taskModel.unavailableOption', { model: selectedModel })}
            </NomiSelect.Option>
          )}
          {state.models.map((m) => (
            <NomiSelect.Option key={m} value={m}>
              {m}
            </NomiSelect.Option>
          ))}
        </NomiSelect>
        {withVoice && (
          <NomiSelect
            size={size}
            contentFit
            contentMaxWidth={200}
            showSearch
            allowCreate
            disabled={disabled || selectedModel == null}
            placeholder={t('settings.taskModel.voicePlaceholder')}
            value={selectedVoice ?? undefined}
            onChange={(voice: string) => {
              if (!providerId || !selectedModel) return;
              onChange({ provider_id: providerId, model: selectedModel, voice: voice || null });
            }}
          >
            {voices.map((voice) => (
              <NomiSelect.Option key={voice} value={voice}>
                {voice}
              </NomiSelect.Option>
            ))}
          </NomiSelect>
        )}
      </div>
      {!hideHint && hint && <span className='text-11px leading-tight text-warning-6'>{hint}</span>}
    </div>
  );
};

export default TaskModelSelect;
```

3c. `ui/src/renderer/pages/nomi/CompanionModelControl.tsx` 整文件替换：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Tooltip } from '@arco-design/web-react';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import type { useCompanion } from './useNomi';

interface Props {
  /** 伙伴 profile + 乐观 patch 通道。 */
  companion: ReturnType<typeof useCompanion>;
  /** 总览的“基础配置”行已经渲染标题时，隐藏内联重复标签。 */
  showLabel?: boolean;
}

/**
 * 桌面伙伴对话模型的【唯一】配置入口（紧凑内联，置于「对话」会话头部与总览）。
 *
 * 写入 profile.model —— 全局唯一事实源：本地专属会话与远程连接(IM 机器人)都跟随此模型，
 * 切换后所有会话即时跟随（后端 service.patch_companion 会同步会话行并清空渠道会话）。
 *
 * 供应商下拉用 `scope='all-enabled'`（列出所有已启用供应商，而不只是「含 chat 模型」的
 * 那些）：这样用户始终看得到自己配置的供应商，已存储的当前供应商也能显示名字而不是生
 * provider id；只有图像/视频/嵌入类模型的供应商也可见，其模型下拉为空并给出说明。失效
 * 引用一律由 TaskModelSelect 渲染成禁用的「(不可用)」项。
 */
const CompanionModelControl: React.FC<Props> = ({ companion, showLabel = true }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;
  const configured = Boolean(profile?.model?.provider_id && profile?.model?.model);

  if (!profile) return null;

  return (
    <div className='flex flex-col gap-6px'>
      <div className='flex items-center gap-6px flex-wrap'>
        {showLabel && (
          <Tooltip content={t('nomi.chat.modelConfigHint')}>
            <span className='flex items-center gap-4px text-12px text-t-tertiary shrink-0 cursor-help'>
              <span
                className='w-7px h-7px rd-full shrink-0'
                style={{ background: configured ? 'rgb(var(--success-6))' : 'rgb(var(--warning-6))' }}
              />
              {t('nomi.chat.modelConfig')}
            </span>
          </Tooltip>
        )}
        <TaskModelSelect
          task='chat'
          scope='all-enabled'
          value={profile.model}
          emptyHint={t('nomi.chat.modelNoTextModel')}
          onChange={({ provider_id, model }) => void patchCompanion({ model: { provider_id, model } })}
        />
      </div>
    </div>
  );
};

export default CompanionModelControl;
```

3d. `ui/src/renderer/services/i18n/locales/zh-CN/settings.json` —— 在顶层 `modelHub` 对象**之前**插入新的顶层 `taskModel` 块：

```json
  "taskModel": {
    "providerPlaceholder": "选择服务商",
    "modelPlaceholder": "选择模型",
    "voicePlaceholder": "默认音色",
    "unavailableOption": "{{model}}（不可用）",
    "staleHint": "当前选择的「{{model}}」已不可用，请重新选择。",
    "emptyHint": "没有可用于该用途的模型，请先到「模型管理 → 供应商与密钥」里添加并打标。",
    "voiceFreeTextHint": "音色可直接输入服务商的音色 ID。"
  },
```

3e. `ui/src/renderer/services/i18n/locales/en-US/settings.json` 同位置插入：

```json
  "taskModel": {
    "providerPlaceholder": "Pick a provider",
    "modelPlaceholder": "Pick a model",
    "voicePlaceholder": "Default voice",
    "unavailableOption": "{{model}} (unavailable)",
    "staleHint": "The current choice \"{{model}}\" is no longer available — please pick another.",
    "emptyHint": "No model is available for this purpose yet. Add one under Model Management → Providers & keys and tag it.",
    "voiceFreeTextHint": "You can type the provider's own voice id."
  },
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun run gen:i18n`
Expected: 输出写入的键数，无报错
Run: `bun test --cwd ui src/renderer/components/model/`
Expected: PASS
Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/shell.structure.test.ts src/renderer/pages/nomi/workspace/rulesOfHooks.test.ts`
Expected: PASS
Run: `bun run typecheck && bun run check:i18n`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/components/model/ \
        ui/src/renderer/pages/nomi/CompanionModelControl.tsx \
        ui/src/renderer/services/i18n/locales/zh-CN/settings.json \
        ui/src/renderer/services/i18n/locales/en-US/settings.json \
        ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "feat(models): add the shared TaskModelSelect and rebuild the companion chat model control on it"
```

---

### Task 6: 总览页 `ModelsSection` 重写为五组槽位行

**Files:**
- Modify: `ui/src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.tsx`（整文件重写，删掉「语音与感知」跳转行 `:56-67` 与 `RowAction`/`Right`/`useNavigate` 依赖）
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/nomi.json`（`overview` 块 `:170-187`：删 `voicePerception` / `voicePerceptionHint` / `goModelSettings`，加新键）
- Modify: `ui/src/renderer/services/i18n/locales/en-US/nomi.json`（同上）
- Test: `ui/src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.structure.test.ts`（新建）

**Interfaces:**
- Consumes: Task 3 的 `ICompanionProfile.{fallback_model, vision_model, voice}` + `ICompanionProfilePatch`；Task 5 的 `TaskModelSelect`；`CompanionHandle = ReturnType<typeof useCompanion>`（`workspace/types.ts:28`）。
- Produces: `ModelsSection` props 由 `{ companion, status, companionName }` 改为 `{ companion: CompanionHandle; status: ICompanionStatus; companionName: string }`（不变），行数从 2 变 6（主对话、备用、VAD、ASR、视觉、TTS）。

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.structure.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhNomi from '@/renderer/services/i18n/locales/zh-CN/nomi.json';
import enNomi from '@/renderer/services/i18n/locales/en-US/nomi.json';

const src = readFileSync(new URL('./ModelsSection.tsx', import.meta.url), 'utf8');

const OVERVIEW_SLOT_KEYS = [
  'mainChatModel',
  'fallbackChatModel',
  'fallbackChatModelHint',
  'fallbackChatUnset',
  'vadSlot',
  'vadSlotHint',
  'vadSensitivity',
  'vadMinSilence',
  'asrSlot',
  'asrSlotHint',
  'asrFallback',
  'visionSlot',
  'visionSlotHint',
  'visionFallback',
  'ttsSlot',
  'ttsSlotHint',
  'ttsFallback',
] as const;

describe('总览 model slots', () => {
  test('renders one row per slot in the designed order', () => {
    const order = [
      'nomi.overview.mainChatModel',
      'nomi.overview.fallbackChatModel',
      'nomi.overview.vadSlot',
      'nomi.overview.asrSlot',
      'nomi.overview.visionSlot',
      'nomi.overview.ttsSlot',
    ];
    const positions = order.map((key) => src.indexOf(`'${key}'`));
    expect(positions.every((p) => p > 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
  });

  test('every model slot goes through the shared selector, none re-implements one', () => {
    expect(src.match(/<TaskModelSelect/g)?.length).toBe(4);
    expect(src.includes("task='chat'")).toBe(true);
    expect(src.includes("task='speech_recognition'")).toBe(true);
    expect(src.includes("task='speech_synthesis'")).toBe(true);
    expect(src.includes("traits={['vision_input']}")).toBe(true);
    expect(src.includes('withVoice')).toBe(true);
    expect(src.includes('NomiSelect')).toBe(false);
  });

  test('the app-level "voice & perception" redirect row is gone', () => {
    // The row existed because TTS/ASR/VAD/vision had no per-companion setting.
    // They do now, so a redirect that sends the user away from the控件 would be
    // actively misleading.
    expect(src.includes('voicePerception')).toBe(false);
    expect(src.includes('useNavigate')).toBe(false);
    expect(src.includes('RowAction')).toBe(false);
  });

  test('an unset slot states its fallback instead of looking broken', () => {
    for (const key of ['fallbackChatUnset', 'asrFallback', 'visionFallback', 'ttsFallback']) {
      expect(src.includes(`nomi.overview.${key}`)).toBe(true);
    }
  });

  test('the VAD row is two local parameters, not a model picker', () => {
    expect(src.includes('NomiInputNumber')).toBe(true);
    expect(src.includes("vad: { sensitivity")).toBe(true);
    expect(src.includes("vad: { min_silence_ms")).toBe(true);
    expect(src.includes('min={200}')).toBe(true);
    expect(src.includes('max={3000}')).toBe(true);
  });

  test('copy exists in both locales and the retired keys are deleted', () => {
    const overview = (locale: Record<string, unknown>) =>
      (locale as { overview: Record<string, string> }).overview;
    for (const [name, locale] of [
      ['zh-CN', zhNomi as unknown as Record<string, unknown>],
      ['en-US', enNomi as unknown as Record<string, unknown>],
    ] as const) {
      for (const key of OVERVIEW_SLOT_KEYS) {
        expect(typeof overview(locale)[key]).toBe('string');
        expect(overview(locale)[key].trim().length > 0).toBe(true);
      }
      expect(overview(locale).voicePerception).toBeUndefined();
      expect(overview(locale).voicePerceptionHint).toBeUndefined();
      expect(overview(locale).goModelSettings).toBeUndefined();
      expect(name.length > 0).toBe(true);
    }
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.structure.test.ts`
Expected: FAIL — `renders one row per slot in the designed order` 起全红（`positions.every` 为 false）。

- [ ] **Step 3: 写最小实现**

3a. `ui/src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.tsx` 整文件替换：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { ICompanionStatus } from '@/common/adapter/ipcBridge';
import NomiInputNumber from '@/renderer/components/base/NomiInputNumber';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import CompanionModelControl from '@renderer/pages/nomi/CompanionModelControl';
import type { CompanionHandle } from '../../types';

interface ModelsSectionProps {
  companion: CompanionHandle;
  status: ICompanionStatus;
  companionName: string;
}

/**
 * 模型配置 — the brains, now five kinds of slot instead of one.
 *
 * Every slot except VAD is a catalog reference and therefore renders through the
 * shared `TaskModelSelect`; VAD is local (Silero, no Provider, no credential)
 * and so is two numeric parameters. An unset slot is NOT an error: each row says
 * what it falls back to, which is why the old "voice & perception are app-level,
 * go configure them elsewhere" redirect row is gone — the controls are here now.
 */
const ModelsSection: React.FC<ModelsSectionProps> = ({ companion, status, companionName }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;

  if (!profile) return null;

  const vad = profile.voice.vad;

  return (
    <NomiSettingSection
      title={t('nomi.overview.modelSection')}
      description={t('nomi.overview.modelSectionHint')}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.overview.mainChatModel')}
          description={
            status.model_configured
              ? t('nomi.chat.modelConfigHint')
              : t('nomi.overview.modelMissing', { companionName })
          }
          style={status.model_configured ? undefined : { background: 'rgb(var(--warning-1))' }}
          controls={<CompanionModelControl companion={companion} showLabel={false} />}
        />

        <NomiSettingRow
          title={t('nomi.overview.fallbackChatModel')}
          description={
            profile.fallback_model
              ? t('nomi.overview.fallbackChatModelHint')
              : t('nomi.overview.fallbackChatUnset')
          }
          controls={
            <TaskModelSelect
              task='chat'
              value={profile.fallback_model}
              onChange={({ provider_id, model }) =>
                void patchCompanion({ fallback_model: { provider_id, model } })
              }
            />
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.vadSlot')}
          description={t('nomi.overview.vadSlotHint')}
          controls={
            <>
              <span className='text-12px text-t-tertiary shrink-0'>{t('nomi.overview.vadSensitivity')}</span>
              <NomiInputNumber
                size='mini'
                contentFit
                min={0}
                max={1}
                step={0.05}
                precision={2}
                value={vad.sensitivity}
                onChange={(sensitivity) => {
                  if (typeof sensitivity !== 'number') return;
                  void patchCompanion({ voice: { vad: { sensitivity } } });
                }}
              />
              <span className='text-12px text-t-tertiary shrink-0'>{t('nomi.overview.vadMinSilence')}</span>
              <NomiInputNumber
                size='mini'
                contentFit
                min={200}
                max={3000}
                step={50}
                value={vad.min_silence_ms}
                onChange={(min_silence_ms) => {
                  if (typeof min_silence_ms !== 'number') return;
                  void patchCompanion({ voice: { vad: { min_silence_ms } } });
                }}
              />
            </>
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.asrSlot')}
          description={
            profile.voice.asr ? t('nomi.overview.asrSlotHint') : t('nomi.overview.asrFallback')
          }
          controls={
            <TaskModelSelect
              task='speech_recognition'
              value={profile.voice.asr}
              onChange={({ provider_id, model }) =>
                void patchCompanion({ voice: { asr: { provider_id, model } } })
              }
            />
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.visionSlot')}
          description={
            profile.vision_model
              ? t('nomi.overview.visionSlotHint')
              : t('nomi.overview.visionFallback')
          }
          controls={
            <TaskModelSelect
              task='chat'
              traits={['vision_input']}
              value={profile.vision_model}
              onChange={({ provider_id, model }) =>
                void patchCompanion({ vision_model: { provider_id, model } })
              }
            />
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.ttsSlot')}
          description={
            profile.voice.tts ? t('nomi.overview.ttsSlotHint') : t('nomi.overview.ttsFallback')
          }
          controls={
            <TaskModelSelect
              task='speech_synthesis'
              withVoice
              value={profile.voice.tts}
              onChange={({ provider_id, model, voice }) =>
                void patchCompanion({ voice: { tts: { provider_id, model, voice: voice ?? null } } })
              }
            />
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default ModelsSection;
```

3b. `ui/src/renderer/services/i18n/locales/zh-CN/nomi.json` 的 `overview` 块：**删除** `"voicePerception"`、`"voicePerceptionHint"`、`"goModelSettings"` 三行，**追加**：

```json
    "fallbackChatModel": "备用对话模型",
    "fallbackChatModelHint": "主模型这一轮调用失败时，自动改用它重试一次。",
    "fallbackChatUnset": "未配置。主模型失败就直接报错，不会自动重试。",
    "vadSlot": "语音活动检测",
    "vadSlotHint": "内置 Silero VAD，在本机运行，用来判断你什么时候说完了一句话。",
    "vadSensitivity": "灵敏度",
    "vadMinSilence": "停顿判停（毫秒）",
    "asrSlot": "语音识别",
    "asrSlotHint": "把这只伙伴听到的声音转成文字的模型。",
    "asrFallback": "未配置。使用「模型管理 → 语音」里的全局语音识别模型。",
    "visionSlot": "视觉大模型",
    "visionSlotHint": "看图片、看摄像头画面时单独调用的模型。",
    "visionFallback": "未配置。主对话模型若本身能看图，就用主对话模型。",
    "ttsSlot": "语音合成",
    "ttsSlotHint": "把回复念出来的模型与音色。",
    "ttsFallback": "未配置。使用「模型管理 → 语音」里的全局语音合成模型与音色。"
```

3c. `ui/src/renderer/services/i18n/locales/en-US/nomi.json` 的 `overview` 块：删除同名三行，追加：

```json
    "fallbackChatModel": "Fallback chat model",
    "fallbackChatModelHint": "Replays the turn once on this model when the main one fails.",
    "fallbackChatUnset": "Not set. A failure on the main model is just a failure — no retry.",
    "vadSlot": "Voice activity detection",
    "vadSlotHint": "The built-in Silero VAD, running locally, decides when you have finished speaking.",
    "vadSensitivity": "Sensitivity",
    "vadMinSilence": "End-of-speech pause (ms)",
    "asrSlot": "Speech recognition",
    "asrSlotHint": "The model that turns what this companion hears into text.",
    "asrFallback": "Not set. Uses the global speech-recognition model from Model Management → Voice.",
    "visionSlot": "Vision model",
    "visionSlotHint": "Called on its own to look at images and camera frames.",
    "visionFallback": "Not set. Uses the main chat model when that model can see images.",
    "ttsSlot": "Speech synthesis",
    "ttsSlotHint": "The model and voice that read replies out loud.",
    "ttsFallback": "Not set. Uses the global speech-synthesis model and voice from Model Management → Voice."
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun run gen:i18n`
Expected: 无报错
Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.structure.test.ts`
Expected: PASS
Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/shell.structure.test.ts src/renderer/pages/nomi/workspace/rulesOfHooks.test.ts`
Expected: PASS
Run: `bun run typecheck && bun run check:i18n`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.tsx \
        ui/src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection.structure.test.ts \
        ui/src/renderer/services/i18n/locales/zh-CN/nomi.json \
        ui/src/renderer/services/i18n/locales/en-US/nomi.json \
        ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "feat(companion): rebuild the overview model section as five kinds of slot"
```

---

### Task 7: modelHub 按模态分区骨架（八区）

**Files:**
- Modify: `ui/src/renderer/pages/modelHub/index.tsx`（`Section` 类型 `:23-30`、`sections` `:100-109`、`content` `:111-119`、默认段 `:58-61`）
- Modify: `ui/src/renderer/services/i18n/locales/zh-CN/settings.json`、`.../en-US/settings.json`（`modelHub.section*` 与 `modelHub.provider.*`）
- Modify: `ui/src/renderer/pages/modelHub/modelConfigurationPlacement.test.ts:30-46`
- Test: `ui/src/renderer/pages/modelHub/modelHubSections.test.ts`（新建）

本任务只落**骨架 + 占位宿主**：`ChatModelsContent` / `VisionModelsContent` / `EmbeddingModelsContent` / `SpeechModelsContent` 由 Task 8、Task 10 填内容。为了让本任务自身可跑通，先创建四个仅渲染 `ModalityModelsPanel` 之外壳的**最小**文件；Task 9/10 再替换其内部。

**Interfaces:**
- Produces: `type Section = 'chat' | 'speech' | 'vision' | 'creation' | 'embedding' | 'free' | 'models' | 'global'`，默认 `'chat'`；`?section=` 兼容旧书签（`models`、`speech`、`free`、`creation`、`global` 键名不变，`agents` 继续 302 到执行引擎页）。
- Consumes: 无（纯路由与布局）。

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/pages/modelHub/modelHubSections.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import enSettings from '@/renderer/services/i18n/locales/en-US/settings.json';

const src = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');

const SECTIONS = [
  'chat',
  'speech',
  'vision',
  'creation',
  'embedding',
  'free',
  'models',
  'global',
] as const;

describe('model hub is a modality-first view', () => {
  test('the eight sections exist in the designed order', () => {
    const start = src.indexOf('const sections: SectionDef[]');
    const list = src.slice(start, src.indexOf('[t]', start));
    const keys = [...list.matchAll(/key: '([a-z]+)'/g)].map((m) => m[1]);
    expect(keys).toEqual([...SECTIONS]);
  });

  test('the default section is 对话, not the provider list', () => {
    expect(src.includes("isSection(param) ? param : 'chat'")).toBe(true);
  });

  test('old bookmarks keep working', () => {
    // `models` / `speech` / `free` / `creation` / `global` were the previous keys
    // and must keep resolving; `agents` keeps its redirect.
    for (const legacy of ['models', 'speech', 'free', 'creation', 'global']) {
      expect(src.includes(`value === '${legacy}'`)).toBe(true);
    }
    expect(src.includes("searchParams.get('section') === 'agents'")).toBe(true);
  });

  test('every section has a label in both locales', () => {
    const labelKey = (s: string) => `section${s[0].toUpperCase()}${s.slice(1)}`;
    for (const locale of [zhSettings, enSettings] as unknown as Record<string, never>[]) {
      const hub = (locale as unknown as { modelHub: Record<string, string> }).modelHub;
      for (const section of SECTIONS) {
        expect(typeof hub[labelKey(section)]).toBe('string');
        expect(hub[labelKey(section)].trim().length > 0).toBe(true);
      }
    }
  });

  test('the provider section is renamed to its narrowed job', () => {
    const zhHub = (zhSettings as unknown as { modelHub: Record<string, string> }).modelHub;
    expect(zhHub.sectionModels).toBe('供应商与密钥');
    const zhProvider = (
      zhSettings as unknown as { modelHub: { provider: Record<string, string> } }
    ).modelHub.provider;
    expect(typeof zhProvider.scopeNote).toBe('string');
    const enProvider = (
      enSettings as unknown as { modelHub: { provider: Record<string, string> } }
    ).modelHub.provider;
    expect(typeof enProvider.scopeNote).toBe('string');
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/modelHub/modelHubSections.test.ts`
Expected: FAIL — `expected [ "models", "free", "speech", "creation", "global" ] to equal [ "chat", "speech", … ]`

- [ ] **Step 3: 写最小实现**

3a. 新建四个最小宿主文件。`ui/src/renderer/pages/modelHub/ChatModelsContent.tsx`：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Comment } from '@icon-park/react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** 对话区：chat 任务的模型投影 + 该模态的全局默认对话模型。 */
const ChatModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='chat'
    icon={<Comment theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.modality.chatTitle'
    subtitleKey='settings.modelHub.modality.chatSubtitle'
  />
);

export default ChatModelsContent;
```

`ui/src/renderer/pages/modelHub/VisionModelsContent.tsx`：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { PreviewOpen } from '@icon-park/react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** 视觉区：带 vision_input trait 的 chat 模型投影（视觉不是独立 ModelTask）。 */
const VisionModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='vision'
    icon={<PreviewOpen theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.modality.visionTitle'
    subtitleKey='settings.modelHub.modality.visionSubtitle'
  />
);

export default VisionModelsContent;
```

`ui/src/renderer/pages/modelHub/EmbeddingModelsContent.tsx`：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { SafeRetrieval } from '@icon-park/react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** 嵌入与检索区：embedding + rerank 两个任务的合并投影。 */
const EmbeddingModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='embedding'
    icon={<SafeRetrieval theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.modality.embeddingTitle'
    subtitleKey='settings.modelHub.modality.embeddingSubtitle'
  />
);

export default EmbeddingModelsContent;
```

`ui/src/renderer/pages/modelHub/SpeechModelsContent.tsx`（Task 8 会把 TTS 与 VAD 填进来，此处先只挂 ASR）：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import SpeechToTextContent from './SpeechToTextContent';

/**
 * 语音区宿主：语音识别（ASR）、语音合成（TTS）与语音活动检测（VAD）三块。
 * 每块自己拉自己的配置，本文件只负责纵向排布。
 */
const SpeechModelsContent: React.FC = () => (
  <div className='flex flex-col gap-14px'>
    <SpeechToTextContent />
  </div>
);

export default SpeechModelsContent;
```

3b. `ui/src/renderer/pages/modelHub/index.tsx`：

- `:11` 的图标导入改为
  `import { Comment, HeadsetOne, LinkCloud, SettingTwo, Platte, Lightning, PreviewOpen, SafeRetrieval } from '@icon-park/react';`
- `:21` 的 `import SpeechToTextContent from './SpeechToTextContent';` 改为
  ```tsx
  import SpeechModelsContent from './SpeechModelsContent';
  import ChatModelsContent from './ChatModelsContent';
  import VisionModelsContent from './VisionModelsContent';
  import EmbeddingModelsContent from './EmbeddingModelsContent';
  ```
- `:23-30` 替换：
  ```tsx
  type Section = 'chat' | 'speech' | 'vision' | 'creation' | 'embedding' | 'free' | 'models' | 'global';

  const isSection = (value: string | null): value is Section =>
    value === 'chat' ||
    value === 'speech' ||
    value === 'vision' ||
    value === 'creation' ||
    value === 'embedding' ||
    value === 'free' ||
    value === 'models' ||
    value === 'global';
  ```
- `:59-60` 的默认值替换为 `return isSection(param) ? param : 'chat';`
- `:100-109` 替换：
  ```tsx
  const sections: SectionDef[] = useMemo(
    () => [
      { key: 'chat', label: t('settings.modelHub.sectionChat'), icon: <Comment theme='outline' size='16' strokeWidth={3} /> },
      { key: 'speech', label: t('settings.modelHub.sectionSpeech'), icon: <HeadsetOne theme='outline' size='16' strokeWidth={3} /> },
      { key: 'vision', label: t('settings.modelHub.sectionVision'), icon: <PreviewOpen theme='outline' size='16' strokeWidth={3} /> },
      { key: 'creation', label: t('settings.modelHub.sectionCreation'), icon: <Platte theme='outline' size='16' strokeWidth={3} /> },
      { key: 'embedding', label: t('settings.modelHub.sectionEmbedding'), icon: <SafeRetrieval theme='outline' size='16' strokeWidth={3} /> },
      { key: 'free', label: t('settings.modelHub.sectionFree'), icon: <Lightning theme='outline' size='16' strokeWidth={3} /> },
      { key: 'models', label: t('settings.modelHub.sectionModels'), icon: <LinkCloud theme='outline' size='16' strokeWidth={3} /> },
      { key: 'global', label: t('settings.modelHub.sectionGlobal'), icon: <SettingTwo theme='outline' size='16' strokeWidth={3} /> },
    ],
    [t]
  );
  ```
- `:111-119` 替换：
  ```tsx
  const content = (
    <>
      {section === 'chat' && <ChatModelsContent />}
      {section === 'speech' && <SpeechModelsContent />}
      {section === 'vision' && <VisionModelsContent />}
      {section === 'creation' && <CreationModelsContent />}
      {section === 'embedding' && <EmbeddingModelsContent />}
      {section === 'free' && <FreeModelsContent />}
      {section === 'models' && <ModelModalContent />}
      {section === 'global' && <GlobalModelConfig />}
    </>
  );
  ```

3c. `ui/src/renderer/services/i18n/locales/zh-CN/settings.json` 的 `modelHub`：

- `"subtitle"` 改为 `"按模态挑模型：对话、语音、视觉、创作、嵌入与检索。"`
- `"sectionModels"` 改为 `"供应商与密钥"`
- `"sectionSpeech"` 改为 `"语音"`
- 新增 `"sectionChat": "对话"`、`"sectionVision": "视觉"`、`"sectionEmbedding": "嵌入与检索"`
- `modelHub.provider` 块的 `"title"` 改 `"供应商与密钥"`、`"subtitle"` 改 `"接入厂商、管理凭证与连接档案，并维护模型行与高级覆写。"`，新增
  `"scopeNote": "「按用途找模型」已经搬到左侧的对话 / 语音 / 视觉 / 创作 / 嵌入与检索分区，这里只管接入与凭证。"`

3d. `ui/src/renderer/services/i18n/locales/en-US/settings.json` 的 `modelHub`：

- `"subtitle"`: `"Pick models by modality: chat, voice, vision, creation, embedding & retrieval."`
- `"sectionModels"`: `"Providers & keys"`
- `"sectionSpeech"`: `"Voice"`
- 新增 `"sectionChat": "Chat"`、`"sectionVision": "Vision"`、`"sectionEmbedding": "Embedding & retrieval"`
- `provider.title`: `"Providers & keys"`；`provider.subtitle`: `"Connect vendors, manage credentials and connection profiles, and maintain model rows and advanced overrides."`；新增
  `"scopeNote": "Finding a model by purpose now lives in the Chat / Voice / Vision / Creation / Embedding sections on the left. This page is only about access and credentials."`

3e. `ui/src/renderer/pages/modelHub/modelConfigurationPlacement.test.ts:30-46` 的 `speech-to-text has a dedicated peer section…` 用例替换为：

```ts
  test('speech has a dedicated peer section and old copied cards are removed', () => {
    const hubSource = readSource('./index.tsx');
    const speechHost = readSource('./SpeechModelsContent.tsx');
    const speechSource = readSource('./SpeechToTextContent.tsx');
    const creationSource = readSource('./CreationModelsContent.tsx');
    const providerSource = readSource(
      '../../components/settings/SettingsModal/contents/ModelModalContent.tsx'
    );

    expect(hubSource.includes("key: 'speech'")).toBe(true);
    expect(hubSource.includes('<SpeechModelsContent />')).toBe(true);
    // The 语音 section hosts ASR, TTS and the local VAD entry.
    expect(speechHost.includes('<SpeechToTextContent />')).toBe(true);
    // Candidates come from the authoritative catalog resolve, not provider
    // rows + name guessing.
    expect(speechSource.includes("useModelsForTask('speech_recognition')")).toBe(true);
    expect(speechSource.includes('inferCloudSpeechService')).toBe(false);
    expect(creationSource.includes('ImageGenerationToolSettings')).toBe(false);
    expect(providerSource.includes('SpeechToTextCloudSettings')).toBe(false);
  });
```

3f. 为让本任务独立通过，先建一个最小 `ui/src/renderer/pages/modelHub/ModalityModelsPanel.tsx`（Task 10 替换为完整实现）：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import type { ModalityKey } from './modalityModels';

export interface ModalityModelsPanelProps {
  modality: ModalityKey;
  icon: React.ReactNode;
  titleKey: I18nKey;
  subtitleKey: I18nKey;
}

/** 模态分区通用面板（Task 10 填充完整行渲染）。 */
const ModalityModelsPanel: React.FC<ModalityModelsPanelProps> = ({ icon, titleKey, subtitleKey }) => {
  const { t } = useTranslation();
  return (
    <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
      <header className='flex items-center gap-9px border-b border-b-solid border-[var(--color-border-2)] pb-14px'>
        <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
          {icon}
        </span>
        <div className='min-w-0'>
          <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>{t(titleKey)}</h2>
          <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>{t(subtitleKey)}</p>
        </div>
      </header>
    </div>
  );
};

export default ModalityModelsPanel;
```

以及一个最小 `ui/src/renderer/pages/modelHub/modalityModels.ts`（Task 9 填充）：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** The modality sections that project `provider_models` rows by task/trait. */
export type ModalityKey = 'chat' | 'vision' | 'embedding';
```

并在两个 locale 的 `modelHub` 下新增 `modality` 块（本任务只需四个键，Task 10 补齐其余）：

zh-CN：
```json
   "modality": {
    "chatTitle": "对话模型",
    "chatSubtitle": "能用来对话的模型；新会话的默认模型也在这里设。",
    "visionTitle": "视觉模型",
    "visionSubtitle": "带「看图」能力的对话模型。",
    "embeddingTitle": "嵌入与检索",
    "embeddingSubtitle": "向量嵌入与重排序模型，供知识库检索使用。"
   },
```

en-US：
```json
   "modality": {
    "chatTitle": "Chat models",
    "chatSubtitle": "Models usable for conversation — the default model for new sessions is set here too.",
    "visionTitle": "Vision models",
    "visionSubtitle": "Chat models that can look at images.",
    "embeddingTitle": "Embedding & retrieval",
    "embeddingSubtitle": "Embedding and rerank models used by knowledge-base retrieval."
   },
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun run gen:i18n`
Expected: 无报错
Run: `bun test --cwd ui src/renderer/pages/modelHub/`
Expected: PASS
Run: `bun run typecheck && bun run check:i18n && bun run check:icons`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/pages/modelHub/ \
        ui/src/renderer/services/i18n/locales/zh-CN/settings.json \
        ui/src/renderer/services/i18n/locales/en-US/settings.json \
        ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "feat(modelhub): make the hub a modality-first view with eight sections"
```

---

### Task 8: 语音区（TTS 全局默认 + ASR 目录化收敛 + VAD 条目）

**Files:**
- Create: `ui/src/renderer/services/textToSpeechConfig.ts`
- Create: `ui/src/renderer/services/textToSpeechConfig.test.ts`
- Create: `ui/src/renderer/pages/modelHub/TextToSpeechContent.tsx`
- Modify: `ui/src/renderer/pages/modelHub/SpeechModelsContent.tsx`（挂上 TTS 与 VAD 条目）
- Modify: `ui/src/common/config/configKeys.ts:57`（`tools.speechToText` 之后加 `tools.textToSpeech`）
- Modify: `ui/src/common/types/provider/speech.ts:30-39`（`SpeechToTextConfig` 注明 legacy 块已退役，加 `TextToSpeechConfig`）
- Modify: `ui/src/renderer/services/speechToTextConfig.ts:19-33`（`normalizeSpeechToTextConfig` 剥离内嵌凭证块；新增 `hasLegacyEmbeddedSpeechBlocks`）
- Modify: `ui/src/renderer/pages/modelHub/SpeechToTextContent.tsx`（一次性迁移 + 用 `TaskModelSelect`）
- Test: `ui/src/renderer/services/speechToTextConfig.test.ts`（新建）

**Interfaces:**
- Consumes: Task 2 的偏好键 `tools.textToSpeech`（后端已注册 provider 引用校验）；Task 5 的 `TaskModelSelect`；`configService.get/set`（`ui/src/common/config/configService.ts:126,130`）。
- Produces:
  - `export type TextToSpeechConfig = { provider_id: ProviderId; model: string; voice: string | null }`（`ui/src/common/types/provider/speech.ts`）
  - `TEXT_TO_SPEECH_CONFIG_KEY = 'tools.textToSpeech'`、`TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT`、`getTextToSpeechConfig(): TextToSpeechConfig | undefined`、`saveTextToSpeechConfig(config: TextToSpeechConfig | null): Promise<void>`（`null` 删除该键）
  - `hasLegacyEmbeddedSpeechBlocks(config?: SpeechToTextConfig): boolean`

- [ ] **Step 1: 写失败测试**

1a. 新建 `ui/src/renderer/services/textToSpeechConfig.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { TEXT_TO_SPEECH_CONFIG_KEY } from './textToSpeechConfig';

const src = readFileSync(new URL('./textToSpeechConfig.ts', import.meta.url), 'utf8');
const panel = readFileSync(
  new URL('../pages/modelHub/TextToSpeechContent.tsx', import.meta.url),
  'utf8'
);

describe('tools.textToSpeech client service', () => {
  test('uses the key the backend registered as a Provider reference', () => {
    expect(TEXT_TO_SPEECH_CONFIG_KEY).toBe('tools.textToSpeech');
  });

  test('there is no enabled switch to disagree with the key itself', () => {
    expect(src.includes('enabled')).toBe(false);
  });

  test('clearing the default deletes the key rather than storing a blank object', () => {
    // The backend registers this key as a REQUIRED model reference: an object
    // with an empty provider_id would be rejected at the write boundary, so
    // "no default" has to be expressed as key deletion (null value).
    expect(src.includes('configService.set(TEXT_TO_SPEECH_CONFIG_KEY, undefined)')).toBe(true);
  });

  test('a failed write restores the persisted view instead of lying', () => {
    expect(src.includes('configService.reload()')).toBe(true);
  });

  test('the panel picks the model through the shared TTS selector variant', () => {
    expect(panel.includes('<TaskModelSelect')).toBe(true);
    expect(panel.includes("task='speech_synthesis'")).toBe(true);
    expect(panel.includes('withVoice')).toBe(true);
  });
});
```

1b. 新建 `ui/src/renderer/services/speechToTextConfig.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  hasLegacyEmbeddedSpeechBlocks,
  normalizeSpeechToTextConfig,
} from './speechToTextConfig';

const speechSection = readFileSync(
  new URL('../pages/modelHub/SpeechToTextContent.tsx', import.meta.url),
  'utf8'
);

const providerId = '0190f5fe-7c00-7a00-8000-0000000000a1' as never;

describe('speech-to-text config is a catalog reference now', () => {
  test('a legacy embedded-credential block is detected and stripped', () => {
    const legacy = {
      enabled: true,
      provider: 'openai' as const,
      openai: { api_key: 'sk-legacy', model: 'whisper-1', language: 'zh' },
    };
    expect(hasLegacyEmbeddedSpeechBlocks(legacy)).toBe(true);
    const normalized = normalizeSpeechToTextConfig(legacy);
    // The model/language carried by the retired block are preserved (they are
    // the user's actual choice) but the credential shape is gone — the backend
    // has refused embedded credentials since the catalog migration, so keeping
    // them only risks re-persisting a secret.
    expect(normalized.model).toBe('whisper-1');
    expect(normalized.language).toBe('zh');
    expect(normalized.openai).toBeUndefined();
    expect(normalized.deepgram).toBeUndefined();
    expect(hasLegacyEmbeddedSpeechBlocks(normalized)).toBe(false);
  });

  test('a catalog-shaped config round-trips untouched', () => {
    const current = {
      enabled: true,
      provider: 'openai' as const,
      provider_id: providerId,
      model: 'whisper-1',
      language: '',
    };
    expect(normalizeSpeechToTextConfig(current)).toEqual(current);
    expect(hasLegacyEmbeddedSpeechBlocks(current)).toBe(false);
  });

  test('the section performs the one-time migration and uses the shared selector', () => {
    expect(speechSection.includes('hasLegacyEmbeddedSpeechBlocks')).toBe(true);
    expect(speechSection.includes('<TaskModelSelect')).toBe(true);
    expect(speechSection.includes("task='speech_recognition'")).toBe(true);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/services/textToSpeechConfig.test.ts src/renderer/services/speechToTextConfig.test.ts`
Expected: FAIL with `Cannot find module './textToSpeechConfig'`

- [ ] **Step 3: 写最小实现**

3a. `ui/src/common/types/provider/speech.ts` —— 在 `SpeechToTextConfig`（`:39`）之后追加：

```ts
/**
 * Install-wide speech-synthesis default (`tools.textToSpeech`).
 *
 * Deliberately parallel to {@link SpeechToTextConfig} minus the `enabled`
 * switch: synthesis has no input-box affordance to gate, so the key's presence
 * IS the configuration. `voice` is free text — provider voice ids differ and
 * change often.
 */
export type TextToSpeechConfig = {
  provider_id: ProviderId;
  model: string;
  voice: string | null;
};
```

3b. `ui/src/common/config/configKeys.ts` —— `:2` 的 import 改为
`import type { SpeechToTextConfig, TextToSpeechConfig } from '@/common/types/provider/speech';`
并在 `:57` 之后加：

```ts
  // Install-wide speech-synthesis default. Registered backend-side as a REQUIRED
  // Provider reference (nomifun-db client_preference), so an absent key — not a
  // blank object — is how "no default" is expressed.
  'tools.textToSpeech': TextToSpeechConfig | undefined;
```

3c. 新建 `ui/src/renderer/services/textToSpeechConfig.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import type { TextToSpeechConfig } from '@/common/types/provider/speech';

export const TEXT_TO_SPEECH_CONFIG_KEY = 'tools.textToSpeech' as const;
export const TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT = 'nomifun:text-to-speech-config-changed';

/** The install-wide synthesis default, or `undefined` when none is set. */
export const getTextToSpeechConfig = (): TextToSpeechConfig | undefined =>
  configService.get(TEXT_TO_SPEECH_CONFIG_KEY);

/**
 * Persist (or clear) the install-wide synthesis default.
 *
 * `null` DELETES the key. The backend registers this preference as a required
 * `{provider_id, model}` reference, so a blank object would be refused at the
 * write boundary — absence is the only representation of "no default".
 */
export const saveTextToSpeechConfig = async (config: TextToSpeechConfig | null): Promise<void> => {
  try {
    if (config == null) {
      await configService.set(TEXT_TO_SPEECH_CONFIG_KEY, undefined);
      return;
    }
    await configService.set(TEXT_TO_SPEECH_CONFIG_KEY, config);
  } catch (error) {
    // configService updates its in-memory cache optimistically. Restore the
    // persisted view when the backend rejects the write, so the form does not
    // claim a voice is configured when nothing was saved.
    await configService.reload();
    throw error;
  } finally {
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT));
    }
  }
};
```

3d. `ui/src/renderer/services/speechToTextConfig.ts:19-33` 替换：

```ts
/**
 * True when this stored config still carries a retired embedded-credential
 * block. The backend stopped executing those at the catalog migration
 * (`nomifun-shell` answers a 400 telling the user to re-pick a provider), so
 * they are dead weight that keeps an API key on disk.
 */
export const hasLegacyEmbeddedSpeechBlocks = (config?: SpeechToTextConfig): boolean =>
  Boolean(config?.openai) || Boolean(config?.deepgram);

export const normalizeSpeechToTextConfig = (config?: SpeechToTextConfig): SpeechToTextConfig => {
  if (!config) return DEFAULT_SPEECH_TO_TEXT_CONFIG;

  // Keep the user's actual model/language choice, drop the credential shape.
  // `provider` stays pinned as a legacy wire constant: the Rust
  // `SpeechToTextConfig` still requires it and the backend ignores its value
  // (transcription executes by provider_id + model).
  const { openai, deepgram, ...rest } = config;
  return {
    ...rest,
    provider: config.provider ?? 'openai',
    language: config.language ?? openai?.language ?? deepgram?.language ?? '',
    model: config.model ?? openai?.model ?? deepgram?.model,
  };
};
```

3e. 新建 `ui/src/renderer/pages/modelHub/TextToSpeechContent.tsx`：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Form } from '@arco-design/web-react';
import { Voice } from '@icon-park/react';
import type { TextToSpeechConfig } from '@/common/types/provider/speech';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import {
  getTextToSpeechConfig,
  saveTextToSpeechConfig,
  TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT,
} from '@/renderer/services/textToSpeechConfig';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';

/**
 * TTS 首次获得配置面：全局默认的「语音合成模型 + 音色」。
 *
 * Every companion whose `voice.tts` slot is empty speaks with this. Clearing it
 * deletes the preference key outright (see `saveTextToSpeechConfig`), because
 * the backend registers the key as a required Provider reference and would
 * refuse a half-empty object.
 */
const TextToSpeechContent: React.FC = () => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage({ maxCount: 2 });
  const [config, setConfig] = useState<TextToSpeechConfig | null>(null);

  useEffect(() => {
    const sync = () => setConfig(getTextToSpeechConfig() ?? null);
    sync();
    window.addEventListener(TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT, sync);
    return () => window.removeEventListener(TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT, sync);
  }, []);

  const persist = useCallback(
    (next: TextToSpeechConfig | null) => {
      setConfig(next);
      void saveTextToSpeechConfig(next)
        .then(() => message.success(t('settings.modelHub.speech.ttsSaved')))
        .catch((error: unknown) => {
          setConfig(getTextToSpeechConfig() ?? null);
          message.error(
            error instanceof Error ? error.message : t('settings.modelHub.speech.ttsSaveFailed')
          );
        });
    },
    [message, t]
  );

  return (
    <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
      {messageContext}
      <header className='flex items-center gap-9px border-b border-b-solid border-[var(--color-border-2)] pb-14px'>
        <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
          <Voice theme='outline' size='18' strokeWidth={3} />
        </span>
        <div className='min-w-0'>
          <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>
            {t('settings.modelHub.speech.ttsTitle')}
          </h2>
          <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>
            {t('settings.modelHub.speech.ttsSubtitle')}
          </p>
        </div>
      </header>

      <Form layout='vertical' className='mt-18px'>
        <Form.Item
          label={t('settings.modelHub.speech.ttsSource')}
          extra={t('settings.taskModel.voiceFreeTextHint')}
        >
          <TaskModelSelect
            task='speech_synthesis'
            size='default'
            withVoice
            value={config}
            emptyHint={t('settings.modelHub.speech.ttsNoSources')}
            onChange={({ provider_id, model, voice }) =>
              persist({ provider_id, model, voice: voice ?? null })
            }
          />
        </Form.Item>
      </Form>

      {config && (
        <div className='flex items-center gap-8px'>
          <Button size='small' onClick={() => persist(null)}>
            {t('settings.modelHub.speech.ttsClear')}
          </Button>
        </div>
      )}
    </div>
  );
};

export default TextToSpeechContent;
```

3f. `ui/src/renderer/pages/modelHub/SpeechToTextContent.tsx` —— 把 `cloudOptions`/`sourceOptions`/`selectedSource`/`selectSource`（`:26-100`）与 `Form.Item label=…source` 的 `NomiSelect`（`:134-146`）整块换成 `TaskModelSelect`，并加一次性迁移。改动点：

- 顶部 import：删掉 `NomiSelect`、`useModelsForTask`、`useModelSelectorProviderLabel`、`useNavigate`、`Empty`、`LinkCloud`、`type SpeechToTextProvider`、`SpeechSourceOption`；加
  ```tsx
  import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
  import { hasLegacyEmbeddedSpeechBlocks } from '@/renderer/services/speechToTextConfig';
  ```
- 在 `useEffect` 同步之后追加一次性迁移 effect：
  ```tsx
  // One-time migration: a config still carrying a retired embedded-credential
  // block is rewritten in the catalog shape the moment this page is opened. The
  // backend has refused those blocks since the catalog migration, so leaving
  // them on disk only keeps a dead API key around.
  useEffect(() => {
    const stored = getSpeechToTextConfig();
    if (!hasLegacyEmbeddedSpeechBlocks(configService.get(SPEECH_TO_TEXT_CONFIG_KEY))) return;
    void saveSpeechToTextConfig(stored).catch((error) => {
      console.error('Failed to migrate the legacy speech-to-text config:', error);
    });
  }, []);
  ```
  并补 import `import { configService } from '@/common/config/configService';` 与 `SPEECH_TO_TEXT_CONFIG_KEY`（同 `@/renderer/services/speechToTextConfig`）。
- 选择器替换为：
  ```tsx
            <Form.Item label={t('settings.modelHub.speech.source')}>
              <TaskModelSelect
                task='speech_recognition'
                size='default'
                value={
                  config.provider_id && config.model
                    ? { provider_id: config.provider_id, model: config.model }
                    : null
                }
                emptyHint={t('settings.modelHub.speech.noSources')}
                onChange={({ provider_id, model }) =>
                  persist({ ...config, enabled: true, provider: 'openai', provider_id, model })
                }
              />
            </Form.Item>
  ```
- 把外层 `sourceOptions.length === 0 ? <Empty…> : <>…</>` 的分支删掉，直接渲染 `<Form>` + 开关（空态提示已由 `TaskModelSelect` 的 `emptyHint` 承担），并把标题键换成 `settings.modelHub.speech.asrTitle` / `asrSubtitle`。

3g. `ui/src/renderer/pages/modelHub/SpeechModelsContent.tsx` 替换为：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Radar } from '@icon-park/react';
import SpeechToTextContent from './SpeechToTextContent';
import TextToSpeechContent from './TextToSpeechContent';

/**
 * 语音区宿主：语音识别（ASR）、语音合成（TTS）与语音活动检测（VAD）三块。
 *
 * VAD is not a model picker: the engine is the bundled Silero ONNX graph running
 * locally, and the gateway recognises only `"silero"` (anything else falls back
 * to its energy detector). So this section states the engine and its defaults;
 * the tunable knobs are per companion, on that companion's 总览 page, because a
 * pause length that suits one companion's owner suits nothing else.
 */
const SpeechModelsContent: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col gap-14px'>
      <SpeechToTextContent />
      <TextToSpeechContent />
      <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
        <header className='flex items-center gap-9px pb-4px'>
          <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
            <Radar theme='outline' size='18' strokeWidth={3} />
          </span>
          <div className='min-w-0'>
            <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>
              {t('settings.modelHub.speech.vadTitle')}
            </h2>
            <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>
              {t('settings.modelHub.speech.vadBuiltin')}
            </p>
          </div>
        </header>
        <p className='m-0 mt-8px text-12px leading-18px text-t-secondary'>
          {t('settings.modelHub.speech.vadBuiltinHint', { sensitivity: '0.5', silence: 700 })}
        </p>
      </div>
    </div>
  );
};

export default SpeechModelsContent;
```

（`Radar` 若不在 `@icon-park/react` 中，改用已在本仓库使用过的 `Heartbeat`；`bun run check:icons` 会给出结论。）

3h. locale 追加。`ui/src/renderer/services/i18n/locales/zh-CN/settings.json` 的 `modelHub.speech`：

```json
   "asrTitle": "语音识别（ASR）",
   "asrSubtitle": "把说话内容转成文字的默认模型。",
   "ttsTitle": "语音合成（TTS）",
   "ttsSubtitle": "把文字念出来的默认模型与音色；伙伴没单独配时都用它。",
   "ttsSource": "默认语音合成模型",
   "ttsNoSources": "暂无可用的语音合成模型，请先到「供应商与密钥」里录入语音合成模型。",
   "ttsSaved": "已保存语音合成默认设置",
   "ttsSaveFailed": "保存语音合成默认设置失败",
   "ttsClear": "清除默认",
   "vadTitle": "语音活动检测（VAD）",
   "vadBuiltin": "内置 Silero VAD（本地）",
   "vadBuiltinHint": "在本机运行，不联网、不需要供应商，也没有可选的模型。默认灵敏度 {{sensitivity}}、停顿判停 {{silence}} 毫秒；每只伙伴可以在它的总览页单独调。"
```

`en-US` 同位置：

```json
   "asrTitle": "Speech recognition (ASR)",
   "asrSubtitle": "The default model that turns speech into text.",
   "ttsTitle": "Speech synthesis (TTS)",
   "ttsSubtitle": "The default model and voice for reading text out loud — used by every companion that has no choice of its own.",
   "ttsSource": "Default synthesis model",
   "ttsNoSources": "No speech-synthesis model is available yet. Add one under Providers & keys first.",
   "ttsSaved": "Speech-synthesis default saved",
   "ttsSaveFailed": "Failed to save the speech-synthesis default",
   "ttsClear": "Clear default",
   "vadTitle": "Voice activity detection (VAD)",
   "vadBuiltin": "Built-in Silero VAD (local)",
   "vadBuiltinHint": "Runs on this machine — no network, no provider, and no model to choose. Defaults: sensitivity {{sensitivity}}, end-of-speech pause {{silence}} ms. Each companion can tune both on its own overview page."
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun run gen:i18n`
Expected: 无报错
Run: `bun test --cwd ui src/renderer/services/textToSpeechConfig.test.ts src/renderer/services/speechToTextConfig.test.ts src/renderer/pages/modelHub/`
Expected: PASS
Run: `bun run typecheck && bun run check:i18n && bun run check:icons`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/common/config/configKeys.ts \
        ui/src/common/types/provider/speech.ts \
        ui/src/renderer/services/textToSpeechConfig.ts \
        ui/src/renderer/services/textToSpeechConfig.test.ts \
        ui/src/renderer/services/speechToTextConfig.ts \
        ui/src/renderer/services/speechToTextConfig.test.ts \
        ui/src/renderer/pages/modelHub/TextToSpeechContent.tsx \
        ui/src/renderer/pages/modelHub/SpeechModelsContent.tsx \
        ui/src/renderer/pages/modelHub/SpeechToTextContent.tsx \
        ui/src/renderer/services/i18n/locales/zh-CN/settings.json \
        ui/src/renderer/services/i18n/locales/en-US/settings.json \
        ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "feat(modelhub): give the voice section a TTS default, a catalog-only ASR and the local VAD entry"
```

---

### Task 9: 模态投影纯模块

**Files:**
- Modify: `ui/src/renderer/pages/modelHub/modalityModels.ts`（Task 7 建的最小文件，本任务填满）
- Test: `ui/src/renderer/pages/modelHub/modalityModels.test.ts`（新建）

**Interfaces:**
- Consumes: `ProviderModelResponse`（`ui/src/common/protocolBindings/ProviderModelResponse.ts`，字段 `provider_id, model, enabled, sort_order, tasks, traits, description, source, …`）、`IProvider`（`ui/src/common/config/storage.ts:317`）。
- Produces:
  - `export type ModalityKey = 'chat' | 'vision' | 'embedding'`
  - `export interface ModalitySpec { tasks: readonly ModelTask[]; traits: readonly ModelTrait[] }`
  - `export const MODALITY_SPECS: Record<ModalityKey, ModalitySpec>`
  - `export interface ModalityModelRow { providerId: ProviderId; model: string; enabled: boolean; description: string | null; tasks: ModelTask[]; traits: ModelTrait[] }`
  - `export interface ModalityProviderGroup { providerId: ProviderId; providerName: string; platform: string; models: ModalityModelRow[] }`
  - `export const rowMatchesModality = (row: ProviderModelResponse, spec: ModalitySpec) => boolean`
  - `export const isUntaggedRow = (row: ProviderModelResponse) => boolean`
  - `export const buildModalityGroups = (rows: readonly ProviderModelResponse[], providers: readonly IProvider[], spec: ModalitySpec, providerName?: (p: IProvider) => string) => ModalityProviderGroup[]`
  - `export const buildUntaggedGroups = (rows, providers, providerName?) => ModalityProviderGroup[]`

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/pages/modelHub/modalityModels.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ProviderModelResponse } from '@/common/protocolBindings/ProviderModelResponse';
import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import {
  buildModalityGroups,
  buildUntaggedGroups,
  isUntaggedRow,
  MODALITY_SPECS,
  rowMatchesModality,
} from './modalityModels';

const A = '0190f5fe-7c00-7a00-8000-0000000000a1' as ProviderId;
const B = '0190f5fe-7c00-7a00-8000-0000000000b2' as ProviderId;

const provider = (id: ProviderId, name: string): IProvider =>
  ({ id, name, platform: 'custom', enabled: true }) as unknown as IProvider;

const row = (
  providerId: ProviderId,
  model: string,
  overrides: Partial<ProviderModelResponse> = {}
): ProviderModelResponse =>
  ({
    provider_id: providerId,
    model,
    enabled: true,
    sort_order: 0,
    tasks: ['chat'],
    traits: [],
    params: {},
    source: 'inferred',
    created_at: 0,
    updated_at: 0,
    ...overrides,
  }) as ProviderModelResponse;

describe('modality specs', () => {
  test('vision is a trait-filtered chat projection, not its own task', () => {
    expect(MODALITY_SPECS.vision.tasks).toEqual(['chat']);
    expect(MODALITY_SPECS.vision.traits).toEqual(['vision_input']);
    expect(MODALITY_SPECS.chat.traits).toEqual([]);
    expect(MODALITY_SPECS.embedding.tasks).toEqual(['embedding', 'rerank']);
  });

  test('a row matches when it owns ANY listed task and EVERY listed trait', () => {
    expect(rowMatchesModality(row(A, 'm'), MODALITY_SPECS.chat)).toBe(true);
    expect(rowMatchesModality(row(A, 'm'), MODALITY_SPECS.vision)).toBe(false);
    expect(
      rowMatchesModality(row(A, 'm', { traits: ['vision_input'] }), MODALITY_SPECS.vision)
    ).toBe(true);
    expect(
      rowMatchesModality(row(A, 'e', { tasks: ['rerank'] }), MODALITY_SPECS.embedding)
    ).toBe(true);
    expect(rowMatchesModality(row(A, 'e', { tasks: ['rerank'] }), MODALITY_SPECS.chat)).toBe(false);
  });

  test('disabled rows stay visible so the section can switch them back on', () => {
    // The projection reads `provider_models` rows directly rather than the
    // resolve endpoint precisely for this: resolve only ever returns ENABLED
    // rows, so a toggle built on it could only ever turn things off and then
    // lose sight of them.
    const groups = buildModalityGroups(
      [row(A, 'on'), row(A, 'off', { enabled: false })],
      [provider(A, 'A')],
      MODALITY_SPECS.chat
    );
    expect(groups[0].models.map((m) => m.model)).toEqual(['on', 'off']);
    expect(groups[0].models[1].enabled).toBe(false);
  });

  test('groups follow provider order, models follow sort_order then name', () => {
    const rows = [
      row(B, 'b2', { sort_order: 1 }),
      row(A, 'a2', { sort_order: 2 }),
      row(A, 'a1', { sort_order: 1 }),
      row(B, 'b1', { sort_order: 1 }),
    ];
    const groups = buildModalityGroups(rows, [provider(A, 'A'), provider(B, 'B')], MODALITY_SPECS.chat);
    expect(groups.map((g) => g.providerId)).toEqual([A, B]);
    expect(groups[0].models.map((m) => m.model)).toEqual(['a1', 'a2']);
    expect(groups[1].models.map((m) => m.model)).toEqual(['b1', 'b2']);
  });

  test('rows of an unknown provider are dropped, and an empty provider yields no group', () => {
    const groups = buildModalityGroups(
      [row(B, 'orphan')],
      [provider(A, 'A')],
      MODALITY_SPECS.chat
    );
    expect(groups).toEqual([]);
  });

  test('untagged rows are collected instead of silently vanishing', () => {
    // A legacy row with `tasks: []` matches no modality. Hiding it would make a
    // configured model invisible on every page of the hub; the 对话 section shows
    // it in an explicit "untagged" bucket with a pointer to the tag editor.
    expect(isUntaggedRow(row(A, 'x', { tasks: [] }))).toBe(true);
    expect(isUntaggedRow(row(A, 'x'))).toBe(false);
    const groups = buildUntaggedGroups(
      [row(A, 'tagged'), row(A, 'bare', { tasks: [] })],
      [provider(A, 'A')]
    );
    expect(groups[0].models.map((m) => m.model)).toEqual(['bare']);
  });

  test('a custom provider label is applied (free-model platform renaming)', () => {
    const groups = buildModalityGroups(
      [row(A, 'm')],
      [provider(A, 'raw')],
      MODALITY_SPECS.chat,
      () => '免费模型'
    );
    expect(groups[0].providerName).toBe('免费模型');
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/modelHub/modalityModels.test.ts`
Expected: FAIL with `export named 'MODALITY_SPECS' not found in module`

- [ ] **Step 3: 写最小实现**

`ui/src/renderer/pages/modelHub/modalityModels.ts` 整文件替换：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * The modality-first projection behind the hub's 对话 / 视觉 / 嵌入与检索 sections.
 *
 * The source of truth is the `provider_models` catalog ROWS
 * (`GET /api/provider-models`), not `POST /api/model-profiles/resolve`: resolve
 * answers "which models may a selector offer", so it returns enabled rows only.
 * A management view has to show a DISABLED row too — otherwise its own toggle
 * could only turn models off and then lose them. The backend data model is
 * untouched: this is a filter over `tasks` / `traits`, nothing more.
 *
 * 视觉 is deliberately a trait-filtered `chat` projection rather than its own
 * `ModelTask`, because that is what the backend vocabulary says
 * (`ModelTrait::VisionInput` modifies `ModelTask::Chat`).
 */

import type { IProvider } from '@/common/config/storage';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ModelTrait } from '@/common/protocolBindings/ModelTrait';
import type { ProviderModelResponse } from '@/common/protocolBindings/ProviderModelResponse';
import type { ProviderId } from '@/common/types/ids';

/** The modality sections that project `provider_models` rows by task/trait. */
export type ModalityKey = 'chat' | 'vision' | 'embedding';

export interface ModalitySpec {
  /** Membership needs ANY of these tasks. */
  tasks: readonly ModelTask[];
  /** …and EVERY one of these traits. */
  traits: readonly ModelTrait[];
}

export const MODALITY_SPECS: Record<ModalityKey, ModalitySpec> = {
  chat: { tasks: ['chat'], traits: [] },
  vision: { tasks: ['chat'], traits: ['vision_input'] },
  embedding: { tasks: ['embedding', 'rerank'], traits: [] },
};

export interface ModalityModelRow {
  providerId: ProviderId;
  model: string;
  enabled: boolean;
  description: string | null;
  tasks: ModelTask[];
  traits: ModelTrait[];
}

export interface ModalityProviderGroup {
  providerId: ProviderId;
  providerName: string;
  platform: string;
  models: ModalityModelRow[];
}

export const rowMatchesModality = (row: ProviderModelResponse, spec: ModalitySpec): boolean =>
  spec.tasks.some((task) => row.tasks.includes(task)) &&
  spec.traits.every((trait) => row.traits.includes(trait));

/**
 * A row carrying no task at all. The backend seeds `tasks` when a row is
 * created, so this is legacy/hand-edited data — it belongs to no modality and
 * would otherwise be invisible everywhere in the hub.
 */
export const isUntaggedRow = (row: ProviderModelResponse): boolean => row.tasks.length === 0;

const groupRows = (
  rows: readonly ProviderModelResponse[],
  providers: readonly IProvider[],
  providerName: (provider: IProvider) => string
): ModalityProviderGroup[] => {
  const byProvider = new Map<string, ModalityModelRow[]>();
  for (const row of rows) {
    const list = byProvider.get(row.provider_id) ?? [];
    list.push({
      providerId: row.provider_id as ProviderId,
      model: row.model,
      enabled: row.enabled,
      description: row.description ?? null,
      tasks: [...row.tasks],
      traits: [...row.traits],
    });
    byProvider.set(row.provider_id, list);
  }

  const groups: ModalityProviderGroup[] = [];
  // Provider order is the selector ordering authority (free-model platform
  // first); rows inside a provider follow the catalog `sort_order`, with the
  // model name as the tie-break so the list never reshuffles between renders.
  for (const provider of providers) {
    const models = byProvider.get(provider.id);
    if (!models || models.length === 0) continue;
    const orderOf = new Map(
      rows
        .filter((row) => row.provider_id === provider.id)
        .map((row) => [row.model, row.sort_order] as const)
    );
    models.sort((a, b) => {
      const delta = (orderOf.get(a.model) ?? 0) - (orderOf.get(b.model) ?? 0);
      return delta !== 0 ? delta : a.model.localeCompare(b.model);
    });
    groups.push({
      providerId: provider.id,
      providerName: providerName(provider),
      platform: provider.platform,
      models,
    });
  }
  return groups;
};

export const buildModalityGroups = (
  rows: readonly ProviderModelResponse[],
  providers: readonly IProvider[],
  spec: ModalitySpec,
  providerName: (provider: IProvider) => string = (provider) => provider.name
): ModalityProviderGroup[] =>
  groupRows(
    rows.filter((row) => rowMatchesModality(row, spec)),
    providers,
    providerName
  );

export const buildUntaggedGroups = (
  rows: readonly ProviderModelResponse[],
  providers: readonly IProvider[],
  providerName: (provider: IProvider) => string = (provider) => provider.name
): ModalityProviderGroup[] => groupRows(rows.filter(isUntaggedRow), providers, providerName);
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun test --cwd ui src/renderer/pages/modelHub/modalityModels.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/pages/modelHub/modalityModels.ts \
        ui/src/renderer/pages/modelHub/modalityModels.test.ts
git commit -m "feat(modelhub): project provider_models rows into modality groups"
```

---

### Task 10: 模态分区面板与对话 / 视觉 / 嵌入三区

**Files:**
- Modify: `ui/src/renderer/pages/modelHub/ModalityModelsPanel.tsx`（Task 7 建的最小外壳，本任务填满）
- Modify: `ui/src/renderer/pages/modelHub/ChatModelsContent.tsx`（补 `showDefaultModel` / `showUntagged`）
- Modify: `ui/src/renderer/services/i18n/locales/{zh-CN,en-US}/settings.json`（`modelHub.modality.*` 补齐）
- Test: `ui/src/renderer/pages/modelHub/ModalityModelsPanel.structure.test.ts`（新建）

**Interfaces:**
- Consumes: Task 9 的 `MODALITY_SPECS` / `buildModalityGroups` / `buildUntaggedGroups` / `ModalityKey`；Task 5 的 `TaskModelSelect`；`ipcBridge.providerModel.list({})` → `ProviderModelResponse[]`、`ipcBridge.providerModel.update({ provider_id, model, enabled? , description? })` → `ProviderModelResponse`；`useProvidersQuery()`；`configService`（`nomi.defaultModel`）。
- Produces: `ModalityModelsPanelProps { modality: ModalityKey; icon: React.ReactNode; titleKey: I18nKey; subtitleKey: I18nKey; showDefaultModel?: boolean; showUntagged?: boolean }`。

**范围边界（不得扩大）：** 面板提供「启停」与「改描述」两种行内编辑；**任务归属打标（tasks/traits）不在这里做**，一律跳到「供应商与密钥」区的 `ModelModalityEditor`。理由：那是打标的唯一权威编辑器，再造一个就多一条双写路径（正是 §7 已记录的 P3 债务形态）。

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/pages/modelHub/ModalityModelsPanel.structure.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import enSettings from '@/renderer/services/i18n/locales/en-US/settings.json';

const panel = readFileSync(new URL('./ModalityModelsPanel.tsx', import.meta.url), 'utf8');
const chat = readFileSync(new URL('./ChatModelsContent.tsx', import.meta.url), 'utf8');
const vision = readFileSync(new URL('./VisionModelsContent.tsx', import.meta.url), 'utf8');
const embedding = readFileSync(new URL('./EmbeddingModelsContent.tsx', import.meta.url), 'utf8');

const MODALITY_KEYS = [
  'chatTitle',
  'chatSubtitle',
  'visionTitle',
  'visionSubtitle',
  'embeddingTitle',
  'embeddingSubtitle',
  'modelCount',
  'empty',
  'emptyHint',
  'manageModels',
  'toggleFailed',
  'descriptionPlaceholder',
  'descriptionSave',
  'descriptionFailed',
  'defaultRow',
  'chatDefaultHint',
  'noDefault',
  'untaggedTitle',
  'untaggedHint',
  'taskChat',
  'taskEmbedding',
  'taskRerank',
  'traitVision',
] as const;

describe('modality panel', () => {
  test('lists catalog ROWS so a disabled model is still reachable', () => {
    expect(panel.includes('providerModel.list')).toBe(true);
    expect(panel.includes('buildModalityGroups')).toBe(true);
    // Resolve is for selectors, not for a management list.
    expect(panel.includes('useModelsForTask')).toBe(false);
  });

  test('a row can be switched on and off in place', () => {
    expect(panel.includes('providerModel.update')).toBe(true);
    expect(panel.includes('<Switch')).toBe(true);
    expect(panel.includes("t('settings.modelHub.modality.toggleFailed')")).toBe(true);
  });

  test('task tagging is NOT re-implemented here; it links to the one editor', () => {
    // Duplicating the tasks/traits editor would create a second write path for
    // the same row — the exact double-write shape this repo already carries as
    // known debt.
    expect(panel.includes('ModelModalityEditor')).toBe(false);
    expect(panel.includes("navigate('/models?section=models')")).toBe(true);
  });

  test('only 对话 carries a modality default, and it writes the existing key', () => {
    expect(chat.includes('showDefaultModel')).toBe(true);
    expect(vision.includes('showDefaultModel')).toBe(false);
    expect(embedding.includes('showDefaultModel')).toBe(false);
    expect(panel.includes("'nomi.defaultModel'")).toBe(true);
    expect(panel.includes('<TaskModelSelect')).toBe(true);
    expect(panel.includes("t('settings.modelHub.modality.noDefault')")).toBe(true);
  });

  test('untagged rows surface in the chat section only', () => {
    expect(chat.includes('showUntagged')).toBe(true);
    expect(panel.includes('buildUntaggedGroups')).toBe(true);
  });

  test('copy exists in both locales', () => {
    for (const locale of [zhSettings, enSettings]) {
      const modality = (locale as unknown as { modelHub: { modality: Record<string, string> } })
        .modelHub.modality;
      for (const key of MODALITY_KEYS) {
        expect(typeof modality[key]).toBe('string');
        expect(modality[key].trim().length > 0).toBe(true);
      }
    }
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/modelHub/ModalityModelsPanel.structure.test.ts`
Expected: FAIL — `lists catalog ROWS so a disabled model is still reachable` 起全红。

- [ ] **Step 3: 写最小实现**

3a. `ui/src/renderer/pages/modelHub/ModalityModelsPanel.tsx` 整文件替换：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import useSWR from 'swr';
import { Button, Input, Popover, Switch, Tag } from '@arco-design/web-react';
import { Edit, LinkCloud } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import type { ProviderId } from '@/common/types/ids';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import {
  buildModalityGroups,
  buildUntaggedGroups,
  MODALITY_SPECS,
  type ModalityKey,
  type ModalityModelRow,
  type ModalityProviderGroup,
} from './modalityModels';

export interface ModalityModelsPanelProps {
  modality: ModalityKey;
  icon: React.ReactNode;
  titleKey: I18nKey;
  subtitleKey: I18nKey;
  /** Render the modality's install-wide default model row. */
  showDefaultModel?: boolean;
  /** Append the "no task tag yet" bucket (the chat section owns it). */
  showUntagged?: boolean;
}

const CATALOG_ROWS_SWR_KEY = 'provider-models.all';

const TASK_LABEL_KEY: Record<string, I18nKey> = {
  chat: 'settings.modelHub.modality.taskChat',
  embedding: 'settings.modelHub.modality.taskEmbedding',
  rerank: 'settings.modelHub.modality.taskRerank',
};

/**
 * One modality section of the model hub: the catalog rows that belong to this
 * modality, grouped by provider, each switchable on/off and describable in place.
 *
 * Task TAGGING is not here on purpose — `ModelModalityEditor` on the 供应商与密钥
 * page is the single editor for `tasks`/`traits`, and a second one would be a
 * second write path for the same row. This panel links there instead.
 */
const ModalityModelsPanel: React.FC<ModalityModelsPanelProps> = ({
  modality,
  icon,
  titleKey,
  subtitleKey,
  showDefaultModel = false,
  showUntagged = false,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [message, messageContext] = useArcoMessage({ maxCount: 2 });
  const { data: providers } = useProvidersQuery();
  const providerLabel = useModelSelectorProviderLabel();
  const { data: rows, mutate } = useSWR(CATALOG_ROWS_SWR_KEY, () =>
    ipcBridge.providerModel.list.invoke({})
  );
  const [defaultModel, setDefaultModel] = useState(() => configService.get('nomi.defaultModel') ?? null);
  const [draftDescription, setDraftDescription] = useState('');

  const enabledProviders = useMemo(
    () => (providers ?? []).filter((p) => p.enabled !== false),
    [providers]
  );

  const groups = useMemo(
    () => buildModalityGroups(rows ?? [], enabledProviders, MODALITY_SPECS[modality], providerLabel),
    [rows, enabledProviders, modality, providerLabel]
  );

  const untagged = useMemo(
    () => (showUntagged ? buildUntaggedGroups(rows ?? [], enabledProviders, providerLabel) : []),
    [showUntagged, rows, enabledProviders, providerLabel]
  );

  const toggleRow = useCallback(
    async (row: ModalityModelRow, enabled: boolean) => {
      try {
        await ipcBridge.providerModel.update.invoke({
          provider_id: row.providerId,
          model: row.model,
          enabled,
        });
        await mutate();
      } catch (error) {
        console.error('[ModalityModels] Failed to toggle a catalog row:', error);
        message.error(t('settings.modelHub.modality.toggleFailed'));
      }
    },
    [message, mutate, t]
  );

  const saveDescription = useCallback(
    async (row: ModalityModelRow, description: string) => {
      try {
        await ipcBridge.providerModel.update.invoke({
          provider_id: row.providerId,
          model: row.model,
          description: description.trim() || null,
        });
        await mutate();
      } catch (error) {
        console.error('[ModalityModels] Failed to save a model description:', error);
        message.error(t('settings.modelHub.modality.descriptionFailed'));
      }
    },
    [message, mutate, t]
  );

  const persistDefault = useCallback((provider_id: ProviderId, model: string) => {
    const next = { provider_id, model };
    setDefaultModel(next);
    void configService.set('nomi.defaultModel', next).catch(async (error: unknown) => {
      await configService.reload();
      setDefaultModel(configService.get('nomi.defaultModel') ?? null);
      console.error('[ModalityModels] Failed to save the default chat model:', error);
    });
  }, []);

  const renderGroup = (group: ModalityProviderGroup) => (
    <div key={group.providerId} className='flex flex-col gap-6px'>
      <div className='flex min-w-0 items-center gap-8px flex-wrap'>
        <span className='text-14px font-600 text-t-primary truncate'>{group.providerName}</span>
        <span className='text-11px text-t-tertiary shrink-0'>{group.platform}</span>
        <span className='text-11px text-t-tertiary shrink-0'>
          · {t('settings.modelHub.modality.modelCount', { count: group.models.length })}
        </span>
      </div>
      <NomiSettingList>
        {group.models.map((row) => (
          <NomiSettingRow
            key={row.model}
            title={
              <div className='flex min-w-0 items-center gap-6px flex-wrap'>
                <span className='truncate'>{row.model}</span>
                {row.tasks
                  .filter((task) => TASK_LABEL_KEY[task])
                  .map((task) => (
                    <Tag key={task} size='small' color='arcoblue'>
                      {t(TASK_LABEL_KEY[task])}
                    </Tag>
                  ))}
                {row.traits.includes('vision_input') && (
                  <Tag size='small' color='purple'>
                    {t('settings.modelHub.modality.traitVision')}
                  </Tag>
                )}
              </div>
            }
            description={row.description ?? undefined}
            controls={
              <>
                <Switch
                  size='small'
                  className='compact-dark-switch shrink-0'
                  checked={row.enabled}
                  onChange={(enabled: boolean) => void toggleRow(row, enabled)}
                />
                <Popover
                  trigger='click'
                  onVisibleChange={(visible) => {
                    if (visible) setDraftDescription(row.description ?? '');
                  }}
                  content={
                    <div className='flex w-260px flex-col gap-8px'>
                      <Input.TextArea
                        autoSize={{ minRows: 2, maxRows: 5 }}
                        value={draftDescription}
                        placeholder={t('settings.modelHub.modality.descriptionPlaceholder')}
                        onChange={setDraftDescription}
                      />
                      <Button
                        size='mini'
                        type='primary'
                        onClick={() => void saveDescription(row, draftDescription)}
                      >
                        {t('settings.modelHub.modality.descriptionSave')}
                      </Button>
                    </div>
                  }
                >
                  <Button size='mini' icon={<Edit theme='outline' size='12' strokeWidth={3} />} />
                </Popover>
              </>
            }
          />
        ))}
      </NomiSettingList>
    </div>
  );

  return (
    <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
      {messageContext}
      <header className='flex items-center gap-9px border-b border-b-solid border-[var(--color-border-2)] pb-14px'>
        <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
          {icon}
        </span>
        <div className='min-w-0'>
          <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>{t(titleKey)}</h2>
          <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>{t(subtitleKey)}</p>
        </div>
      </header>

      <div className='mt-14px'>
        <NomiSettingList>
          <NomiSettingRow
            title={t('settings.modelHub.modality.defaultRow')}
            description={
              showDefaultModel
                ? t('settings.modelHub.modality.chatDefaultHint')
                : t('settings.modelHub.modality.noDefault')
            }
            controls={
              showDefaultModel ? (
                <TaskModelSelect
                  task='chat'
                  size='small'
                  value={defaultModel}
                  onChange={({ provider_id, model }) => persistDefault(provider_id, model)}
                />
              ) : undefined
            }
          />
        </NomiSettingList>
      </div>

      <NomiScrollArea className='mt-14px flex-1 min-h-0' disableOverflow>
        {groups.length === 0 ? (
          <div className='flex flex-col items-center justify-center py-42px text-center'>
            <h3 className='m-0 text-16px font-500 text-t-primary'>
              {t('settings.modelHub.modality.empty')}
            </h3>
            <p className='mt-6px max-w-420px text-13px leading-20px text-t-secondary'>
              {t('settings.modelHub.modality.emptyHint')}
            </p>
          </div>
        ) : (
          <div className='flex flex-col gap-14px'>{groups.map(renderGroup)}</div>
        )}

        {untagged.length > 0 && (
          <div className='mt-18px flex flex-col gap-8px'>
            <div className='text-14px font-600 text-t-primary'>
              {t('settings.modelHub.modality.untaggedTitle')}
            </div>
            <div className='text-12px leading-18px text-t-secondary'>
              {t('settings.modelHub.modality.untaggedHint')}
            </div>
            {untagged.map(renderGroup)}
          </div>
        )}
      </NomiScrollArea>

      <div className='mt-12px flex items-center gap-8px flex-wrap'>
        <Button
          type='text'
          size='small'
          icon={<LinkCloud theme='outline' size='14' />}
          onClick={() => navigate('/models?section=models')}
        >
          {t('settings.modelHub.modality.manageModels')}
        </Button>
      </div>
    </div>
  );
};

export default ModalityModelsPanel;
```

3b. `ui/src/renderer/pages/modelHub/ChatModelsContent.tsx` 的 `<ModalityModelsPanel …>` 加两个 prop：

```tsx
  <ModalityModelsPanel
    modality='chat'
    icon={<Comment theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.modality.chatTitle'
    subtitleKey='settings.modelHub.modality.chatSubtitle'
    showDefaultModel
    showUntagged
  />
```

3c. `zh-CN/settings.json` 的 `modelHub.modality` 补齐（保留 Task 7 已加的六个 title/subtitle）：

```json
    "modelCount": "{{count}} 个模型",
    "empty": "这一类暂时没有模型",
    "emptyHint": "到「供应商与密钥」里添加模型，并给它打上对应的任务标签。",
    "manageModels": "去供应商与密钥",
    "toggleFailed": "保存模型开关失败",
    "descriptionPlaceholder": "给这个模型写一句说明，会显示在选择器里",
    "descriptionSave": "保存说明",
    "descriptionFailed": "保存说明失败",
    "defaultRow": "本类默认模型",
    "chatDefaultHint": "新建会话时默认选中的对话模型。",
    "noDefault": "这一类没有全局默认，由用到它的功能各自选择。",
    "untaggedTitle": "还没有任务标签的模型",
    "untaggedHint": "这些模型没有任何任务标签，任何分区和选择器都不会列出它们。到「供应商与密钥」里给它们打标即可。",
    "taskChat": "对话",
    "taskEmbedding": "嵌入",
    "taskRerank": "重排序",
    "traitVision": "看图"
```

3d. `en-US/settings.json` 的 `modelHub.modality` 补齐：

```json
    "modelCount": "{{count}} model(s)",
    "empty": "No model in this category yet",
    "emptyHint": "Add models under Providers & keys and tag them with the matching task.",
    "manageModels": "Open Providers & keys",
    "toggleFailed": "Failed to save the model switch",
    "descriptionPlaceholder": "One line about this model — shown in selectors",
    "descriptionSave": "Save description",
    "descriptionFailed": "Failed to save the description",
    "defaultRow": "Default model for this category",
    "chatDefaultHint": "The chat model preselected for new sessions.",
    "noDefault": "This category has no global default — whatever uses it picks its own.",
    "untaggedTitle": "Models with no task tag",
    "untaggedHint": "These models carry no task tag, so no section and no selector lists them. Tag them under Providers & keys.",
    "taskChat": "Chat",
    "taskEmbedding": "Embedding",
    "taskRerank": "Rerank",
    "traitVision": "Vision"
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun run gen:i18n`
Expected: 无报错
Run: `bun test --cwd ui src/renderer/pages/modelHub/`
Expected: PASS
Run: `bun run typecheck && bun run check:i18n && bun run check:icons && bun run check:theme`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/pages/modelHub/ModalityModelsPanel.tsx \
        ui/src/renderer/pages/modelHub/ModalityModelsPanel.structure.test.ts \
        ui/src/renderer/pages/modelHub/ChatModelsContent.tsx \
        ui/src/renderer/services/i18n/locales/zh-CN/settings.json \
        ui/src/renderer/services/i18n/locales/en-US/settings.json \
        ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "feat(modelhub): build the chat, vision and embedding modality sections"
```

---

### Task 11: 「供应商与密钥」区职责收窄

**Files:**
- Modify: `ui/src/renderer/components/settings/SettingsModal/contents/ModelModalContent.tsx`（header 区 `:905-945`：在既有隐私提示条之后加一条职责说明）
- Test: `ui/src/renderer/pages/modelHub/providerSectionScope.test.ts`（新建）

**Interfaces:**
- Consumes: Task 7 已改好的 `settings.modelHub.provider.{title,subtitle,scopeNote}` 文案。
- Produces: 无新导出；只有可测的结构事实（说明条存在、页面不再自称「按用途找模型」）。

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/pages/modelHub/providerSectionScope.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const provider = readFileSync(
  new URL('../../components/settings/SettingsModal/contents/ModelModalContent.tsx', import.meta.url),
  'utf8'
);

describe('providers & keys section scope', () => {
  test('states where "find a model by purpose" moved to', () => {
    // Without this line the page still reads as the place to shop for a model,
    // and users keep hunting for the voice/vision settings inside a provider card.
    expect(provider.includes("t('settings.modelHub.provider.scopeNote')")).toBe(true);
  });

  test('keeps its actual job: the two-level provider/model list and the credential editors', () => {
    expect(provider.includes('SortableProviderCard')).toBe(true);
    expect(provider.includes('SortableModelRow')).toBe(true);
    expect(provider.includes('ApiKeyEditorModal')).toBe(true);
    expect(provider.includes('ProviderConnectionsSection')).toBe(true);
    // The tasks/traits editor stays HERE and nowhere else.
    expect(provider.includes('ModelModalityEditor')).toBe(true);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/modelHub/providerSectionScope.test.ts`
Expected: FAIL with `expected false to be true`（`scopeNote` 尚未被引用）

- [ ] **Step 3: 写最小实现**

`ui/src/renderer/components/settings/SettingsModal/contents/ModelModalContent.tsx` —— 在渲染 `settings.modelHub.provider.note` 的那个提示块（`:942` 附近）之后，紧邻插入：

```tsx
            {/* 职责收窄：本页只管接入与凭证；「按用途找模型」已迁到模态分区。 */}
            <div className='mt-8px text-12px leading-18px text-t-tertiary'>
              {t('settings.modelHub.provider.scopeNote')}
            </div>
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun test --cwd ui src/renderer/pages/modelHub/providerSectionScope.test.ts`
Expected: PASS
Run: `bun run typecheck && bun run check:i18n`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/components/settings/SettingsModal/contents/ModelModalContent.tsx \
        ui/src/renderer/pages/modelHub/providerSectionScope.test.ts
git commit -m "docs(modelhub): narrow the providers section to access and credentials"
```

---

### Task 12: 机器人 DTO / API / 事件契约（ipcBridge）

**Files:**
- Modify: `ui/src/common/adapter/ipcBridge.ts`（在 SSH 块结束、即 `export const ssh = { … };` 之后 `// Database` 注释块 `:2027` 之前插入机器人块）
- Modify: `ui/src/renderer/components/capability/capabilityStatusColors.ts`（末尾加 `ROBOT_STATUS_COLOR`）
- Create: `ui/src/common/adapter/ipcBridge.robot-status-wire.test.ts`

**Interfaces:**
- Consumes: Plan A 的六条 REST 与 `robot.status` 事件（本任务不实现后端）；既有 `httpGet` / `httpPost` / `httpPatch` / `httpDelete` / `withResponseMap` / `wsMappedEmitter`（`ui/src/common/adapter/httpBridge.ts:681-745, 1173`）、`parseCompanionId`（`ipcBridge.ts:179`）。
- Produces（后续任务逐字消费）：
  - `export type IApiRobotPhase = 'offline' | 'idle' | 'listening' | 'speaking'`
  - `export interface IApiRobot { robot_id: string; name: string; companion_id: CompanionId | null; board: string; firmware_version: string; last_seen: string | null; created_at: string }`
  - `export interface IApiRobotStatus { robot_id: string; companion_id: CompanionId | null; phase: IApiRobotPhase; changed_at: number }`
  - `export interface IApiRobotEndpoints { ota_urls: string[]; lan_enabled: boolean }`
  - `export const robot = { list, claim, update, remove, statuses, endpoints, onStatus }`
    - `list.invoke(): Promise<IApiRobot[]>`（`GET /api/robots` → `{ robots }` 解包）
    - `claim.invoke({ code: string; companion_id: CompanionId }): Promise<IApiRobot>`（`POST /api/robots/claim`）
    - `update.invoke({ robot_id: string; updates: { name?: string; companion_id?: CompanionId | null } }): Promise<IApiRobot>`（`PATCH /api/robots/{robot_id}`）
    - `remove.invoke({ robot_id: string }): Promise<void>`（`DELETE /api/robots/{robot_id}`）
    - `statuses.invoke(): Promise<IApiRobotStatus[]>`（`GET /api/robots/statuses` → `{ statuses }` 解包）
    - `endpoints.invoke(): Promise<IApiRobotEndpoints>`（`GET /api/robots/endpoints`）
    - `onStatus.on(cb: (event: IApiRobotStatus) => void): () => void`（`wsMappedEmitter<IApiRobotStatus>('robot.status', …)`）
  - `export const ROBOT_STATUS_COLOR: Record<IApiRobotPhase, string>`

- [ ] **Step 1: 写失败测试**

新建 `ui/src/common/adapter/ipcBridge.robot-status-wire.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { InvalidEntityIdError } from '@/common/types/ids';
import { robot, type IApiRobot, type IApiRobotStatus } from './ipcBridge';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const COMPANION_ID = '0190f5fe-7c00-7a00-8000-0000000000c1';
const ROBOT_ID = 'aa:bb:cc:dd:ee:ff';
const realFetch = globalThis.fetch;

function respondWith(data: unknown): void {
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(JSON.stringify({ success: true, data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )) as unknown as typeof fetch;
}

const rawRobot = (companionId: unknown) => ({
  robot_id: ROBOT_ID,
  name: '书桌机器人',
  companion_id: companionId,
  board: 'esp32-s3n16r8-emoji',
  firmware_version: '1.9.0',
  last_seen: '2026-08-06T10:00:00Z',
  created_at: '2026-08-05T09:00:00Z',
});

describe('robot wire contract', () => {
  test('the six routes and the push event name are the ones the backend serves', () => {
    expect(source.includes("'/api/robots'")).toBe(true);
    expect(source.includes("'/api/robots/claim'")).toBe(true);
    expect(source.includes("'/api/robots/statuses'")).toBe(true);
    expect(source.includes("'/api/robots/endpoints'")).toBe(true);
    expect(source.includes('`/api/robots/${p.robot_id}`')).toBe(true);
    expect(source.includes("wsMappedEmitter<IApiRobotStatus>('robot.status'")).toBe(true);
  });

  test('robot_id is NOT branded — it is the device MAC, not a UUIDv7', () => {
    // Every other entity id in this bridge is a canonical UUIDv7 and gets a
    // parser. A robot is keyed by its Device-Id (MAC address) because that is
    // what the firmware reports, so branding it would reject every real device.
    expect(source.includes('parseRobotId')).toBe(false);
    expect(source.includes('robot_id: string;')).toBe(true);
  });

  test('every phase the backend can publish is a declared literal', () => {
    for (const phase of ['offline', 'idle', 'listening', 'speaking']) {
      expect(source.includes(`'${phase}'`)).toBe(true);
    }
    expect(source.includes('export type IApiRobotPhase')).toBe(true);
  });

  test('the snapshot and the push path share one mapper, so a status cannot differ by arrival route', () => {
    expect(source.includes('const fromApiRobotStatus')).toBe(true);
    expect(source.split('fromApiRobotStatus').length - 1).toBeGreaterThanOrEqual(3);
    expect(source.includes('changed_at: number;')).toBe(true);
  });

  test('the list route unwraps the {robots} envelope', async () => {
    try {
      respondWith({ robots: [rawRobot(COMPANION_ID)] });
      const rows: IApiRobot[] = await robot.list.invoke();
      expect(rows).toHaveLength(1);
      expect(rows[0]?.robot_id).toBe(ROBOT_ID);
      expect(rows[0]?.companion_id).toBe(COMPANION_ID);
      expect(rows[0]?.firmware_version).toBe('1.9.0');
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('an unbound robot arrives as an explicit null owner, and a legacy id is rejected', async () => {
    try {
      respondWith({ robots: [rawRobot(null)] });
      const unbound: IApiRobot[] = await robot.list.invoke();
      expect(unbound[0]?.companion_id).toBe(null);

      respondWith({ robots: [rawRobot(`companion_${COMPANION_ID}`)] });
      let error: unknown;
      try {
        await robot.list.invoke();
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof InvalidEntityIdError).toBe(true);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('the statuses snapshot unwraps {statuses} and brands the owner', async () => {
    try {
      respondWith({
        statuses: [
          { robot_id: ROBOT_ID, companion_id: COMPANION_ID, phase: 'listening', changed_at: 7 },
        ],
      });
      const rows: IApiRobotStatus[] = await robot.statuses.invoke();
      expect(rows[0]?.phase).toBe('listening');
      expect(rows[0]?.changed_at).toBe(7);
      expect(rows[0]?.companion_id).toBe(COMPANION_ID);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('the endpoints route reports both the OTA candidates and the LAN switch', async () => {
    try {
      respondWith({ ota_urls: ['http://192.168.1.5:25808/robot/ota'], lan_enabled: false });
      const endpoints = await robot.endpoints.invoke();
      expect(endpoints.ota_urls).toEqual(['http://192.168.1.5:25808/robot/ota']);
      expect(endpoints.lan_enabled).toBe(false);
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/common/adapter/ipcBridge.robot-status-wire.test.ts`
Expected: FAIL with `export named 'robot' not found in module`

- [ ] **Step 3: 写最小实现**

3a. `ui/src/common/adapter/ipcBridge.ts` —— 在 `export const ssh = { … };` 之后插入：

```ts
// ---------------------------------------------------------------------------
// Physical robots — ESP32 devices bound to a companion, served by the embedded
// robot gateway (`/robot/*` for the DEVICE, `/api/robots*` for this UI).
//
// A robot is keyed by `robot_id`, which is the firmware's Device-Id — a MAC
// address, not a UUIDv7 — so unlike every other entity id in this bridge it is
// deliberately NOT branded: a parser would reject every real device.
// ---------------------------------------------------------------------------

/** Live phase of one robot. `offline` = no WS session right now. */
export type IApiRobotPhase = 'offline' | 'idle' | 'listening' | 'speaking';

/** One registered robot. `companion_id === null` = paired with nobody yet. */
export interface IApiRobot {
  robot_id: string;
  name: string;
  companion_id: CompanionId | null;
  /** Firmware board type, e.g. `esp32-s3n16r8-emoji`. */
  board: string;
  firmware_version: string;
  /** RFC 3339, or null when the device has never reported in. */
  last_seen: string | null;
  /** RFC 3339. */
  created_at: string;
}

/**
 * The single wire shape for robot liveness: the `robot.status` event and the
 * `/api/robots/statuses` snapshot both carry it, so a robot cannot look
 * different depending on how the client learned about it. `changed_at` is when
 * the phase CHANGED (ms), not when it was asked — which is what makes it a
 * usable tiebreak across both arrival paths.
 */
export interface IApiRobotStatus {
  robot_id: string;
  companion_id: CompanionId | null;
  phase: IApiRobotPhase;
  changed_at: number;
}

/**
 * Where a device should be pointed, and whether it can reach us at all.
 * `ota_urls` lists one candidate per non-loopback NIC; `lan_enabled` is the LAN
 * listener's state — with it off, no device can connect no matter what it is
 * configured with.
 */
export interface IApiRobotEndpoints {
  ota_urls: string[];
  lan_enabled: boolean;
}

const fromApiRobot = (value: IApiRobot): IApiRobot => ({
  ...value,
  companion_id: value.companion_id == null ? null : parseCompanionId(value.companion_id),
});

const fromApiRobotStatus = (value: IApiRobotStatus): IApiRobotStatus => ({
  ...value,
  companion_id: value.companion_id == null ? null : parseCompanionId(value.companion_id),
});

export const robot = {
  list: withResponseMap(httpGet<{ robots: IApiRobot[] }, void>('/api/robots'), (payload) =>
    (payload.robots ?? []).map(fromApiRobot)
  ),
  /**
   * Claim the device showing `code` for `companion_id`.
   * 404 = no such code (mistyped or expired); 409 = already bound to another
   * companion. The caller surfaces the backend message verbatim.
   */
  claim: withResponseMap(
    httpPost<IApiRobot, { code: string; companion_id: CompanionId }>('/api/robots/claim'),
    fromApiRobot
  ),
  /** Rename, rebind (`companion_id`) or unbind (`companion_id: null`). */
  update: withResponseMap(
    httpPatch<IApiRobot, { robot_id: string; updates: { name?: string; companion_id?: CompanionId | null } }>(
      (p) => `/api/robots/${p.robot_id}`,
      (p) => p.updates
    ),
    fromApiRobot
  ),
  /** Revoke the device token and forget the record; the device becomes new again. */
  remove: httpDelete<void, { robot_id: string }>((p) => `/api/robots/${p.robot_id}`),
  /** Snapshot of every robot's phase. Plural for the same reason ssh statuses is. */
  statuses: withResponseMap(
    httpGet<{ statuses: IApiRobotStatus[] }, void>('/api/robots/statuses'),
    (payload) => (payload.statuses ?? []).map(fromApiRobotStatus)
  ),
  endpoints: httpGet<IApiRobotEndpoints, void>('/api/robots/endpoints'),
  /** Every phase transition, owner-scoped. Same payload as `statuses`. */
  onStatus: wsMappedEmitter<IApiRobotStatus>('robot.status', (raw) =>
    fromApiRobotStatus(raw as IApiRobotStatus)
  ),
};
```

3b. `ui/src/renderer/components/capability/capabilityStatusColors.ts` —— `:7` 的 import 加 `IApiRobotPhase`，文件末尾追加：

```ts
/**
 * Robot phase → colour for the 机器人连接 list pill.
 *
 * `idle` is green because it means the device IS connected and waiting — for a
 * physical robot "reachable" is the good state, and `offline` (gray) is the
 * neutral absence, not a fault: a robot that is simply unplugged must not glow
 * red. `listening` / `speaking` share the primary tint: something is happening
 * right now, and distinguishing them by hue would only add noise to a row whose
 * label already says which.
 */
export const ROBOT_STATUS_COLOR: Record<IApiRobotPhase, string> = {
  offline: CAPABILITY_COLORS.off,
  idle: CAPABILITY_COLORS.active,
  listening: CAPABILITY_COLORS.primary,
  speaking: CAPABILITY_COLORS.primary,
};
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun test --cwd ui src/common/adapter/ipcBridge.robot-status-wire.test.ts`
Expected: PASS
Run: `bun run typecheck`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/common/adapter/ipcBridge.ts \
        ui/src/common/adapter/ipcBridge.robot-status-wire.test.ts \
        ui/src/renderer/components/capability/capabilityStatusColors.ts
git commit -m "feat(robot): add the robot management wire contract to the bridge"
```

---

### Task 13: `useRobotStatuses` 三段式实时投影

**Files:**
- Create: `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.ts`
- Create: `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.structure.test.ts`

**Interfaces:**
- Consumes: Task 12 的 `ipcBridge.robot.statuses` / `ipcBridge.robot.onStatus` / `IApiRobotStatus`；既有 `ipcBridge.conversation.reconnected`（`useSshLinkStatus.ts:89` 同款）。
- Produces: `export function useRobotStatuses(): Record<string, IApiRobotStatus>`（键 = `robot_id`）；`export default useRobotStatuses`。

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.structure.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const src = readFileSync(new URL('./useRobotStatuses.ts', import.meta.url), 'utf8');

describe('useRobotStatuses is the three-part realtime shape', () => {
  test('takes a snapshot on mount', () => {
    // The socket has no replay buffer: a robot that was already speaking before
    // this section mounted is only learnable by asking.
    expect(src.includes('ipcBridge.robot.statuses.invoke()')).toBe(true);
  });

  test('merges incremental events', () => {
    expect(src.includes('ipcBridge.robot.onStatus.on(')).toBe(true);
  });

  test('re-snapshots when the socket reconnects', () => {
    // Frames dropped while the socket was down are never replayed, and a stale
    // "speaking" is the worst possible lie for this pill.
    expect(src.includes('ipcBridge.conversation.reconnected.on(')).toBe(true);
  });

  test('the newer changed_at wins so an out-of-order delivery cannot walk state back', () => {
    expect(src.includes('prev.changed_at > next.changed_at')).toBe(true);
  });

  test('listeners are installed before the snapshot is requested', () => {
    // Otherwise a transition emitted mid-flight falls into a subscribe gap.
    const offStatus = src.indexOf('ipcBridge.robot.onStatus.on(');
    const firstSnapshot = src.indexOf('resnapshot();', offStatus);
    expect(offStatus).toBeGreaterThan(0);
    expect(firstSnapshot).toBeGreaterThan(offStatus);
  });

  test('keyed by robot_id, so a rebound robot keeps its live phase', () => {
    expect(src.includes('Record<string, IApiRobotStatus>')).toBe(true);
    expect(src.includes('[next.robot_id]: next')).toBe(true);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.structure.test.ts`
Expected: FAIL with `ENOENT … useRobotStatuses.ts`

- [ ] **Step 3: 写最小实现**

新建 `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useState } from 'react';
import { ipcBridge } from '@/common';
import type { IApiRobotStatus } from '@/common/adapter/ipcBridge';

/**
 * Live phase of every robot this installation owns, keyed by `robot_id`.
 *
 * Same three-part shape every durable realtime projection in this renderer uses
 * (see `useSshLinkStatus`):
 *
 * 1. a snapshot on mount — the socket has no replay buffer, so a robot that was
 *    already connected before this section mounted is only learnable by asking;
 * 2. incremental patches from `robot.status`;
 * 3. a re-snapshot on socket reconnect, because frames dropped while the socket
 *    was down are never replayed and a stale `speaking` is the worst possible
 *    lie for this pill.
 *
 * Not filtered by companion: the map is keyed by device, so rebinding a robot to
 * another companion keeps its live phase instead of blanking it for one refetch.
 * Listeners are installed before the snapshot is requested, so a transition
 * emitted mid-flight cannot fall into a snapshot/subscribe gap; the newest
 * `changed_at` wins if it does arrive out of order.
 */
export function useRobotStatuses(): Record<string, IApiRobotStatus> {
  const [statuses, setStatuses] = useState<Record<string, IApiRobotStatus>>({});

  useEffect(() => {
    let disposed = false;

    const apply = (next: IApiRobotStatus): void => {
      if (disposed) return;
      setStatuses((prev) => {
        const current = prev[next.robot_id];
        // An out-of-order delivery must not walk a newer phase backwards.
        if (current != null && current.changed_at > next.changed_at) return prev;
        return { ...prev, [next.robot_id]: next };
      });
    };

    const resnapshot = (): void => {
      void (async () => {
        try {
          const rows = await ipcBridge.robot.statuses.invoke();
          if (disposed) return;
          // Through `apply`, not straight into state: a re-fetch is a READ of
          // the same transitions the events carry, so an in-flight snapshot that
          // answers after a newer event must not walk a pill backwards.
          rows.forEach(apply);
        } catch {
          // A failed snapshot leaves whatever we already knew in place rather
          // than blanking robots that are probably still up. The section's own
          // list request is what reports a broken backend to the user.
        }
      })();
    };

    const offStatus = ipcBridge.robot.onStatus.on(apply);
    const offReconnected = ipcBridge.conversation.reconnected.on(() => {
      resnapshot();
    });

    resnapshot();

    return () => {
      disposed = true;
      offStatus();
      offReconnected();
    };
  }, []);

  return statuses;
}

export default useRobotStatuses;
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.structure.test.ts`
Expected: PASS
Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/shell.structure.test.ts src/renderer/pages/nomi/workspace/rulesOfHooks.test.ts`
Expected: PASS
Run: `bun run typecheck`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.ts \
        ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/useRobotStatuses.structure.test.ts
git commit -m "feat(robot): project robot.status into a live per-device map"
```

---

### Task 14: 「机器人连接」section + 远程 Tab 接线 + IM 文案区分

**Files:**
- Create: `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/AddRobotModal.tsx`
- Create: `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/RobotConnectSection.tsx`
- Create: `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/RobotConnectSection.structure.test.ts`
- Modify: `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/index.tsx:24-56`（插 section + attention 双来源）
- Modify: `ui/src/renderer/services/i18n/locales/{zh-CN,en-US}/nomi.json`（新增 `robot` 顶层块；修 `settings.remoteCreateBot` / `remoteUnboundBot` / `remoteOtherBots` / `remoteBotIdentity` 的 IM 措辞）

**Interfaces:**
- Consumes: Task 12 的 `ipcBridge.robot.*` 与 `ROBOT_STATUS_COLOR`；Task 13 的 `useRobotStatuses()`；既有 `webui.getStatus` / `webui.start` / `webui.lifecycleSupported`（`ipcBridge.ts:2351-2375`）、`CopyIconButton`（`@/renderer/components/base/CopyIconButton`）、`NomiModal`、`NomiSettingSection/List/Row`、`isBackendHttpError`（`@/common/adapter/httpBridge`）。
- Produces:
  - `RobotConnectSection` props `{ companionId: CompanionId; companionName: string; onAttentionChange?: (hasAttention: boolean) => void }`
  - `AddRobotModal` props `{ visible: boolean; companionId: CompanionId; companionName: string; onCancel: () => void; onClaimed: () => void }`

**i18n 归属决定（写死，不再讨论）：** 机器人文案放 `nomi.json` 的新顶层 `robot` 块（zh + en 各一份）。理由：SSH 之所以自成 `ssh.json` 命名空间，是因为它横跨设置页、会话分组、会话头药丸与新建入口四个面；本期机器人 UI 只有伙伴工作台里一个 section，与 `nomi.settings.remote*`（IM 节）同处一个 Tab，放同一个命名空间才不会让同一屏文案跨两个文件。

- [ ] **Step 1: 写失败测试**

新建 `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/RobotConnectSection.structure.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import zhNomi from '@/renderer/services/i18n/locales/zh-CN/nomi.json';
import enNomi from '@/renderer/services/i18n/locales/en-US/nomi.json';

const section = readFileSync(new URL('./RobotConnectSection.tsx', import.meta.url), 'utf8');
const modal = readFileSync(new URL('./AddRobotModal.tsx', import.meta.url), 'utf8');
const tab = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const imSection = readFileSync(new URL('./RemoteConnectSection.tsx', import.meta.url), 'utf8');

describe('robot connect section', () => {
  test('lists only this companion’s robots and pills them from the live map', () => {
    expect(section.includes('useRobotStatuses()')).toBe(true);
    expect(section.includes('ROBOT_STATUS_COLOR')).toBe(true);
    expect(section.includes('row.companion_id === companionId')).toBe(true);
  });

  test('offers rename, unbind and delete, and delete is a danger confirm', () => {
    expect(section.includes('ipcBridge.robot.update.invoke')).toBe(true);
    expect(section.includes('companion_id: null')).toBe(true);
    expect(section.includes('ipcBridge.robot.remove.invoke')).toBe(true);
    expect(section.includes("okButtonProps: { status: 'danger' }")).toBe(true);
  });

  test('a failed list read renders an explanation, never a crash or a silent empty', () => {
    // Plan A's backend may not be deployed yet, and a 404 must read as "cannot
    // reach the robot service", not as "you own no robots".
    expect(section.includes("t('nomi.robot.loadFailed')")).toBe(true);
  });

  test('the add dialog shows every OTA candidate with a copy button and the 6-digit code field', () => {
    expect(modal.includes('ipcBridge.robot.endpoints.invoke()')).toBe(true);
    expect(modal.includes('<CopyIconButton')).toBe(true);
    expect(modal.includes('maxLength={6}')).toBe(true);
    expect(modal.includes('ipcBridge.robot.claim.invoke')).toBe(true);
  });

  test('a wrong code and an already-claimed device get their own message', () => {
    expect(modal.includes('claimNotFound')).toBe(true);
    expect(modal.includes('claimTaken')).toBe(true);
    expect(modal.includes('status === 404')).toBe(true);
    expect(modal.includes('status === 409')).toBe(true);
  });

  test('the LAN dependency is stated and can be switched on from the dialog', () => {
    expect(modal.includes('lan_enabled')).toBe(true);
    expect(modal.includes('webui.start.invoke')).toBe(true);
    expect(modal.includes('webui.lifecycleSupported')).toBe(true);
  });

  test('the tab renders the section and aggregates attention from BOTH sources', () => {
    expect(tab.includes('<RobotConnectSection')).toBe(true);
    expect(tab.includes('pendingPairings > 0 || robotAttention')).toBe(true);
  });

  test('the IM section says "IM robot" so the two never collide on screen', () => {
    // 「远程连接」节里的「机器人」一直指 IM bot; the new section is about physical
    // hardware, so the older copy has to name its own kind.
    expect(imSection.includes("t('nomi.settings.remoteCreateBot')")).toBe(true);
    const zh = (zhNomi as unknown as { settings: Record<string, string> }).settings;
    expect(zh.remoteCreateBot).toBe('连接 IM 机器人');
    expect(zh.remoteBotIdentity.startsWith('IM 机器人')).toBe(true);
  });

  test('robot copy is complete in both locales', () => {
    const keys = [
      'title',
      'hint',
      'add',
      'addTitle',
      'otaStep',
      'otaNone',
      'codeStep',
      'codePlaceholder',
      'claim',
      'claimOk',
      'claimNotFound',
      'claimTaken',
      'claimFailed',
      'lanOff',
      'lanEnable',
      'lanEnabled',
      'lanEnableFailed',
      'lanUnavailable',
      'empty',
      'board',
      'firmware',
      'lastSeen',
      'lastSeenNever',
      'rename',
      'renameTitle',
      'renamePlaceholder',
      'renameFailed',
      'unbind',
      'unbindConfirm',
      'remove',
      'removeConfirm',
      'removeFailed',
      'loadFailed',
    ];
    for (const locale of [zhNomi, enNomi]) {
      const robot = (locale as unknown as { robot: Record<string, unknown> }).robot;
      for (const key of keys) {
        expect(typeof robot[key]).toBe('string');
      }
      const status = robot.status as Record<string, string>;
      for (const phase of ['offline', 'idle', 'listening', 'speaking']) {
        expect(typeof status[phase]).toBe('string');
      }
    }
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/tabs/RemoteTab/RobotConnectSection.structure.test.ts`
Expected: FAIL with `ENOENT … RobotConnectSection.tsx`

- [ ] **Step 3: 写最小实现**

3a. 新建 `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/AddRobotModal.tsx`：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Message } from '@arco-design/web-react';
import { ipcBridge, webui } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { IApiRobotEndpoints } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import NomiModal from '@/renderer/components/base/NomiModal';

interface AddRobotModalProps {
  visible: boolean;
  companionId: CompanionId;
  companionName: string;
  onCancel: () => void;
  onClaimed: () => void;
}

/**
 * 「添加机器人」弹窗：两步——把设备指向本机的 OTA 地址，然后输入设备屏上的 6 位激活码。
 *
 * Every non-loopback NIC is listed because the machine the robot can reach is
 * not necessarily the one the user thinks of as "the" address. The LAN listener
 * gate is stated here rather than hidden: with it off the device cannot connect
 * whatever it is configured with, so the dialog offers to switch it on.
 */
const AddRobotModal: React.FC<AddRobotModalProps> = ({
  visible,
  companionId,
  companionName,
  onCancel,
  onClaimed,
}) => {
  const { t } = useTranslation();
  const [endpoints, setEndpoints] = useState<IApiRobotEndpoints | null>(null);
  const [code, setCode] = useState('');
  const [claiming, setClaiming] = useState(false);
  const [enablingLan, setEnablingLan] = useState(false);

  const refreshEndpoints = useCallback(async () => {
    try {
      setEndpoints(await ipcBridge.robot.endpoints.invoke());
    } catch (error) {
      console.error('[RobotConnect] Failed to read the robot endpoints:', error);
      setEndpoints(null);
    }
  }, []);

  useEffect(() => {
    if (!visible) return;
    setCode('');
    void refreshEndpoints();
  }, [visible, refreshEndpoints]);

  const enableLan = useCallback(async () => {
    setEnablingLan(true);
    try {
      const status = await webui.start.invoke();
      if (status.error) throw new Error(status.error);
      Message.success(t('nomi.robot.lanEnabled'));
      await refreshEndpoints();
    } catch (error) {
      console.error('[RobotConnect] Failed to enable LAN access:', error);
      Message.error(t('nomi.robot.lanEnableFailed'));
    } finally {
      setEnablingLan(false);
    }
  }, [refreshEndpoints, t]);

  const claim = useCallback(async () => {
    setClaiming(true);
    try {
      await ipcBridge.robot.claim.invoke({ code: code.trim(), companion_id: companionId });
      Message.success(t('nomi.robot.claimOk', { companionName }));
      onClaimed();
    } catch (error) {
      console.error('[RobotConnect] Failed to claim a robot:', error);
      const status = isBackendHttpError(error) ? error.status : 0;
      if (status === 404) {
        Message.error(t('nomi.robot.claimNotFound'));
      } else if (status === 409) {
        Message.error(t('nomi.robot.claimTaken'));
      } else {
        Message.error(t('nomi.robot.claimFailed'));
      }
    } finally {
      setClaiming(false);
    }
  }, [code, companionId, companionName, onClaimed, t]);

  const lanOff = endpoints != null && !endpoints.lan_enabled;

  return (
    <NomiModal
      visible={visible}
      onCancel={onCancel}
      header={{ title: t('nomi.robot.addTitle'), showClose: true }}
      footer={null}
      style={{ width: 560 }}
    >
      <div className='flex flex-col gap-14px py-4px'>
        {lanOff && (
          <div className='flex flex-wrap items-center gap-8px rd-8px border border-solid border-[rgba(var(--warning-6),0.32)] bg-[rgba(var(--warning-6),0.08)] px-12px py-8px'>
            <span className='min-w-0 flex-1 text-12px leading-18px text-t-primary'>
              {t('nomi.robot.lanOff')}
            </span>
            {webui.lifecycleSupported ? (
              <Button size='mini' type='primary' loading={enablingLan} onClick={() => void enableLan()}>
                {t('nomi.robot.lanEnable')}
              </Button>
            ) : (
              <span className='text-12px text-t-tertiary'>{t('nomi.robot.lanUnavailable')}</span>
            )}
          </div>
        )}

        <div className='flex flex-col gap-6px'>
          <span className='text-12px leading-18px text-t-secondary'>{t('nomi.robot.otaStep')}</span>
          {endpoints == null || endpoints.ota_urls.length === 0 ? (
            <span className='text-12px text-t-tertiary'>{t('nomi.robot.otaNone')}</span>
          ) : (
            endpoints.ota_urls.map((url) => (
              <div
                key={url}
                className='flex min-w-0 items-center gap-8px rd-8px border border-solid border-[var(--color-border-2)] px-10px py-6px'
              >
                <span className='min-w-0 flex-1 truncate font-mono text-12px text-t-primary'>{url}</span>
                <CopyIconButton text={url} size={14} className='h-22px w-22px shrink-0' />
              </div>
            ))
          )}
        </div>

        <div className='flex flex-col gap-6px'>
          <span className='text-12px leading-18px text-t-secondary'>{t('nomi.robot.codeStep')}</span>
          <div className='flex items-center gap-8px'>
            <Input
              value={code}
              maxLength={6}
              placeholder={t('nomi.robot.codePlaceholder')}
              className='max-w-160px'
              onChange={(next: string) => setCode(next.replace(/\D/g, ''))}
            />
            <Button
              type='primary'
              loading={claiming}
              disabled={code.trim().length !== 6}
              onClick={() => void claim()}
            >
              {t('nomi.robot.claim')}
            </Button>
          </div>
        </div>
      </div>
    </NomiModal>
  );
};

export default AddRobotModal;
```

3b. 新建 `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/RobotConnectSection.tsx`：

```tsx
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import dayjs from 'dayjs';
import { Button, Input, Message, Modal, Tag } from '@arco-design/web-react';
import { Robot } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IApiRobot } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import { ROBOT_STATUS_COLOR } from '@/renderer/components/capability/capabilityStatusColors';
import AddRobotModal from './AddRobotModal';
import { useRobotStatuses } from './useRobotStatuses';

interface RobotConnectSectionProps {
  companionId: CompanionId;
  companionName: string;
  onAttentionChange?: (hasAttention: boolean) => void;
}

/**
 * 「机器人连接」节：绑到这只伙伴的实体机器人。
 *
 * 与同一 Tab 上方的「远程连接」节严格区分——那里的「机器人」指 IM bot（渠道插件），
 * 这里指真实硬件设备。Attention 只在**可行动**时点亮：本伙伴已绑机器人、但局域网访问
 * 关着，于是设备无论如何都连不上电脑。设备单纯离线（拔电、带走了）不是待办。
 */
const RobotConnectSection: React.FC<RobotConnectSectionProps> = ({
  companionId,
  companionName,
  onAttentionChange,
}) => {
  const { t } = useTranslation();
  const statuses = useRobotStatuses();
  const [robots, setRobots] = useState<IApiRobot[]>([]);
  const [loadFailed, setLoadFailed] = useState(false);
  const [lanEnabled, setLanEnabled] = useState<boolean | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [busyRobotId, setBusyRobotId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [rows, endpoints] = await Promise.all([
        ipcBridge.robot.list.invoke(),
        ipcBridge.robot.endpoints.invoke(),
      ]);
      setRobots(rows);
      setLanEnabled(endpoints.lan_enabled);
      setLoadFailed(false);
    } catch (error) {
      console.error('[RobotConnect] Failed to load robots:', error);
      setLoadFailed(true);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const mine = useMemo(
    () => robots.filter((row) => row.companion_id === companionId),
    [robots, companionId]
  );

  const attention = mine.length > 0 && lanEnabled === false;
  useEffect(() => {
    onAttentionChange?.(attention);
  }, [attention, onAttentionChange]);

  const rename = useCallback(
    (row: IApiRobot) => {
      let draft = row.name;
      Modal.confirm({
        title: t('nomi.robot.renameTitle'),
        content: (
          <Input
            defaultValue={row.name}
            placeholder={t('nomi.robot.renamePlaceholder')}
            onChange={(next: string) => {
              draft = next;
            }}
          />
        ),
        onOk: async () => {
          setBusyRobotId(row.robot_id);
          try {
            await ipcBridge.robot.update.invoke({
              robot_id: row.robot_id,
              updates: { name: draft.trim() || row.name },
            });
            await refresh();
          } catch (error) {
            console.error('[RobotConnect] Failed to rename a robot:', error);
            Message.error(t('nomi.robot.renameFailed'));
          } finally {
            setBusyRobotId(null);
          }
        },
      });
    },
    [refresh, t]
  );

  const unbind = useCallback(
    (row: IApiRobot) => {
      Modal.confirm({
        title: t('nomi.robot.unbind'),
        content: t('nomi.robot.unbindConfirm'),
        onOk: async () => {
          setBusyRobotId(row.robot_id);
          try {
            await ipcBridge.robot.update.invoke({
              robot_id: row.robot_id,
              updates: { companion_id: null },
            });
            await refresh();
          } catch (error) {
            console.error('[RobotConnect] Failed to unbind a robot:', error);
            Message.error(t('nomi.robot.renameFailed'));
          } finally {
            setBusyRobotId(null);
          }
        },
      });
    },
    [refresh, t]
  );

  const remove = useCallback(
    (row: IApiRobot) => {
      Modal.confirm({
        title: t('nomi.robot.remove'),
        content: t('nomi.robot.removeConfirm'),
        okButtonProps: { status: 'danger' },
        onOk: async () => {
          setBusyRobotId(row.robot_id);
          try {
            await ipcBridge.robot.remove.invoke({ robot_id: row.robot_id });
            await refresh();
          } catch (error) {
            console.error('[RobotConnect] Failed to delete a robot:', error);
            Message.error(t('nomi.robot.removeFailed'));
          } finally {
            setBusyRobotId(null);
          }
        },
      });
    },
    [refresh, t]
  );

  const phaseOf = (row: IApiRobot) => statuses[row.robot_id]?.phase ?? 'offline';

  return (
    <>
      <NomiSettingSection
        title={t('nomi.robot.title')}
        description={t('nomi.robot.hint', { companionName })}
        action={
          <Button size='small' type='primary' onClick={() => setAddOpen(true)}>
            {t('nomi.robot.add')}
          </Button>
        }
      >
        <NomiSettingList>
          {loadFailed ? (
            <NomiSettingRow title={t('nomi.robot.loadFailed')} />
          ) : mine.length === 0 ? (
            <NomiSettingRow title={t('nomi.robot.empty')} />
          ) : (
            mine.map((row) => (
              <NomiSettingRow
                key={row.robot_id}
                leading={
                  <Robot
                    theme='outline'
                    size='16'
                    fill='currentColor'
                    strokeWidth={3}
                    className='shrink-0'
                    style={{ color: ROBOT_STATUS_COLOR[phaseOf(row)] }}
                  />
                }
                title={
                  <div className='flex min-w-0 flex-wrap items-center gap-6px'>
                    <span className='truncate'>{row.name}</span>
                    <Tag size='small' bordered={false} style={{ color: ROBOT_STATUS_COLOR[phaseOf(row)] }}>
                      {t(`nomi.robot.status.${phaseOf(row)}` as never)}
                    </Tag>
                  </div>
                }
                description={[
                  t('nomi.robot.board', { board: row.board }),
                  t('nomi.robot.firmware', { version: row.firmware_version }),
                  row.last_seen
                    ? t('nomi.robot.lastSeen', {
                        time: dayjs(row.last_seen).format('YYYY-MM-DD HH:mm'),
                      })
                    : t('nomi.robot.lastSeenNever'),
                ].join(' · ')}
                controls={
                  <>
                    <Button size='small' loading={busyRobotId === row.robot_id} onClick={() => rename(row)}>
                      {t('nomi.robot.rename')}
                    </Button>
                    <Button size='small' onClick={() => unbind(row)}>
                      {t('nomi.robot.unbind')}
                    </Button>
                    <Button size='small' status='danger' onClick={() => remove(row)}>
                      {t('nomi.robot.remove')}
                    </Button>
                  </>
                }
              />
            ))
          )}
        </NomiSettingList>
      </NomiSettingSection>

      <AddRobotModal
        visible={addOpen}
        companionId={companionId}
        companionName={companionName}
        onCancel={() => setAddOpen(false)}
        onClaimed={() => {
          setAddOpen(false);
          void refresh();
        }}
      />
    </>
  );
};

export default RobotConnectSection;
```

**注意**：上面 `t(\`nomi.robot.status.${phaseOf(row)}\` as never)` 用了 `as never`——`SshHostManagement` 的结构测试禁止这种写法。改用显式键表，替换 `phaseOf` 之后的用法：

```tsx
import type { IApiRobotPhase } from '@/common/adapter/ipcBridge';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';

/** One key per phase the backend can publish; a new phase must fail to compile. */
const PHASE_LABEL_KEY: Record<IApiRobotPhase, I18nKey> = {
  offline: 'nomi.robot.status.offline',
  idle: 'nomi.robot.status.idle',
  listening: 'nomi.robot.status.listening',
  speaking: 'nomi.robot.status.speaking',
};
```

并把标签渲染改成 `{t(PHASE_LABEL_KEY[phaseOf(row)])}`。

3c. `ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/index.tsx` —— import 加 `RobotConnectSection`，组件体改为（hook 全部在 early return 之前）：

```tsx
const RemoteTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { profile, status } = companion;

  const pendingPairings = usePairingAttention(profile ? companionId : null);
  const [robotAttention, setRobotAttention] = useState(false);

  const attentionRef = useRef(onAttentionChange);
  attentionRef.current = onAttentionChange;
  useEffect(() => {
    attentionRef.current?.(pendingPairings > 0 || robotAttention);
  }, [pendingPairings, robotAttention]);
  // Leaving the tab must not leave a stale dot behind.
  useEffect(() => () => attentionRef.current?.(false), []);

  if (!profile) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  return (
    <div className='flex flex-col gap-16px py-8px'>
      {/* IM 渠道：按伙伴接待（platform → companionId 反向视图）/ Per-companion IM channels */}
      <RemoteConnectSection companionId={profile.companion_id} companionName={profile.name} />
      {/* 实体机器人：绑到这只伙伴的硬件设备 / Physical robots bound to this companion */}
      <RobotConnectSection
        companionId={profile.companion_id}
        companionName={profile.name}
        onAttentionChange={setRobotAttention}
      />
      <AccessTokenSection
        companionId={profile.companion_id}
        companionName={profile.name}
        modelConfigured={status?.model_configured ?? null}
      />
    </div>
  );
};
```

并把 `:8` 的 import 改为 `import React, { useEffect, useRef, useState } from 'react';`，文件头文档注释里的「两条路径」改为「三条路径」。

3d. `ui/src/renderer/services/i18n/locales/zh-CN/nomi.json` —— 新增顶层 `robot` 块：

```json
  "robot": {
    "title": "机器人连接",
    "hint": "把实体机器人接到 {{companionName}}：它会用这只伙伴的人格、模型和记忆跟你说话。",
    "add": "添加机器人",
    "addTitle": "添加机器人",
    "otaStep": "1. 打开机器人的配网页，在「高级设置」里把 OTA 地址填成下面任意一个：",
    "otaNone": "还没有可用的局域网地址。",
    "codeStep": "2. 机器人开机后会在屏幕上显示并读出 6 位激活码，把它填在这里：",
    "codePlaceholder": "6 位数字",
    "claim": "绑定到本伙伴",
    "claimOk": "机器人已绑定到 {{companionName}}",
    "claimNotFound": "激活码不存在或已失效，请再看一眼机器人屏幕上的号码。",
    "claimTaken": "这台机器人已经绑定到别的伙伴了，先在那只伙伴那里解绑。",
    "claimFailed": "绑定失败",
    "lanOff": "机器人要连上电脑，需要先打开「局域网访问」。",
    "lanEnable": "现在开启",
    "lanEnabled": "局域网访问已开启",
    "lanEnableFailed": "开启局域网访问失败",
    "lanUnavailable": "请在桌面应用里开启局域网访问。",
    "empty": "还没有绑定机器人",
    "board": "板型 {{board}}",
    "firmware": "固件 {{version}}",
    "lastSeen": "最近在线 {{time}}",
    "lastSeenNever": "还没连上过",
    "rename": "重命名",
    "renameTitle": "给机器人改个名字",
    "renamePlaceholder": "例如：书桌机器人",
    "renameFailed": "保存失败",
    "unbind": "解绑",
    "unbindConfirm": "解绑后这台机器人不再属于任何伙伴，会重新显示激活码等人认领。",
    "remove": "删除",
    "removeConfirm": "删除后设备令牌立刻失效，机器人需要重新报到并重新绑定。",
    "removeFailed": "删除失败",
    "loadFailed": "读取机器人列表失败，请确认后台的机器人服务已启动。",
    "status": {
      "offline": "离线",
      "idle": "在线",
      "listening": "聆听中",
      "speaking": "说话中"
    }
  },
```

同文件 `settings` 块内四处 IM 措辞改为：

```json
    "remoteCreateBot": "连接 IM 机器人",
    "remoteUnboundBot": "已有未绑定伙伴的 IM 机器人",
    "remoteOtherBots": "已有 {{num}} 个 IM 机器人由其他伙伴接待：{{companions}}",
    "remoteBotIdentity": "IM 机器人：{{bot}}",
```

3e. `ui/src/renderer/services/i18n/locales/en-US/nomi.json` —— 新增顶层 `robot` 块：

```json
  "robot": {
    "title": "Robot connection",
    "hint": "Attach a physical robot to {{companionName}}: it speaks with this companion's persona, models and memories.",
    "add": "Add a robot",
    "addTitle": "Add a robot",
    "otaStep": "1. Open the robot's Wi-Fi setup page and set the OTA address under Advanced settings to one of these:",
    "otaNone": "No LAN address is available yet.",
    "codeStep": "2. On boot the robot shows and reads out a 6-digit activation code. Enter it here:",
    "codePlaceholder": "6 digits",
    "claim": "Bind to this companion",
    "claimOk": "The robot is now bound to {{companionName}}",
    "claimNotFound": "No such activation code, or it has expired — check the number on the robot's screen again.",
    "claimTaken": "This robot is already bound to another companion. Unbind it there first.",
    "claimFailed": "Binding failed",
    "lanOff": "The robot needs LAN access turned on before it can reach this computer.",
    "lanEnable": "Turn it on",
    "lanEnabled": "LAN access is on",
    "lanEnableFailed": "Failed to turn on LAN access",
    "lanUnavailable": "Turn on LAN access from the desktop app.",
    "empty": "No robot bound yet",
    "board": "Board {{board}}",
    "firmware": "Firmware {{version}}",
    "lastSeen": "Last seen {{time}}",
    "lastSeenNever": "Never connected",
    "rename": "Rename",
    "renameTitle": "Rename this robot",
    "renamePlaceholder": "e.g. Desk robot",
    "renameFailed": "Failed to save",
    "unbind": "Unbind",
    "unbindConfirm": "After unbinding, this robot belongs to no companion and shows an activation code again, waiting to be claimed.",
    "remove": "Delete",
    "removeConfirm": "Deleting revokes the device token immediately; the robot has to report in and be bound again.",
    "removeFailed": "Failed to delete",
    "loadFailed": "Could not read the robot list — check that the robot service is running.",
    "status": {
      "offline": "Offline",
      "idle": "Online",
      "listening": "Listening",
      "speaking": "Speaking"
    }
  },
```

同文件 `settings` 块四处改为：

```json
    "remoteCreateBot": "Connect an IM bot",
    "remoteUnboundBot": "An IM bot without a bound companion already exists",
    "remoteOtherBots": "{{num}} IM bot(s) already answered by other companions: {{companions}}",
    "remoteBotIdentity": "IM bot: {{bot}}",
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun run gen:i18n`
Expected: 无报错
Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/tabs/RemoteTab/`
Expected: PASS
Run: `bun test --cwd ui src/renderer/pages/nomi/workspace/shell.structure.test.ts src/renderer/pages/nomi/workspace/rulesOfHooks.test.ts`
Expected: PASS
Run: `bun run typecheck && bun run check:i18n && bun run check:icons && bun run check:theme`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/pages/nomi/workspace/tabs/RemoteTab/ \
        ui/src/renderer/services/i18n/locales/zh-CN/nomi.json \
        ui/src/renderer/services/i18n/locales/en-US/nomi.json \
        ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "feat(robot): add the robot connection section to the companion remote tab"
```

---

### Task 15: 会话列表排除机器人会话 + i18n / 结构测试收尾

**Files:**
- Modify: `ui/src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.ts`
- Modify: `ui/src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.test.ts`
- Create: `ui/src/renderer/services/i18n/robotLocales.test.ts`

**Interfaces:**
- Consumes: Plan A 写入的会话 `extra = { robot_session: true, robot_id, companion_id }`。
- Produces: `isOrdinaryWorkConversation` 显式识别 `robot_session` / `robot_id`。

- [ ] **Step 1: 写失败测试**

1a. `ui/src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.test.ts` 追加：

```ts
  test('robot sessions never enter the ordinary work list', () => {
    // A robot thread is a long-lived companion conversation owned by a device.
    // It is excluded EXPLICITLY rather than incidentally via `companion_id`:
    // that marker is what the companion group already keys on, and relying on it
    // would silently break the day a robot thread stops carrying it.
    const robotSession = {
      execution_step_id: undefined,
      extra: { robot_session: true, robot_id: 'aa:bb:cc:dd:ee:ff' },
    };
    expect(isOrdinaryWorkConversation(robotSession as never)).toBe(false);

    const robotIdOnly = { execution_step_id: undefined, extra: { robot_id: 'aa:bb:cc:dd:ee:ff' } };
    expect(isOrdinaryWorkConversation(robotIdOnly as never)).toBe(false);
  });
```

1b. 新建 `ui/src/renderer/services/i18n/robotLocales.test.ts`：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import enNomi from './locales/en-US/nomi.json';
import zhNomi from './locales/zh-CN/nomi.json';

type LocaleJson = Record<string, unknown>;

/**
 * One key per `RobotStatusDto.phase` the backend can serialize. The Rust enum,
 * the `IApiRobotPhase` literals, `ROBOT_STATUS_COLOR` and this list must grow
 * together — a phase with no label renders as a raw key in the list pill.
 */
const ROBOT_PHASE_KEYS = [
  'robot.status.offline',
  'robot.status.idle',
  'robot.status.listening',
  'robot.status.speaking',
] as const;

/** Copy carrying placeholders the section interpolates. */
const ROBOT_INTERPOLATED: readonly [string, string][] = [
  ['robot.hint', '{{companionName}}'],
  ['robot.claimOk', '{{companionName}}'],
  ['robot.board', '{{board}}'],
  ['robot.firmware', '{{version}}'],
  ['robot.lastSeen', '{{time}}'],
];

function getLocaleValue(locale: LocaleJson, key: string): unknown {
  let cursor: unknown = locale;
  for (const segment of key.split('.')) {
    if (!cursor || typeof cursor !== 'object' || !Object.prototype.hasOwnProperty.call(cursor, segment)) {
      return undefined;
    }
    cursor = (cursor as LocaleJson)[segment];
  }
  return cursor;
}

describe('robot locale coverage', () => {
  test('every robot phase has a label in both locales', () => {
    const failures: string[] = [];
    for (const [name, locale] of [
      ['en-US', enNomi as unknown as LocaleJson],
      ['zh-CN', zhNomi as unknown as LocaleJson],
    ] as const) {
      for (const key of ROBOT_PHASE_KEYS) {
        const value = getLocaleValue(locale, key);
        if (typeof value !== 'string' || !value.trim()) failures.push(`${name} nomi.${key}`);
      }
    }
    expect(failures).toEqual([]);
  });

  test('interpolated robot copy keeps its placeholders in both locales', () => {
    const failures: string[] = [];
    for (const [name, locale] of [
      ['en-US', enNomi as unknown as LocaleJson],
      ['zh-CN', zhNomi as unknown as LocaleJson],
    ] as const) {
      for (const [key, placeholder] of ROBOT_INTERPOLATED) {
        if (!String(getLocaleValue(locale, key)).includes(placeholder)) {
          failures.push(`${name} nomi.${key} lost ${placeholder}`);
        }
      }
    }
    expect(failures).toEqual([]);
  });

  test('the IM section names its own kind of bot so the two never collide', () => {
    for (const locale of [enNomi as unknown as LocaleJson, zhNomi as unknown as LocaleJson]) {
      expect(String(getLocaleValue(locale, 'settings.remoteCreateBot')).toUpperCase()).toContain('IM');
      expect(String(getLocaleValue(locale, 'settings.remoteBotIdentity')).toUpperCase()).toContain('IM');
    }
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bun test --cwd ui src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.test.ts`
Expected: FAIL with `expected true to be false`（`robot_id`-only 的那条会话目前被判为普通会话）
Run: `bun test --cwd ui src/renderer/services/i18n/robotLocales.test.ts`
Expected: PASS（Task 14 已加齐文案）—— 若红，说明 Task 14 的 locale 漏键，先补齐。

- [ ] **Step 3: 写最小实现**

`ui/src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.ts` 整文件替换：

```ts
/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import type { CompanionId, SshHostId } from '@/common/types/ids';

type ConversationListItem = Pick<TChatConversation, 'execution_step_id' | 'extra'>;

/** Attempt transcripts, companion-owned sessions, SSH-bound sessions and robot
 * threads have dedicated surfaces; they never re-enter the ordinary
 * work-conversation list. */
export const isOrdinaryWorkConversation = (conversation: ConversationListItem): boolean => {
  const extra = conversation.extra as
    | {
        is_health_check?: boolean;
        companion_session?: boolean;
        companion_id?: CompanionId;
        channel_platform?: string;
        ssh_host_id?: SshHostId;
        robot_session?: boolean;
        robot_id?: string;
      }
    | undefined;
  const isCompanionConversation =
    !!extra?.companion_session || !!extra?.companion_id || !!extra?.channel_platform;
  const isSshHostConversation = !!extra?.ssh_host_id;
  // Named explicitly rather than left to `companion_id`: a robot thread is a
  // device's long-lived conversation, and the companion marker it happens to
  // carry is the companion GROUP's key — leaning on it would put robot threads
  // back in this list the day that marker changes.
  const isRobotConversation = !!extra?.robot_session || !!extra?.robot_id;
  const isExecutionAttemptTranscript = Boolean(conversation.execution_step_id);
  return (
    extra?.is_health_check !== true &&
    !isCompanionConversation &&
    !isSshHostConversation &&
    !isRobotConversation &&
    !isExecutionAttemptTranscript
  );
};
```

- [ ] **Step 4: 跑测试确认通过**

Run: `bun test --cwd ui src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.test.ts src/renderer/services/i18n/robotLocales.test.ts`
Expected: PASS

全量收尾（仍不跑 Rust / UI 全量测试，只跑门禁与本计划触及的测试）：

Run: `bun run gen:i18n && bun run check:i18n && bun run typecheck && bun run check:theme && bun run check:icons && bun run check:dead-css`
Expected: PASS
Run: `bun test --cwd ui src/renderer/components/model/ src/renderer/pages/modelHub/ src/renderer/pages/nomi/ src/renderer/services/ src/common/adapter/ipcBridge.robot-status-wire.test.ts src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.test.ts`
Expected: PASS
Run: `cargo test -p nomifun-companion --lib && cargo test -p nomifun-api-types --lib shell::tests && cargo test -p nomifun-db --lib client_preference && cargo test -p nomifun-shell --lib routes::tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add ui/src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.ts \
        ui/src/renderer/pages/conversation/SessionList/hooks/conversationListFilter.test.ts \
        ui/src/renderer/services/i18n/robotLocales.test.ts \
        ui/src/renderer/services/i18n/i18n-keys.d.ts
git commit -m "fix(conversation): keep robot threads out of the ordinary session list"
```
