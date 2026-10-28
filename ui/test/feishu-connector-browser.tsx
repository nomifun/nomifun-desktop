/** Actual renderer, simulated business boundary. Never contacts Feishu or a backend. */
import { createRoot } from 'react-dom/client';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import '@arco-design/web-react/es/_util/react-19-adapter';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import { ipcBridge } from '../src/common';
import { feishuConnector } from '../src/common/adapter/feishuConnectorBridge';
import type { AuthStartRequest, AuthorizationFlow, FeishuActor, FeishuItem, FeishuOperation, FeishuProfile, FeishuQueryResult, FeishuStatus } from '../src/common/types/feishuConnector';
import { uuidv7 } from '../src/common/utils/uuidv7';
import feishu from '../src/renderer/services/i18n/locales/zh-CN/feishu.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import FeishuKnowledgePage from '../src/renderer/pages/knowledge/FeishuKnowledgePage';

const scenario = new URLSearchParams(location.search).get('scenario') ?? 'list';
const actor = { app_id: 'qa-app', user_id: 'qa-user', name: '离线验收' };
const now = new Date().toISOString();
const capabilityScopes: Record<string, string[]> = {
  'documents.browse': ['search:docs:read'], 'documents.create': ['docx:document:create'],
  'documents.delete': ['space:document:delete', 'drive:drive.metadata:readonly'],
  'base.read': ['base:record:read', 'base:field:read'], 'base.write': ['base:record:update'],
};
const status: FeishuStatus = {
  provider: 'feishu', connection: { id: 'qa-connection', name: 'QA SIMULATED', enabled: true, profile: 'qa', executable: null, created_at: now, updated_at: now },
  runtime: { phase: 'ready', executable: 'QA SIMULATED', version: '1.0.98', source: 'external', message: null, official_install_command: 'npx @larksuite/cli@latest install' },
  phase: 'ready', actor, host_name: 'QA SIMULATED', host_os: 'windows', message: null,
  permissions: Object.entries(capabilityScopes).map(([capability, required_scopes]) => ({ capability, granted: true, required_scopes })),
};
const clearPermissions = () => { for (const permission of status.permissions) permission.granted = false; };
if (['missing', 'needs-login', 'expired', 'broken', 'unconfigured'].includes(scenario)) {
  status.actor = null; clearPermissions(); status.phase = 'needs_login';
}
if (scenario === 'missing') {
  status.runtime = { ...status.runtime, phase: 'missing', executable: null, version: null, source: null };
  status.connection.profile = null; status.phase = 'not_configured';
} else if (scenario === 'expired') {
  status.phase = 'expired';
} else if (scenario === 'broken') {
  status.runtime.phase = 'broken'; status.runtime.message = 'QA SIMULATED: 已发现组件，但无法启动。';
} else if (scenario === 'unconfigured') {
  status.connection.profile = null; status.phase = 'not_configured';
}
const profiles: FeishuProfile[] = [{ name: 'qa', app_id: actor.app_id, brand: 'feishu', user_name: actor.name, token_status: 'valid' }];
type PreviewAuth = { flow: AuthorizationFlow; request: Pick<AuthStartRequest, 'kind' | 'profile' | 'scopes'> };
const flows = new Map<string, PreviewAuth>();
let activeFlowId: string | null = null;
let lastAction = '尚未操作';
const resource = (id: string, title: string): FeishuItem => ({
  id, title, kind: 'docx', url: 'https://qa.feishu.cn/docx/' + id, modified_at: now,
  base_token: null, table_id: null, pending_operation_id: null, pending_status: null, fields: {},
});
let resources = scenario === 'empty' ? [] : [resource('qa-document', 'nomifun-test-3'), resource('qa-second', '项目计划与工作记录')];
if (scenario === 'paged') resources = Array.from({ length: 25 }, (_, index) => resource('qa-' + index, '已有文档 ' + (index + 1)));
const operations = new Map<string, FeishuOperation>();
const browserLinks: string[] = [];
const empty = (): FeishuQueryResult => ({ items: [], fields: [], has_more: false, next_cursor: null, warnings: [] });
const activeFlow = () => activeFlowId ? flows.get(activeFlowId) : undefined;
const report = () => {
  const output = window.document.querySelector('output');
  if (output) output.textContent = JSON.stringify({ scenario, action: lastAction, writes: operations.size, browserLinks });
  window.document.querySelectorAll<HTMLButtonElement>('[data-auth-result]').forEach(button => { button.disabled = !activeFlow() || !['starting', 'waiting_user', 'waiting_admin'].includes(activeFlow()!.flow.phase); });
};
const completeAuth = (otherAccount = false) => {
  const entry = activeFlow(); if (!entry) return;
  const profile = entry.request.profile || status.connection.profile || 'qa';
  status.connection.profile = profile; status.connection.enabled = true;
  if (entry.request.kind === 'login') {
    status.actor = { ...actor, app_id: profiles.find(value => value.name === profile)?.app_id || actor.app_id,
      ...(otherAccount ? { user_id: 'qa-other-user', name: '另一离线账号' } : {}) };
    status.phase = 'ready';
    for (const permission of status.permissions) permission.granted ||= permission.required_scopes.every(scope => entry.request.scopes.includes(scope));
  } else {
    if (!profiles.some(value => value.name === profile)) profiles.push({ name: profile, app_id: actor.app_id, brand: 'feishu', user_name: null, token_status: null });
    status.actor = null; status.phase = 'needs_login'; clearPermissions();
  }
  entry.flow.phase = 'complete'; entry.flow.verification_url = null;
  lastAction = otherAccount ? '模拟完成另一账号登录' : '模拟完成授权'; report();
};
// Every unimplemented boundary fails locally. No method contacts a backend or CLI.
Object.assign(feishuConnector, Object.fromEntries(Object.keys(feishuConnector).map(key => [key, async () => { throw new Error(`QA SIMULATED: unsupported ${key}`); }])));
ipcBridge.shell.openExternal.invoke = async url => { browserLinks.push(url); report(); };
feishuConnector.status = async () => structuredClone(status);
feishuConnector.profiles = async () => structuredClone(profiles);
feishuConnector.prepare = async () => {
  status.runtime.phase = 'preparing'; lastAction = '模拟准备组件'; report();
  window.setTimeout(() => {
    status.runtime = { ...status.runtime, phase: 'ready', executable: 'QA SIMULATED', version: '1.0.98', source: 'shared_user', message: null };
    status.connection.profile ||= 'qa'; status.phase = status.actor ? 'ready' : 'needs_login';
    lastAction = '模拟组件准备完成'; report();
  }, 800);
  return structuredClone(status.runtime);
};
feishuConnector.repair = async () => {
  status.runtime.phase = 'preparing'; lastAction = '模拟修复组件'; report();
  window.setTimeout(() => {
    status.runtime = { ...status.runtime, phase: 'ready', executable: 'QA SIMULATED', version: '1.0.98', source: 'shared_user', message: null };
    status.connection.executable = 'QA SIMULATED';
    status.phase = status.actor ? 'ready' : 'needs_login';
    lastAction = '模拟修复完成'; report();
  }, 800);
  return structuredClone(status.runtime);
};
feishuConnector.checkUpdate = async () => ({ installed_version: status.runtime.version, latest_version: '1.0.99', compatible_version: '1.0.98', update_available: true, message: 'QA SIMULATED: 只读版本检查，不会安装。' });
feishuConnector.selectRuntime = async executable => {
  status.connection.executable = executable;
  status.runtime = { ...status.runtime, phase: 'ready', executable: executable || 'QA SIMULATED', version: '1.0.98', source: executable ? 'selected' : 'external', message: null };
  lastAction = '模拟选择组件'; report(); return structuredClone(status.runtime);
};
feishuConnector.updateConnection = async body => {
  if (body.profile !== status.connection.profile) { status.actor = null; clearPermissions(); }
  Object.assign(status.connection, body);
  status.phase = !body.enabled ? 'disabled' : !body.profile ? 'not_configured' : status.actor ? 'ready' : 'needs_login';
  lastAction = body.enabled ? '模拟启用连接' : '模拟停用连接'; report(); return structuredClone(status.connection);
};
feishuConnector.startAuth = async request => {
  if (activeFlow() && ['starting', 'waiting_user', 'waiting_admin', 'cancelling'].includes(activeFlow()!.flow.phase)) throw new Error('QA SIMULATED: 请先完成或取消当前授权。');
  const flow: AuthorizationFlow = { id: uuidv7(), kind: request.kind, phase: 'waiting_user', verification_url: 'https://open.feishu.cn/qa-simulated-auth', expires_at: null, missing_scopes: [], console_url: null, message: '离线演示：点击页面底部“模拟完成授权”，不会打开真实登录。' };
  flows.set(flow.id, { flow, request: { kind: request.kind, profile: request.profile, scopes: [...request.scopes] } });
  activeFlowId = flow.id; lastAction = '模拟等待授权'; report(); return structuredClone(flow);
};
feishuConnector.auth = async id => { const entry = flows.get(id); if (!entry) throw new Error('QA SIMULATED: authorization not found'); return structuredClone(entry.flow); };
feishuConnector.cancelAuth = async id => { const entry = flows.get(id); if (!entry) throw new Error('QA SIMULATED: authorization not found'); entry.flow.phase = 'cancelled'; entry.flow.verification_url = null; lastAction = '模拟取消授权'; report(); return structuredClone(entry.flow); };
Object.assign(feishuConnector, { logout: async (body: { profile: string; expected_actor: FeishuActor | null }) => {
  if (body.profile !== status.connection.profile || (body.expected_actor && (!status.actor || body.expected_actor.app_id !== status.actor.app_id || body.expected_actor.user_id !== status.actor.user_id))) throw new Error('QA SIMULATED: 退出目标身份已变化。');
  if (activeFlow() && ['starting', 'waiting_user', 'waiting_admin', 'cancelling'].includes(activeFlow()!.flow.phase)) throw new Error('QA SIMULATED: 授权仍在进行。');
  status.actor = null; status.phase = 'needs_login'; clearPermissions(); activeFlowId = null;
  lastAction = '模拟退出当前账号'; report();
} });
feishuConnector.query = async query => {
  if (query.kind !== 'documents') return empty();
  if (scenario === 'error') throw new Error('QA SIMULATED: document search is unavailable');
  const matches = resources.filter(item => item.title.includes(query.query) && (query.filter === 'all' || (query.filter === 'base' ? item.kind === 'base' : item.kind !== 'base')));
  const offset = Number(query.cursor ?? 0), items = matches.slice(offset, offset + 20), has_more = offset + items.length < matches.length;
  return { ...empty(), items: structuredClone(items), has_more, next_cursor: has_more ? String(offset + items.length) : null };
};
feishuConnector.execute = async request => {
  const original = operations.get(request.request_id);
  if (original) return structuredClone(original);
  const mutation = request.items[0];
  if (request.items.length !== 1 || !['document_create', 'document_delete'].includes(mutation.kind)) throw new Error('QA SIMULATED: unsupported mutation');
  const id = mutation.kind === 'document_delete' ? mutation.document_id : uuidv7();
  const unknown = scenario === 'pending' && mutation.kind === 'document_delete';
  if (!unknown && mutation.kind === 'document_delete') resources = resources.filter(item => item.id !== id);
  if (mutation.kind === 'document_create') resources.unshift(resource(id, mutation.title));
  const operation: FeishuOperation = {
    id: uuidv7(), request_id: request.request_id, connection_id: status.connection.id, actor: request.expected_actor, profile: request.profile, status: unknown ? 'unknown' : 'succeeded',
    items: [{ id: uuidv7(), mutation, target_key: id, status: unknown ? 'unknown' : 'succeeded', remote_id: id, remote_url: null, message: null, attempts: [], observations: [] }],
  };
  if (unknown) { const item = resources.find(item => item.id === id)!; item.pending_operation_id = operation.id; item.pending_status = 'unknown'; }
  operations.set(request.request_id, operation); report();
  return structuredClone(operation);
};
feishuConnector.request = async id => { const operation = operations.get(id); if (!operation) throw new Error('QA SIMULATED: request not admitted'); return structuredClone(operation); };
feishuConnector.operation = async id => { const operation = [...operations.values()].find(operation => operation.id === id); if (!operation) throw new Error('QA SIMULATED: operation not found'); return structuredClone(operation); };
feishuConnector.reconcile = async id => { const operation = [...operations.values()].find(operation => operation.id === id); if (!operation) throw new Error('QA SIMULATED: operation not found'); lastAction = '模拟读取原回执'; report(); return structuredClone(operation); };

