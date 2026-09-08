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
import GuidAgentSelectorPreview from './GuidAgentSelectorPreview';
import catalog from './fixtures/agent-workbench-catalog.json';
import seed from '../../crates/backend/nomifun-agent-contracts/contracts/presets/official-preset-seed-manifest.payload.json';

const PREVIEW_KEY = 'nomifun.agent-workbench.visual-preview.v1';
const OWNER = '0190f5fe-7c00-7a00-8000-000000000001';
const route = {
  schema: 'nomifun.chat-route-record.v1', task: 'agent_chat',
  primary: { model_route_id: '0190f5fe-7c00-7a00-8000-000000000002', model_route_revision: 1,
    provider_id: '0190f5fe-7c00-7a00-8000-000000000003', model: '预览模型', protocol: 'openai_chat',
    connection_config_ref: 'preview-connection', config_revision_digest: 'a'.repeat(64),
    credential_ref: 'preview-only-no-credential', features: ['text_input', 'text_output', 'tool_calls'] }, failovers: [],
};
const emptyDocument = () => ({ schema_version: '1.0.0', model_route_refs: { agent_chat: route.primary.model_route_id }, chat_route_records: { agent_chat: route }, initial_capabilities: [], on_demand_capabilities: [], skill_bindings: [], system_role_provider_overrides: {}, persona: '', instructions: '', starter_prompts: [] });
const selection = (id: string) => ({ capability: { id, version: '1.0.0' }, action_allowlist: [] });
const makeEditor = (id: string, name: string, document: any, revisionNumber = 1, description = '') => {
  document = { ...emptyDocument(), ...document, model_route_refs: { agent_chat: route.primary.model_route_id, ...document.model_route_refs }, chat_route_records: { agent_chat: route, ...document.chat_route_records } };
  const reference = { preset_id: id, revision: revisionNumber, revision_digest: 'a'.repeat(64) };
  const preset = { preset_id: id, owner_user_id: OWNER, source: 'user', display_name: name, description, current_stable_revision: reference, bound_target_count: 0 };
  return { preset, revision: { reference, document, created_by: OWNER, created_at_ms: Date.now() }, draft: { preset_id: id, display_name: name, description, current_revision: reference, document } };
};
const initialEditors = () => [
  makeEditor('0190f5fe-7c00-7a00-8000-000000000101', '研究助理', { initial_capabilities: ['knowledge.search', 'knowledge.read', 'memory.project.read'].map(selection), on_demand_capabilities: ['web.fetch', 'fs.read'].map(selection) }, 1, '查阅资料、整理信息，形成有据可查的结论'),
  makeEditor('0190f5fe-7c00-7a00-8000-000000000102', '开发搭档', { initial_capabilities: ['fs.read', 'fs.search', 'agent.execution.plan'].map(selection), on_demand_capabilities: ['fs.write', 'fs.patch', 'process.exec', 'vcs.diff', 'vcs.status'].map(selection) }, 1, '理解项目、修改代码，并检查改动结果'),
];
let editors: any[];
try { editors = JSON.parse(localStorage.getItem(PREVIEW_KEY) || 'null') || initialEditors(); } catch { editors = initialEditors(); }
const persist = () => localStorage.setItem(PREVIEW_KEY, JSON.stringify(editors));
const library = () => ({ official_templates: Object.entries(seed.templates).map(([key, value]) => ({ template_key: key, seed: value, role_coverage: { required_capability_categories: [], required_capability_ids: [], required_runtime_features: [], required_resource_kinds: [] }, immutable: true, forkable: true })), user_presets: editors.map((entry) => entry.preset), active_bindings: [], fresh_start: { data_generation: 4, legacy_data_imported: false, official_template_count: 7, user_preset_count: editors.length } });
const response = (data: unknown, status = 200) => new Response(JSON.stringify(status < 400 ? { success: true, data } : { success: false, error: data }), { status, headers: { 'Content-Type': 'application/json' } });
const preview = (draft: any) => {
  const map = (items: any[]) => items.map(({ capability }) => ({ capability, display_name: capability.id, source_package: catalog.find((row) => row.capability.id === capability.id)?.source_package, dependency_path: [], required_runtime_features: [] }));
  const initial = map(draft.document.initial_capabilities), onDemand = map(draft.document.on_demand_capabilities);
  const bad = [...initial, ...onDemand].filter((entry) => catalog.find((row) => row.capability.id === entry.capability.id)?.materialization_state !== 'materialized');
  const reference = { preset_id: draft.preset_id, revision: (draft.current_revision?.revision || 0) + 1, revision_digest: 'b'.repeat(64) };
  return { status: bad.length ? 'blocked' : 'ready', draft_digest: 'a'.repeat(64), preview_digest: 'b'.repeat(64), candidate_revision_ref: reference,
    diagnostics: bad.map((entry) => ({ severity: 'error', code: 'CAPABILITY_UNAVAILABLE', message: entry.display_name })),
    summary: { initial_count: initial.length, on_demand_count: onDemand.length, active_at_start_count: initial.length, model_tool_count: initial.length, context_contributor_count: 0, on_demand_index_count: onDemand.length, skill_count: 0, mcp_count: 0, required_resource_kind_count: 0, provider_initialization_count: 0 },
    revision_diff: { added_initial: [], removed_initial: [], added_on_demand: [], removed_on_demand: [], added_skills: [], removed_skills: [], model_routes_changed: false, instructions_changed: false },
    inspector: { required_runtime_protocol_version: '1.0.0', runtime_profile: 'managed_minimal', required_runtime_features: [], initial_capabilities: initial, on_demand_capabilities: onDemand, compact_on_demand_index: onDemand.map((entry) => entry.capability.id), tool_schema_refs: [], context_schema_refs: [], mcp_materializations: [], required_resource_kinds: [], service_key_diagnostics: [] },
    can_save_revision: bad.length === 0, can_create_session: false,
  };
};

