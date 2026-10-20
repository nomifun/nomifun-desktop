/** Development-only visual harness. No backend or Agent execution is connected. */
import React from 'react';
import { createRoot } from 'react-dom/client';
import { HashRouter, Route, Routes, Link } from 'react-router-dom';
import { ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/feedback-bubble-contract.css';
import '../src/renderer/styles/modal-contract.css';
import agentSettings from '../src/renderer/services/i18n/locales/zh-CN/agentSettings.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import guid from '../src/renderer/services/i18n/locales/zh-CN/guid.json';
import settings from '../src/renderer/services/i18n/locales/zh-CN/settings.json';
import AgentSelectorPreview from './AgentSelectorPreview';
import catalogDefinitions from './fixtures/agent-workbench-catalog.json';
import seed from '../../crates/backend/nomifun-agent-contracts/contracts/presets/official-agent-seed-manifest.payload.json';
import { createEmptyAgentPresetDocument } from '../src/common/types/agentPlatform';
import type { ProviderResponse } from '../src/common/types/provider/providerApi';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '../src/renderer/pages/settings/DisplaySettings/presets';
import { processCustomCss } from '../src/renderer/utils/theme/customCssProcessor';
import { ensureThemeControlContract, THEME_CONTROL_CONTRACT_STYLE_ID } from '../src/renderer/utils/theme/themeControlContract';

const PREVIEW_KEY = 'nomifun.agent-workbench.visual-preview.v5';
const OWNER = '0190f5fe-7c00-7a00-8000-000000000001';
const theme = new URLSearchParams(location.search).get('theme') === 'dark' ? 'dark' : 'light';
const providerId = '0190f5fe-7c00-7a00-8000-000000000003';
const providers: ProviderResponse[] = [{
  provider_id: providerId, platform: 'stepfun', name: 'StepFun',
  base_url: 'https://example.invalid/v1', auth_scheme: 'bearer', has_credentials: false,
  enabled: true, sort_order: 0, created_at: 1, updated_at: 1,
  models: ['step-3.5-flash', 'step-3.5-pro'].map((model, sort_order) => ({
    provider_id: providerId, model, enabled: true, sort_order, created_at: 1, updated_at: 1,
    capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default',
      allow_cross_origin_credentials: false, provider_params: {}, context_limit: 128000, created_at: 1, updated_at: 1 }],
  })),
}];
const clientSettings: Record<string, unknown> = { language: 'zh-CN', theme, colorScheme: 'default' };
const route = {
  schema: 'nomifun.chat-route-record.v1', task: 'agent_chat',
  primary: { model_route_id: '0190f5fe-7c00-7a00-8000-000000000002', model_route_revision: 1,
    provider_id: providerId, model: 'step-3.5-flash', protocol: 'openai_chat',
    connection_config_ref: 'preview-connection', config_revision_digest: 'a'.repeat(64),
    credential_ref: 'preview-only-no-credential', features: ['text_input', 'text_output', 'tool_calls'] }, failovers: [],
};
const emptyDocument = () => ({ ...createEmptyAgentPresetDocument(), model_route_refs: { agent_chat: route.primary.model_route_id }, chat_route_records: { agent_chat: route } });
const definitions = catalogDefinitions as Array<{ id: string; name: string; description: string; resources: string[]; actions: Array<[string, string]> }>;
const modules = definitions.map((definition) => ({
  module: { id: definition.id, version: '1.0.0' }, display_name: definition.name,
  description: definition.description, source_package: { id: `nomifun.${definition.id}`, version: '1.0.0' },
  authoring_policy: 'direct', summary_kind: 'tool',
  actions: definition.actions.map(([action_id, effect_class]) => ({ action_id,
    effect_class, presentation: 'function_tool' })),
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: definition.resources,
  required_host_ports: [], required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
}));
const catalog = { modules, capabilities: modules.map((module) => ({
  capability: module.module, kind: 'tool', display_name: module.display_name, description: module.description,
  source_package: module.source_package, source_kind: 'bundled', materialization_state: 'materialized',
  supported_surfaces: ['desktop'], required_runtime_features: [], required_resource_kinds: module.required_resource_kinds,
  required_capabilities: [], conflicting_capabilities: [], action_count: module.actions.length, context_contributor_count: 0,
})), skills: [], mcp_tools: [], roles: [] };
const selection = (id: string, action_allowlist: string[]) => ({ capability: { id, version: '1.0.0' }, action_allowlist });
const makeEditor = (id: string, name: string, document: any, revisionNumber = 1, description = '') => {
  document = { ...emptyDocument(), ...document };
  const reference = { preset_id: id, revision: revisionNumber, revision_digest: 'a'.repeat(64) };
  const preset = { preset_id: id, owner_user_id: OWNER, source: 'user', display_name: name, description, current_stable_revision: reference, bound_target_count: 0 };
  return { preset, revision: { reference, document, created_by: OWNER, created_at_ms: Date.now() }, draft: { preset_id: id, display_name: name, description, current_revision: reference, document } };
};
const initialEditors = () => [
  ...Object.entries(seed.templates).map(([key, value], index) => {
    const copy = key.split('.').reduce((current: any, part) => current?.[part],
      { ...agentSettings.template, 'customer-service': agentSettings.template.customerService, 'creative-studio': agentSettings.template.creativeStudio });
    const editor = makeEditor(`0190f5fe-7c00-7a00-8000-00000000020${index}`, copy?.name ?? key,
      { enabled_capabilities: value.enabled_capabilities, skill_bindings: value.skill_bindings }, 1, copy?.description ?? '');
    return { ...editor, preset: { ...editor.preset, source: 'official' }, draft: { ...editor.draft, source_template_key: key } };
  }),
  makeEditor('0190f5fe-7c00-7a00-8000-000000000101', '研究助理', { enabled_capabilities: [
    selection('knowledge', ['knowledge/search', 'knowledge/read']),
    selection('project.memory', ['project.memory/read']),
    selection('web.research', ['web.research/search', 'web.research/fetch']),
  ] }, 1, '查阅资料、整理信息，形成有据可查的结论'),
  makeEditor('0190f5fe-7c00-7a00-8000-000000000102', '开发搭档', { enabled_capabilities: [
    selection('workspace.files', ['workspace.files/read', 'workspace.files/search', 'workspace.files/write', 'workspace.files/patch']),
    selection('workspace.process', ['workspace.process/start', 'workspace.process/poll']),
    selection('workspace.vcs', ['workspace.vcs/diff', 'workspace.vcs/status']),
  ] }, 1, '理解项目、修改代码，并检查改动结果'),
];
let editors: any[];
try { editors = JSON.parse(localStorage.getItem(PREVIEW_KEY) || 'null') || initialEditors(); } catch { editors = initialEditors(); }
const persist = () => localStorage.setItem(PREVIEW_KEY, JSON.stringify(editors));
const library = () => ({ official_templates: Object.entries(seed.templates).map(([key, value]) => ({ template_key: key, seed: value, role_coverage: seed.role_coverage[key as keyof typeof seed.role_coverage], immutable: true, forkable: true })), official_presets: Object.fromEntries(editors.filter(entry => entry.preset.source === 'official').map(entry => [entry.draft.source_template_key, entry.preset])), user_presets: editors.filter(entry => entry.preset.source === 'user').map((entry) => entry.preset), active_bindings: [], fresh_start: { data_generation: 6, legacy_data_imported: false, official_template_count: Object.keys(seed.templates).length, user_preset_count: editors.filter(entry => entry.preset.source === 'user').length } });
const response = (data: unknown, status = 200) => new Response(JSON.stringify(status < 400 ? { success: true, data } : { success: false, error: data }), { status, headers: { 'Content-Type': 'application/json' } });
const hasUnavailableCapability = (document: any) => document.enabled_capabilities.some(
  ({ capability }: any) => catalog.capabilities.find((row) => row.capability.id === capability.id)?.materialization_state !== 'materialized'
);

