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
import { officeHub } from '../src/common/adapter/officeHubBridge';
import type { OfficeDocument, OfficeOperation, OfficeQueryResult, OfficeStatus } from '../src/common/types/officeHub';
import { uuidv7 } from '../src/common/utils/uuidv7';
import office from '../src/renderer/services/i18n/locales/zh-CN/office.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import OfficeHubPage from '../src/renderer/pages/officeHub';

const scenario = new URLSearchParams(location.search).get('scenario') ?? 'paragraph';
const actor = { app_id: 'qa-app', user_id: 'qa-user', name: '离线验收' };
const now = new Date().toISOString();
const status: OfficeStatus = {
  provider: 'feishu', connection: { id: 'qa-connection', name: 'QA SIMULATED', enabled: true, profile: 'qa', executable: null, created_at: now, updated_at: now },
  runtime: { phase: 'ready', executable: 'QA SIMULATED', version: '1.0.98', source: 'external', message: null, official_install_command: 'npx @larksuite/cli@latest install' },
  phase: 'ready', actor, host_name: 'QA SIMULATED', host_os: 'windows', message: null,
  permissions: ['documents.read', 'documents.write', 'documents.search', 'documents.create'].map(capability => ({ capability, granted: true, required_scopes: [] })),
};
let text = 'nomifun-test-3感觉';
let revision = 28;
const operations = new Map<string, OfficeOperation>();
const empty = (): OfficeQueryResult => ({ items: [], document: null, fields: [], has_more: false, next_cursor: null, warnings: [] });
const literal = (value: string) => value.replace(/[\\`*_\[\]#$~<>&|]/g, '\\$&');
function qaDocument(editing: boolean): OfficeDocument {
  return {
    id: 'qa-document', title: 'nomifun-test-3', content: scenario === 'empty' && revision === 28 ? '' : literal(text), url: null,
    pending_operation_id: null, pending_status: null,
    edit_baseline: !editing ? null : {
      revision: String(revision), can_insert_first: scenario === 'empty' && revision === 28,
      blocks: scenario === 'empty' && revision === 28 ? [] : [{ id: 'qa-paragraph', kind: 'paragraph', content: text, editable: scenario !== 'readonly', readonly_reason: scenario === 'readonly' ? 'rich_text' : null }],
    },
  };
}
// Fail closed for every non-fixture method, including connection/authentication.
Object.assign(officeHub, Object.fromEntries(Object.keys(officeHub).map(key => [key, async () => { throw new Error(`QA SIMULATED: unsupported ${key}`); }])));
officeHub.status = async () => structuredClone(status);
officeHub.operations = async () => ({ items: [...operations.values()], next_cursor: null });
officeHub.query = async query => query.kind === 'document' ? { ...empty(), document: qaDocument(query.editing) } : empty();
officeHub.execute = async request => {
  const original = operations.get(request.request_id);
  if (original) return structuredClone(original);
  const mutation = request.items[0];
  if (request.items.length !== 1 || mutation.kind !== 'document_save' || mutation.document_id !== 'qa-document' || mutation.revision !== String(revision)) throw new Error('QA SIMULATED: invalid document target or baseline');
  text = mutation.content; revision++;
  const operation: OfficeOperation = {
    id: uuidv7(), request_id: request.request_id, connection_id: status.connection.id, actor, profile: 'qa', status: 'succeeded', created_at: now, updated_at: now,
    items: [{ id: uuidv7(), mutation, target_key: 'qa-document', status: 'succeeded', remote_id: 'qa-document', remote_url: null, message: null, attempts: [], observations: [] }],
  };
  operations.set(request.request_id, operation);
  const output = window.document.querySelector('output');
  if (output) output.textContent = JSON.stringify({ writes: operations.size, revision, block_id: 'qa-paragraph', content: text }, null, 2);
  return structuredClone(operation);
};
officeHub.request = async id => { const operation = operations.get(id); if (!operation) throw new Error('QA SIMULATED: request not admitted'); return structuredClone(operation); };

const locale = createInstance();
await locale.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { office, common } } } });
const router = createMemoryRouter([{ path: '/office', element: <OfficeHubPage /> }], { initialEntries: ['/office?tab=documents&document=qa-document'] });
createRoot(window.document.getElementById('root')!).render(<I18nextProvider i18n={locale}><RouterProvider router={router} /></I18nextProvider>);
window.document.getElementById('root')!.style.cssText = 'height:calc(100vh - 76px);min-height:524px;min-width:880px';
const note = window.document.createElement('aside');
note.style.cssText = 'box-sizing:border-box;height:76px;padding:8px 20px;border-top:1px solid #ddd;font:12px/1.4 monospace;white-space:pre-wrap;overflow:auto';
note.innerHTML = 'QA SIMULATED · 实际 renderer / 离线业务边界 / 不连接飞书<br><output>尚未写入</output>';
window.document.body.appendChild(note);
