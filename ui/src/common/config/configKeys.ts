import type { SpeechToTextConfig, TextToSpeechConfig } from '@/common/types/provider/speech';
import type { ICssTheme } from '@/common/config/storage';
import type { AgentPresetId, CompanionId, ProviderId } from '@/common/types/ids';
import type { OfficialPresetKey } from '@/common/types/agentPlatform';
import type { LanguageMode } from './i18n';
import type { ThinkingContentDisplayLength, ThinkingSummaryDisplayLength } from './thinkingDisplay';

export type GuidAgentSelectionPreference =
  | { kind: 'template'; templateKey: OfficialPresetKey }
  | { kind: 'preset'; presetId: AgentPresetId };


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
  'nomi.defaultModel': { provider_id: ProviderId; model: string } | undefined;
  // 智能协作的模型偏好：除主模型（nomi.defaultModel）外，可为不同任务选择的
  // 额外模型。仅创建 Nomi 对话时使用；空数组表示只使用主模型。
  'nomi.collaborationModels': { provider_id: ProviderId; model: string }[] | undefined;
  // Default provider+model for the knowledge-base AI description/overview
  // generators (autogen / description.generate / description.polish). Empty
  // value = let the backend fall back to its own default completer model.
  'knowledge.autogenModel': { provider_id: ProviderId; model: string } | undefined;
  // Install-wide exact defaults for media tasks. When a generation default is
  // absent, ordinary conversations choose a ranked compatible task model.
  'models.default.imageGeneration': { provider_id: ProviderId; model: string } | undefined;
  'models.default.imageEdit': { provider_id: ProviderId; model: string } | undefined;
  'models.default.vision': { provider_id: ProviderId; model: string } | undefined;
  'models.default.videoGeneration': { provider_id: ProviderId; model: string } | undefined;
  'models.default.musicGeneration': { provider_id: ProviderId; model: string } | undefined;
  'models.default.speechSynthesis': { provider_id: ProviderId; model: string } | undefined;

  'tools.speechToText': SpeechToTextConfig | undefined;
  // Install-wide speech-synthesis default. Registered backend-side as a REQUIRED
  // Provider reference (nomifun-db client_preference), so an absent key — not a
  // blank object — is how "no default" is expressed.
  'tools.textToSpeech': TextToSpeechConfig | undefined;
  'workspace.pasteConfirm': boolean | undefined;
  'upload.saveToWorkspace': boolean | undefined;
  /** Explicit Agent Workbench default for newly-created Guid conversations. */
  'guid.defaultAgentSelection': GuidAgentSelectionPreference | undefined;
  /**
   * Legacy preference written by Guid's former "last selection wins" behavior.
   * Read only as an upgrade fallback; new code must not update it.
   */
  'guid.agentSelection': GuidAgentSelectionPreference | undefined;
  'system.notificationEnabled': boolean | undefined;
  'system.cronNotificationEnabled': boolean | undefined;
  'system.keepAwake': boolean | undefined;
  'system.autoPreviewOfficeFiles': boolean | undefined;
  // 发送键偏好：'enter'=Enter 发送/Shift+Enter 换行（默认）；'mod-enter'=Ctrl/⌘+Enter 发送、Enter 换行
  'chat.sendKey': 'enter' | 'mod-enter' | undefined;
  // Install-wide conversation presentation preferences. These only affect
  // rendering; the complete reasoning payload remains available in history.
  'chat.thinking.visible': boolean | undefined;
  'chat.thinking.contentLength': ThinkingContentDisplayLength | undefined;
  'chat.thinking.summaryLength': ThinkingSummaryDisplayLength | undefined;
  // Desktop control (computer-use): gates the nomi engine's Computer tool
  // (observe/click/type/launch). Read by the backend agent factory per session.
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
};

export type ConfigKey = keyof ConfigKeyMap;