// This transport serves fixed preview data only. Unrecognized calls fail; no
// request falls through to a real service, file tool, model, or plugin host.
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.origin);
  const path = url.pathname; const method = init?.method || 'GET'; const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
  if (path === '/api/agent-preset-templates') return response(library());
  if (path === '/api/agent-catalog') return response(catalog);
  if (path === '/api/capabilities') return response(catalog.capabilities);
  if (path === '/api/providers') return response(providers);
  if (path === '/api/agent-catalog/skills' || path === '/api/mcp-tool-mappings' || path === '/api/agent-role-defaults' || path === '/api/app-hosting/mcp-sources') return response([]);
  if (path === '/api/system/permissions') return response({ platform: 'windows', app_label: 'NomiFun', permissions: [
    { kind: 'accessibility', state: 'not_required', can_request: false, can_open_settings: false, requires_restart_after_grant: false, capabilities: ['computer_use'] },
    { kind: 'screen_recording', state: 'not_required', can_request: false, can_open_settings: false, requires_restart_after_grant: false, capabilities: ['computer_use'] },
  ] });
  if (path === '/api/auth/status') return new Response(JSON.stringify({ success: true, needs_setup: false }), { headers: { 'Content-Type': 'application/json' } });
  if (path === '/api/auth/user') return new Response(JSON.stringify({ success: true, user: { user_id: OWNER, username: 'preview' } }), { headers: { 'Content-Type': 'application/json' } });
  if (path === '/api/webui/access-token') return response({ configured: false });
  if (path === '/api/settings/client') {
    if (method === 'PUT') Object.assign(clientSettings, body);
    return response(clientSettings);
  }
  if (path === '/api/agent-presets' && method === 'POST') {
    const id = `0190f5fe-7c00-7a00-8000-${String(Date.now()).slice(-12)}`;
    const document = body.document || emptyDocument();
    if (hasUnavailableCapability(document)) return response('请先移除待处理的能力', 422);
    const entry = makeEditor(id, body.display_name, document, 1, body.description); editors.push(entry); persist(); return response(entry);
  }
  const match = path.match(/^\/api\/agent-presets\/([^/]+)(?:\/(.+))?$/);
  if (match) {
    const entry = editors.find((item) => item.preset.preset_id === match[1]);
    if (!entry) return response('Preview Agent not found', 404);
    if (match[2] === 'editor') return response(entry);
    if (match[2] === 'chat-model-route' && method === 'POST') {
      const provider = providers.find(candidate => candidate.provider_id === body.provider_id);
      const model = provider?.models.find(candidate => candidate.enabled && candidate.model === body.model);
      if (!model) return response('Preview chat model not found', 422);
      return response({ ...route, primary: { ...route.primary, provider_id: provider!.provider_id,
        model: model.model, model_route_id: model.sort_order === 0 ? route.primary.model_route_id : '0190f5fe-7c00-7a00-8000-000000000004' } });
    }
    if (match[2] === 'revisions') {
      if (hasUnavailableCapability(body.draft.document)) return response('请先移除待处理的能力', 422);
      const updated = makeEditor(match[1], body.draft.display_name, body.draft.document, entry.revision.reference.revision + 1, body.draft.description);
      updated.preset.source = entry.preset.source;
      updated.draft = { ...updated.draft, source_template_key: entry.draft.source_template_key };
      editors = editors.map((item) => item === entry ? updated : item); persist(); return response(updated);
    }
    if (method === 'DELETE') {
      if (entry.preset.source === 'official') return response('Official Agents cannot be deleted.', 409);
      editors = editors.filter((item) => item !== entry); persist(); return new Response(null, { status: 204 });
    }
  }
  return response('This UI preview does not execute backend operations.', 404);
}) as typeof fetch;

