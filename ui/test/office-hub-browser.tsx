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
import { officeHub } from '../src/common/adapter/officeHubBridge';
import type { OfficeItem, OfficeOperation, OfficeQueryResult, OfficeStatus } from '../src/common/types/officeHub';
import { uuidv7 } from '../src/common/utils/uuidv7';
import office from '../src/renderer/services/i18n/locales/zh-CN/office.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import OfficeHubPage from '../src/renderer/pages/officeHub';

const scenario = new URLSearchParams(location.search).get('scenario') ?? 'list';
const actor = { app_id: 'qa-app', user_id: 'qa-user', name: '离线验收' };
const now = new Date().toISOString();
const status: OfficeStatus = {
  provider: 'feishu', connection: { id: 'qa-connection', name: 'QA SIMULATED', enabled: true, profile: 'qa', executable: null, created_at: now, updated_at: now },
  runtime: { phase: 'ready', executable: 'QA SIMULATED', version: '1.0.98', source: 'external', message: null, official_install_command: 'npx @larksuite/cli@latest install' },
  phase: 'ready', actor, host_name: 'QA SIMULATED', host_os: 'windows', message: null,
  permissions: ['documents.browse', 'documents.delete', 'documents.create'].map(capability => ({ capability, granted: true, required_scopes: [] })),
};
const resource = (id: string, title: string): OfficeItem => ({
  id, title, kind: 'docx', url: 'https://qa.feishu.cn/docx/' + id, modified_at: now, opened_at: null, description: null,
  assignee_ids: [], creator_id: null, tasklist_ids: [], due: null, completed: null, start: null, end: null, calendar_id: null,
  base_token: null, table_id: null, pending_operation_id: null, pending_status: null, fields: {},
});
let resources = scenario === 'empty' ? [] : [resource('qa-document', 'nomifun-test-3'), resource('qa-second', '项目计划与工作记录')];
if (scenario === 'paged') resources = Array.from({ length: 25 }, (_, index) => resource('qa-' + index, '已有文档 ' + (index + 1)));
const operations = new Map<string, OfficeOperation>();
const browserLinks: string[] = [];
const recent: OfficeItem[] = [];
const empty = (): OfficeQueryResult => ({ items: [], fields: [], has_more: false, next_cursor: null, warnings: [] });
const report = () => { const output = window.document.querySelector('output'); if (output) output.textContent = JSON.stringify({ writes: operations.size, browserLinks }, null, 2); };
// Fail closed for every non-fixture method, including connection/authentication.
Object.assign(officeHub, Object.fromEntries(Object.keys(officeHub).map(key => [key, async () => { throw new Error(`QA SIMULATED: unsupported ${key}`); }])));
ipcBridge.shell.openExternal.invoke = async url => { browserLinks.push(url); report(); };
officeHub.rememberResource = async link => { const item = resources.find(item => item.url === link.url); if (item) recent.unshift(item); };
officeHub.status = async () => structuredClone(status);
officeHub.operations = async () => ({ items: [...operations.values()], next_cursor: null });
officeHub.query = async query => {
  if (query.kind === 'recent') return { ...empty(), items: structuredClone(recent) };
  if (query.kind !== 'documents') return empty();
  if (scenario === 'error') throw new Error('QA SIMULATED: document search is unavailable');
  const matches = resources.filter(item => item.title.includes(query.query) && (query.filter === 'all' || (query.filter === 'base' ? item.kind === 'base' : item.kind !== 'base')));
  const offset = Number(query.cursor ?? 0), items = matches.slice(offset, offset + 20), has_more = offset + items.length < matches.length;
  return { ...empty(), items: structuredClone(items), has_more, next_cursor: has_more ? String(offset + items.length) : null };
};
officeHub.execute = async request => {
  const original = operations.get(request.request_id);
  if (original) return structuredClone(original);
  const mutation = request.items[0];
  if (request.items.length !== 1 || !['document_create', 'document_delete'].includes(mutation.kind)) throw new Error('QA SIMULATED: unsupported mutation');
  const id = mutation.kind === 'document_delete' ? mutation.document_id : uuidv7();
  const unknown = scenario === 'pending' && mutation.kind === 'document_delete';
  if (!unknown && mutation.kind === 'document_delete') resources = resources.filter(item => item.id !== id);
  if (mutation.kind === 'document_create') resources.unshift(resource(id, mutation.title));
  const operation: OfficeOperation = {
    id: uuidv7(), request_id: request.request_id, connection_id: status.connection.id, actor, profile: 'qa', status: unknown ? 'unknown' : 'succeeded', created_at: now, updated_at: now,
    items: [{ id: uuidv7(), mutation, target_key: id, status: unknown ? 'unknown' : 'succeeded', remote_id: id, remote_url: null, message: null, attempts: [], observations: [] }],
  };
  if (unknown) { const item = resources.find(item => item.id === id)!; item.pending_operation_id = operation.id; item.pending_status = 'unknown'; }
  operations.set(request.request_id, operation); report();
  return structuredClone(operation);
};
officeHub.request = async id => { const operation = operations.get(id); if (!operation) throw new Error('QA SIMULATED: request not admitted'); return structuredClone(operation); };
officeHub.operation = async id => { const operation = [...operations.values()].find(operation => operation.id === id); if (!operation) throw new Error('QA SIMULATED: operation not found'); return structuredClone(operation); };

const locale = createInstance();
await locale.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { office, common } } } });
const router = createMemoryRouter([{ path: '/office', element: <OfficeHubPage /> }], { initialEntries: ['/office?tab=documents'] });
createRoot(window.document.getElementById('root')!).render(<I18nextProvider i18n={locale}><RouterProvider router={router} /></I18nextProvider>);
window.document.getElementById('root')!.style.cssText = 'height:calc(100vh - 76px);min-height:524px;min-width:880px';
const note = window.document.createElement('aside');
note.style.cssText = 'box-sizing:border-box;height:76px;padding:8px 20px;border-top:1px solid #ddd;font:12px/1.4 monospace;white-space:pre-wrap;overflow:auto';
note.innerHTML = 'QA SIMULATED · 实际 renderer / 离线业务边界 / 浏览器打开也仅记录链接<br><output>尚未操作</output>';
window.document.body.appendChild(note);