// This transport serves fixed preview data only. Unrecognized calls fail; no
// request falls through to a real service, file tool, model, or plugin host.
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.origin);
  const path = url.pathname; const method = init?.method || 'GET'; const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
  if (path === '/api/agent-preset-templates') return response(library());
  if (path === '/api/capabilities') return response(catalog);
  if (path === '/api/agent-catalog/skills' || path === '/api/mcp-tool-mappings' || path.includes('providers')) return response([]);
  if (path === '/api/webui/access-token') return response({ configured: false });
  if (path === '/api/settings/client') return response({ language: 'zh-CN' });
  if (path === '/api/agent-presets' && method === 'POST') {
    const id = `0190f5fe-7c00-7a00-8000-${String(Date.now()).slice(-12)}`;
    const document = body.document || emptyDocument();
    if (preview({ preset_id: id, document }).status === 'blocked') return response('请先移除待处理的能力', 422);
    const entry = makeEditor(id, body.display_name, document, 1, body.description); editors.push(entry); persist(); return response(entry);
  }
  const match = path.match(/^\/api\/agent-presets\/([^/]+)(?:\/(.+))?$/);
  if (match) {
    const entry = editors.find((item) => item.preset.preset_id === match[1]);
    if (!entry) return response('Preview Agent not found', 404);
    if (match[2] === 'editor') return response(entry);
    if (match[2] === 'resolve-preview') return response(preview(body.draft));
    if (match[2] === 'revisions') {
      const updated = makeEditor(match[1], body.draft.display_name, body.draft.document, entry.revision.reference.revision + 1, body.draft.description);
      editors = editors.map((item) => item === entry ? updated : item); persist(); return response(updated);
    }
    if (method === 'DELETE') { editors = editors.filter((item) => item !== entry); persist(); return new Response(null, { status: 204 }); }
  }
  return response('This UI preview does not execute backend operations.', 404);
}) as typeof fetch;

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', fallbackLng: 'zh-CN', resources: { 'zh-CN': { translation: { agentSettings, common, guid } } }, interpolation: { escapeValue: false } });
const { default: AgentSettingsPage } = await import('../src/renderer/pages/agentSettings/AgentSettingsPage');
document.body.setAttribute('data-theme', 'light');
if (!location.hash) location.hash = '/agent';
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;min-height:100%;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif}body{background:var(--color-bg-1)}.preview-note{height:30px;display:flex;align-items:center;justify-content:center;background:var(--color-fill-2);color:var(--color-text-3);font-size:11px}.preview-frame{max-width:1220px;margin:auto}.preview-destination{padding:50px;font-size:16px}';
document.head.appendChild(style);
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN} theme={{ primaryColor: '#ef2355' }}><HashRouter><div className='preview-note'>交互预览 · 仅使用测试数据，不连接后台或执行 Agent</div><div className='preview-frame'><Routes><Route path='/selector' element={<GuidAgentSelectorPreview templates={library().official_templates as any} />} /><Route path='/agent' element={<AgentSettingsPage />} /><Route path='*' element={<div className='preview-destination'>此预览只验证工作台交互。<br /><Link to='/agent'>返回 Agent 工作台</Link></div>} /></Routes></div></HashRouter></ConfigProvider></I18nextProvider>);