const { ipcBridge } = await import('../src/common');
const { setBrowserStorageGeneration, initializeAgentBrowserStorageGeneration } = await import('../src/common/utils/browserStorageKey');
// Explicit isolated dataset fixture replaces the normal backend bootstrap.
setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000004');
initializeAgentBrowserStorageGeneration(6);
// The preview does not open the production event socket. Model catalog updates
// and reconnect subscriptions are inert in this isolated document.
ipcBridge.mode.onProvidersChanged.on = () => () => {};
ipcBridge.conversation.reconnected.on = () => () => {};
const { configService } = await import('../src/common/config/configService');
await configService.initialize();

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', fallbackLng: 'zh-CN', resources: { 'zh-CN': { translation: { agentSettings, common, guid, settings } } }, interpolation: { escapeValue: false } });
const { default: AgentSettingsPage } = await import('../src/renderer/pages/agentSettings/AgentSettingsPage');
const { default: ModelAliasEditorPreview } = await import('./ModelAliasEditorPreview');
const { AuthProvider } = await import('../src/renderer/hooks/context/AuthContext');
document.documentElement.setAttribute('data-color-scheme', 'default');
document.documentElement.setAttribute('data-theme', theme);
document.body.setAttribute('arco-theme', theme);
document.body.setAttribute('data-theme', theme);
if (!location.hash) location.hash = '/agent';
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;height:100%;min-height:0;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif}body{background:var(--color-bg-1)}.preview-note{height:30px;display:flex;align-items:center;justify-content:center;background:var(--color-fill-2);color:var(--color-text-3);font-size:11px}.preview-frame{height:calc(100% - 30px);max-width:1220px;margin:auto}.preview-destination{padding:50px;font-size:16px}';
document.head.appendChild(style);
// Mirror the shell's processed, runtime-injected default preset and the final
// control contract so preview input colors and interaction states match it.
const themeStyle = document.createElement('style');
themeStyle.id = 'user-defined-custom-css';
themeStyle.textContent = processCustomCss(PRESET_THEMES.find(item => item.id === DEFAULT_THEME_ID)!.css);
const ensurePreviewTheme = () => {
  if (themeStyle.nextElementSibling?.id === THEME_CONTROL_CONTRACT_STYLE_ID && document.head.lastElementChild?.id === THEME_CONTROL_CONTRACT_STYLE_ID) return;
  document.head.appendChild(themeStyle);
  ensureThemeControlContract();
};
ensurePreviewTheme();
new MutationObserver(ensurePreviewTheme).observe(document.head, { childList: true });
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><AuthProvider><ConfigProvider locale={zhCN}><HashRouter><div className='preview-note'>交互预览 · 仅使用测试数据，不连接后台或执行 Agent</div><div className='preview-frame'><Routes><Route path='/model-editor' element={<ModelAliasEditorPreview />} /><Route path='/selector' element={<AgentSelectorPreview templates={library().official_templates as any} />} /><Route path='/agent' element={<AgentSettingsPage />} /><Route path='*' element={<div className='preview-destination'>此预览只验证工作台交互。<br /><Link to='/agent'>返回 Agent 工作台</Link></div>} /></Routes></div></HashRouter></ConfigProvider></AuthProvider></I18nextProvider>);
