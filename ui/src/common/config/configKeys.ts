import type { SpeechToTextConfig, TextToSpeechConfig } from '@/common/types/provider/speech';
import type { ICssTheme } from '@/common/config/storage';
import type { CompanionId, ProviderId } from '@/common/types/ids';
import type { LanguageMode } from './i18n';

// `auto` (default), `headless` and `external` are the three supported user
// policies; `embedded` remains in the read type only so installations can
// migrate the removed viewer's persisted value. New product code persists only
// `auto`, `headless` or `external`.
export type BrowserDisplayMode = 'embedded' | 'external' | 'headless' | 'auto';

export type ConfigKeyMap = {
  'google.config': {
    proxy?: string;
  };
  language: string;
  languageMode: LanguageMode | undefined;
  theme: string;
  colorScheme: string;
  'ui.zoomFactor': number | undefined;
  'window.bounds': { x?: number; y?: number; width: number; height: number } | undefined;
  'webui.desktop.enabled': boolean | undefined;
  'webui.desktop.allowRemote': boolean | undefined;
  'webui.desktop.port': number | undefined;
  customCss: string;
  'css.themes': ICssTheme[];
  'css.activeThemeId': string;
  'nomi.config': { preferredMode?: string } | undefined;
  'nomi.defaultModel': { provider_id: ProviderId; model: string } | undefined;
  // 智能协作的模型偏好：除主模型（nomi.defaultModel）外，可为不同任务选择的
  // 额外模型。仅创建 Nomi 对话时使用；空数组表示只使用主模型。
  'nomi.collaborationModels': { provider_id: ProviderId; model: string }[] | undefined;
  // Default provider+model for the knowledge-base AI description/overview
  // generators (autogen / description.generate / description.polish). Empty
  // value = let the backend fall back to its own default completer model.
  'knowledge.autogenModel': { provider_id: ProviderId; model: string } | undefined;
  // Install-wide default for the native image-generation task. Missing means
  // the backend may choose from the available image models (for example by
  // round-robin); there is no separate tool enable switch.
  'models.default.imageGeneration': { provider_id: ProviderId; model: string } | undefined;
  'tools.speechToText': SpeechToTextConfig | undefined;
  // Install-wide speech-synthesis default. Registered backend-side as a REQUIRED
  // Provider reference (nomifun-db client_preference), so an absent key — not a
  // blank object — is how "no default" is expressed.
  'tools.textToSpeech': TextToSpeechConfig | undefined;
  'workspace.pasteConfirm': boolean | undefined;
  'upload.saveToWorkspace': boolean | undefined;
  'guid.lastSelectedAgent': string | undefined;
  'system.notificationEnabled': boolean | undefined;
  'system.cronNotificationEnabled': boolean | undefined;
  'system.keepAwake': boolean | undefined;
  'system.autoPreviewOfficeFiles': boolean | undefined;
  // 发送键偏好：'enter'=Enter 发送/Shift+Enter 换行（默认）；'mod-enter'=Ctrl/⌘+Enter 发送、Enter 换行
  'chat.sendKey': 'enter' | 'mod-enter' | undefined;
  // Desktop control (computer-use): gates the nomi engine's Computer tool
  // (observe/click/type/launch). Read by the backend agent factory per session.
  'agent.computerUse': boolean | undefined;
  // Browser control (browser-use): gates the nomi engine's built-in browser
  // tools (native CDP engine). ON by default on browser-use (desktop) builds; it
  // opens an isolated managed Chrome / Edge instance. Routine work is silent
  // and headless unless the installation owner explicitly selects `external`.
  // Read by the backend agent factory per session.
  'agent.browserUse': boolean | undefined;
  // Application-level browser default visibility policy. New installs persist
  // `auto` (the host resolves visibility per lane, staying silent for routine
  // work); the user may pin `headless` (never visible) or `external`
  // (default-visible Primary). Historical `embedded`, unversioned, and legacy
  // `agent.browserUse.silent` state all fail closed to `auto`, which still
  // launches silently. Agent tool input can only declare intent, never select
  // the mode.
  'agent.browserUse.displayMode': BrowserDisplayMode | undefined;
  // Lineage marker for an explicit visibility policy. Only the current version
  // plus a valid displayMode is authoritative as a local fallback; a v2 marker
  // is still recognized so an explicit `external` survives migration. The live
  // owner API remains authoritative.
  'agent.browserUse.displayModeVersion': 2 | 3 | undefined;
  // Legacy compatibility read only. New settings code must not write this key.
  // Visibility migration no longer derives an external window from this key.
  // Elastic crawl/replica/isolated hosts choose headless execution internally.
  'agent.browserUse.silent': boolean | undefined;
  // Browser source (browser-use sub-setting, orthogonal to silent): 'managed' =
  // bundled/downloaded Chrome for Testing; 'system' (default) = the user's
  // installed Chrome/Edge binary (still an isolated profile — never the real
  // profile). Read by the backend agent factory per session.
  'agent.browserUse.source': 'managed' | 'system' | undefined;
  // Persistent login (browser-use sub-setting): keeps cookies/storage across
  // sessions in an encrypted vault. ON by default. When on, evaluate full-power
  // mode is blocked (security mutex). Read by the backend browser engine.
  'agent.browserUse.persistentLogin': boolean | undefined;
  // Full-power browser evaluate mode: unlocks arbitrary page-script evaluation.
  // OFF by default and mutually exclusive with persistent login on the backend.
  'agent.browserUse.fullPower': boolean | undefined;
  // Site memory (browser-use sub-setting): persists per-site interaction hints to
  // disk + injects them into the agent's context. OFF by default (opt-in,
  // privacy-relevant). Read by the backend browser factory.
  'agent.browserUse.siteMemory': boolean | undefined;
  // Human takeover / approval (browser-use sub-setting): irreversible browser
  // actions + gated cross-origin POSTs are held for the user's approval instead of
  // hard-blocked. ON by default. Read by the backend agent factory.
  'agent.browserUse.takeover': boolean | undefined;
  // Dangerous Browser Use approval bypass: skips Browser-specific irreversible
  // action and gated egress confirmations. OFF by default.
  'agent.browserUse.unrestrictedApproval': boolean | undefined;
  // Visual fallback (browser-use sub-setting): when DOM/aria anchoring fails, the
  // agent screenshots the page and asks the vision model to locate the target, then
  // clicks the mapped point. OFF by default (opt-in, vision-token cost). Read by the
  // backend agent factory.
  'agent.browserUse.visualFallback': boolean | undefined;
  'channels.telegram.agent':
    | { agent_type: string; backend?: string; name?: string }
    | undefined;
  // Companion binding per IM channel platform (mirror of the backend
  // client-preference written by POST /api/channel/settings/companion).
  // Empty/missing = no binding → no companion greets this platform's channel.
  'channels.telegram.companion_id': CompanionId | undefined;
  'channels.lark.agent':
    | { agent_type: string; backend?: string; name?: string }
    | undefined;
  'channels.lark.companion_id': CompanionId | undefined;
  'channels.dingtalk.agent':
    | { agent_type: string; backend?: string; name?: string }
    | undefined;
  'channels.dingtalk.companion_id': CompanionId | undefined;
  'channels.weixin.agent':
    | { agent_type: string; backend?: string; name?: string }
    | undefined;
  'channels.weixin.companion_id': CompanionId | undefined;
  'channels.wecom.agent':
    | { agent_type: string; backend?: string; name?: string }
    | undefined;
  'channels.wecom.companion_id': CompanionId | undefined;
  'skillsMarket.enabled': boolean | undefined;
  // Computer-history master switch. Gates the native observer + the
  // `computer_history_*` agent tools; read by the backend per session. The
  // settings page writes it via the shared client-preference channel.
  'feature.computerHistory': boolean | undefined;
};

export type ConfigKey = keyof ConfigKeyMap;