const locale = createInstance();
await locale.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { feishu, common } } } });
const router = createMemoryRouter([{ path: '/knowledge/feishu', element: <FeishuKnowledgePage /> }], { initialEntries: ['/knowledge/feishu'] });
createRoot(window.document.getElementById('root')!).render(<I18nextProvider i18n={locale}><RouterProvider router={router} /></I18nextProvider>);
window.document.getElementById('root')!.style.cssText = 'height:calc(100vh - 76px);min-height:524px;min-width:880px';
const note = window.document.createElement('aside');
note.style.cssText = 'box-sizing:border-box;height:76px;padding:8px 20px;border-top:1px solid #ddd;font:12px/1.4 monospace;white-space:pre-wrap;overflow:auto';
note.innerHTML = '<div style="display:flex;align-items:center;gap:8px;margin-bottom:5px"><strong>QA SIMULATED · 全部操作仅在内存中模拟</strong><label>场景 <select aria-label="离线验收场景"></select></label><button type="button" data-auth-result="complete">模拟完成授权</button><button type="button" data-auth-result="other">模拟另一账号登录</button><button type="button" data-auth-result="deny">模拟拒绝授权</button></div><output>尚未操作</output>';
window.document.body.appendChild(note);
const scenarioSelect = note.querySelector('select')!;
for (const [value, label] of [['list','文档列表'],['missing','未安装组件'],['needs-login','未登录'],['connected','已连接'],['expired','登录过期'],['broken','组件损坏'],['unconfigured','未配置应用'],['paged','多页文档'],['error','读取错误'],['empty','空文档'],['pending','删除结果待核验']]) {
  const option = window.document.createElement('option'); option.value = value; option.textContent = label; option.selected = value === scenario; scenarioSelect.appendChild(option);
}
scenarioSelect.addEventListener('change', () => { const url = new URL(window.location.href); url.searchParams.set('scenario', scenarioSelect.value); window.location.href = url.href; });
note.querySelector('[data-auth-result="complete"]')!.addEventListener('click', () => completeAuth());
note.querySelector('[data-auth-result="other"]')!.addEventListener('click', () => completeAuth(true));
note.querySelector('[data-auth-result="deny"]')!.addEventListener('click', () => { const entry = activeFlow(); if (!entry) return; entry.flow.phase = 'denied'; entry.flow.verification_url = null; entry.flow.message = 'QA SIMULATED: 已模拟拒绝授权。'; lastAction = '模拟拒绝授权'; report(); });
report();
