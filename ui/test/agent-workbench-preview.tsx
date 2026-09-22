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
import GuidAgentSelectorPreview from './GuidAgentSelectorPreview';
import catalogDefinitions from './fixtures/agent-workbench-catalog.json';
import seed from '../../crates/backend/nomifun-agent-contracts/contracts/presets/official-agent-seed-manifest.payload.json';

const PREVIEW_KEY = 'nomifun.agent-workbench.visual-preview.v2';
const OWNER = '0190f5fe-7c00-7a00-8000-000000000001';
const route = {
  schema: 'nomifun.chat-route-record.v1', task: 'agent_chat',
  primary: { model_route_id: '0190f5fe-7c00-7a00-8000-000000000002', model_route_revision: 1,
    provider_id: '0190f5fe-7c00-7a00-8000-000000000003', model: '预览模型', protocol: 'openai_chat',
    connection_config_ref: 'preview-connection', config_revision_digest: 'a'.repeat(64),
    credential_ref: 'preview-only-no-credential', features: ['text_input', 'text_output', 'tool_calls'] }, failovers: [],
};
const emptyDocument = () => ({ schema_version: '1.0.0', model_route_refs: { agent_chat: route.primary.model_route_id }, chat_route_records: { agent_chat: route }, enabled_capabilities: [], skill_bindings: [], system_role_provider_overrides: {}, persona: '', instructions: '', starter_prompts: [] });
const definitions = catalogDefinitions as Array<{ id: string; name: string; description: string; resources: string[]; actions: Array<[string, string]> }>;
const modules = definitions.map((definition) => ({
  module: { id: definition.id, version: '1.0.0' }, display_name: definition.name,
  description: definition.description, source_package: { id: `nomifun.${definition.id}`, version: '1.0.0' },
  authoring_policy: 'direct', summary_kind: 'tool',
  actions: definition.actions.map(([action_id, effect_class]) => ({ action_id,
    input_schema: `schema://${action_id}/input`, output_schema: `schema://${action_id}/output`,
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
  document = { ...emptyDocument(), ...document, model_route_refs: { agent_chat: route.primary.model_route_id, ...document.model_route_refs }, chat_route_records: { agent_chat: route, ...document.chat_route_records } };
  const reference = { preset_id: id, revision: revisionNumber, revision_digest: 'a'.repeat(64) };
  const preset = { preset_id: id, owner_user_id: OWNER, source: 'user', display_name: name, description, current_stable_revision: reference, bound_target_count: 0 };
  return { preset, revision: { reference, document, created_by: OWNER, created_at_ms: Date.now() }, draft: { preset_id: id, display_name: name, description, current_revision: reference, document } };
};
const initialEditors = () => [
  makeEditor('0190f5fe-7c00-7a00-8000-000000000101', '研究助理', { enabled_capabilities: [
    selection('knowledge', ['knowledge/search', 'knowledge/read']),
    selection('project.memory', ['project.memory/read']),
    selection('web.research', ['web.research/search', 'web.research/fetch']),
  ] }, 1, '查阅资料、整理信息，形成有据可查的结论'),
  makeEditor('0190f5fe-7c00-7a00-8000-000000000102', '开发搭档', { enabled_capabilities: [
    selection('workspace.files', ['workspace.files/read', 'workspace.files/search', 'workspace.files/write', 'workspace.files/patch']),
    selection('workspace.process', ['workspace.process/exec', 'workspace.process/poll']),
    selection('workspace.vcs', ['workspace.vcs/diff', 'workspace.vcs/status']),
  ] }, 1, '理解项目、修改代码，并检查改动结果'),
];
let editors: any[];
try { editors = JSON.parse(localStorage.getItem(PREVIEW_KEY) || 'null') || initialEditors(); } catch { editors = initialEditors(); }
const persist = () => localStorage.setItem(PREVIEW_KEY, JSON.stringify(editors));
const library = () => ({ official_templates: Object.entries(seed.templates).map(([key, value]) => ({ template_key: key, seed: value, role_coverage: seed.role_coverage[key as keyof typeof seed.role_coverage], immutable: true, forkable: true })), user_presets: editors.map((entry) => entry.preset), active_bindings: [], fresh_start: { data_generation: 6, legacy_data_imported: false, official_template_count: Object.keys(seed.templates).length, user_preset_count: editors.length } });
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
  if (path === '/api/agent-catalog/skills' || path === '/api/mcp-tool-mappings' || path.includes('providers')) return response([]);
  if (path === '/api/webui/access-token') return response({ configured: false });
  if (path === '/api/settings/client') return response({ language: 'zh-CN' });
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
    if (match[2] === 'revisions') {
      if (hasUnavailableCapability(body.draft.document)) return response('请先移除待处理的能力', 422);
      const updated = makeEditor(match[1], body.draft.display_name, body.draft.document, entry.revision.reference.revision + 1, body.draft.description);
      editors = editors.map((item) => item === entry ? updated : item); persist(); return response(updated);
    }
    if (method === 'DELETE') { editors = editors.filter((item) => item !== entry); persist(); return new Response(null, { status: 204 }); }
  }
  return response('This UI preview does not execute backend operations.', 404);
}) as typeof fetch;

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', fallbackLng: 'zh-CN', resources: { 'zh-CN': { translation: { agentSettings, common, guid, settings } } }, interpolation: { escapeValue: false } });
const { default: AgentSettingsPage } = await import('../src/renderer/pages/agentSettings/AgentSettingsPage');
const { default: ModelAliasEditorPreview } = await import('./ModelAliasEditorPreview');
document.body.setAttribute('data-theme', 'light');
if (!location.hash) location.hash = '/agent';
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;height:100%;min-height:0;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif}body{background:var(--color-bg-1)}.preview-note{height:30px;display:flex;align-items:center;justify-content:center;background:var(--color-fill-2);color:var(--color-text-3);font-size:11px}.preview-frame{height:calc(100% - 30px);max-width:1220px;margin:auto}.preview-destination{padding:50px;font-size:16px}';
document.head.appendChild(style);
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN} theme={{ primaryColor: '#ef2355' }}><HashRouter><div className='preview-note'>交互预览 · 仅使用测试数据，不连接后台或执行 Agent</div><div className='preview-frame'><Routes><Route path='/model-editor' element={<ModelAliasEditorPreview />} /><Route path='/selector' element={<GuidAgentSelectorPreview templates={library().official_templates as any} />} /><Route path='/agent' element={<AgentSettingsPage />} /><Route path='*' element={<div className='preview-destination'>此预览只验证工作台交互。<br /><Link to='/agent'>返回 Agent 工作台</Link></div>} /></Routes></div></HashRouter></ConfigProvider></I18nextProvider>);
