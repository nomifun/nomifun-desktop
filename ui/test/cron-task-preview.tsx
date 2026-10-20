/** Isolated desktop fixture. No request reaches a real backend or executes a task. */
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/modal-contract.css';
import zh from '../src/renderer/services/i18n/locales/zh-CN';
import { createEmptyAgentPresetDocument, asAgentPresetId, asDigestHex } from '../src/common/types/agentPlatform';
import type { AgentPresetLibraryResponse, OfficialPresetKey, OfficialPresetTemplate } from '../src/common/types/agentPlatform';
import type { ProviderResponse } from '../src/common/types/provider/providerApi';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '../src/renderer/pages/settings/DisplaySettings/presets';
import { processCustomCss } from '../src/renderer/utils/theme/customCssProcessor';
import { ensureThemeControlContract } from '../src/renderer/utils/theme/themeControlContract';

const providerId = '0190f5fe-7c00-7a00-8000-000000000201';
const personalId = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000202');
const officialId = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000203');
const provider: ProviderResponse = {
  provider_id: providerId, name: '已配置的模型服务', platform: 'custom', enabled: true,
  base_url: 'https://example.invalid/v1', auth_scheme: 'bearer', has_credentials: false,
  sort_order: 0, created_at: 1, updated_at: 1,
  models: ['日常模型', '研究模型'].map((model, sort_order) => ({
    provider_id: providerId, model, enabled: true, sort_order, created_at: 1, updated_at: 1,
    capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default',
      allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1 }],
  })),
};
const templates: OfficialPresetTemplate[] = ['assistant.general', 'chat.minimal', 'coding.codex'].map(key => ({
  template_key: key as OfficialPresetKey, immutable: true, forkable: true,
  seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
  role_coverage: { required_capability_categories: [], required_capability_ids: [], required_resource_kinds: [], required_runtime_features: [] },
}));
const library: AgentPresetLibraryResponse = {
  official_templates: templates,
  official_presets: { 'assistant.general': { preset_id: officialId, source: 'official', display_name: 'assistant.general', bound_target_count: 0 } },
  user_presets: [{ preset_id: personalId, source: 'user', display_name: '每日研究助理', bound_target_count: 0,
    current_stable_revision: { preset_id: personalId, revision: 1, revision_digest: asDigestHex('a'.repeat(64)) } }],
  active_bindings: [], fresh_start: { data_generation: 6, legacy_data_imported: false, official_template_count: 3, user_preset_count: 1 },
};
const preferences = { 'nomi.defaultModel': { provider_id: providerId, model: '日常模型' }, theme: 'light', colorScheme: 'default' };
const respond = (data: unknown) => new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const path = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href).pathname;
  if (init?.method && !['GET', 'HEAD'].includes(init.method) && path !== '/api/settings/client') {
    return new Response(JSON.stringify({ success: false, code: 'PREVIEW_ONLY', error: '交互预览不会保存或执行任务。' }), { status: 403 });
  }
  if (path === '/api/settings/client') return respond(preferences);
  if (path === '/api/providers') return respond([provider]);
  if (path === '/api/agent-preset-templates') return respond(library);
  if (path === '/api/agent-catalog') return respond({ modules: [], roles: [], skills: [], mcp_tools: [] });
  if (path.endsWith('/editor')) return respond({ preset: library.user_presets[0], draft: {
    preset_id: path.includes(officialId) ? officialId : personalId,
    display_name: '预览 Agent', document: createEmptyAgentPresetDocument(),
  } });
  return respond([]);
}) as typeof fetch;
class PreviewSocket extends EventTarget {
  static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
  readyState = PreviewSocket.CLOSED;
  send() {} close() {}
}
globalThis.WebSocket = PreviewSocket as unknown as typeof WebSocket;
const { initializeAgentBrowserStorageGeneration } = await import('../src/common/utils/browserStorageKey');
initializeAgentBrowserStorageGeneration(6);
const { configService } = await import('../src/common/config/configService');
await configService.initialize();
const { application } = await import('../src/common/adapter/ipcBridge');
application.applyKeepAwake.invoke = async () => {};
const { ThemeProvider } = await import('../src/renderer/hooks/context/ThemeContext');
const { default: CreateTaskDialog } = await import('../src/renderer/pages/cron/ScheduledTasksPage/CreateTaskDialog');
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: zh } } });
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;height:100%;min-width:880px;min-height:600px;font-family:Inter,system-ui,sans-serif;background:var(--bg-base);color:var(--text-primary)}.cron-preview-note{padding:20px;font-size:13px}';
document.head.append(style);
const theme = document.createElement('style');
theme.textContent = processCustomCss(PRESET_THEMES.find(item => item.id === DEFAULT_THEME_ID)!.css);
document.head.append(theme);
ensureThemeControlContract();
function Preview() {
  const [visible, setVisible] = useState(true);
  return <><p className='cron-preview-note'>定时任务交互预览 · 数据隔离，不保存或执行任务。<button onClick={() => setVisible(true)}>打开创建任务</button></p>
    <CreateTaskDialog visible={visible} onClose={() => setVisible(false)} /></>;
}
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN}><MemoryRouter><ThemeProvider><Preview /></ThemeProvider></MemoryRouter></ConfigProvider></I18nextProvider>);
