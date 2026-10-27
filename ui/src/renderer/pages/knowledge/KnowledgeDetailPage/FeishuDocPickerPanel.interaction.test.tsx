import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { officeHub } from '@/common/adapter/officeHubBridge';
import type { OfficeItem, OfficeQueryResult, OfficeStatus } from '@/common/types/officeHub';
import office from '@/renderer/services/i18n/locales/en-US/office.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import knowledge from '@/renderer/services/i18n/locales/en-US/knowledge.json';
import FeishuDocPickerPanel from './FeishuDocPickerPanel';

const locale = createInstance();
const restores: Array<() => void> = [];
const track = <T extends { mockRestore(): void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
const actor = { app_id: 'real-app', user_id: 'real-user', name: 'Verified user' };
const empty = (): OfficeQueryResult => ({ items: [], fields: [], has_more: false, next_cursor: null, warnings: [] });
function status(): OfficeStatus {
  return { provider: 'feishu', connection: { id: 'connection', name: 'Work connection', enabled: true, profile: 'work', executable: null, created_at: '2026-10-26T00:00:00Z', updated_at: '2026-10-26T00:00:00Z' }, runtime: { phase: 'ready', executable: 'C:/tools/lark-cli.exe', version: '1.0.98', source: 'external', message: null, official_install_command: 'npx @larksuite/cli@latest install' }, phase: 'ready', actor: { ...actor }, host_name: 'backend-machine', host_os: 'windows', message: null, permissions: ['documents.browse'].map(capability => ({ capability, granted: true, required_scopes: [capability + '-scope'] })) };
}
function doc(id: string, title: string, kind: string, url: string | null): OfficeItem {
  return { id, title, kind, url, modified_at: null, base_token: null, table_id: null, pending_operation_id: null, pending_status: null, fields: {} };
}
beforeAll(async () => { await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { office, common, knowledge } } } }); });
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });
function fixture(initial = status()) {
  let current = initial;
  track(spyOn(Message, 'success').mockReturnValue(() => {}));
  track(spyOn(Message, 'info').mockReturnValue(() => {}));
  track(spyOn(Message, 'warning').mockReturnValue(() => {}));
  const getStatus = track(spyOn(officeHub, 'status').mockImplementation(async () => structuredClone(current)));
  const query = track(spyOn(officeHub, 'query').mockImplementation(async () => empty()));
  const addContent = track(spyOn(ipcBridge.knowledge.addContent, 'invoke').mockResolvedValue({ type: 'feishu', added: 1, duplicates: 0, indexed: 1, failed: 0, errors: [] }));
  const mount = async (props?: Partial<Parameters<typeof FeishuDocPickerPanel>[0]>) => {
    const merged = {
      knowledgeBaseId: 'kb_test' as Parameters<typeof FeishuDocPickerPanel>[0]['knowledgeBaseId'],
      curatedUrls: [] as string[],
      destinationPath: '',
      hasSelection: false,
      onAdded: async () => {},
      ...props,
    };
    let view!: ReturnType<typeof render>;
    await act(async () => { view = render(<I18nextProvider i18n={locale}><MemoryRouter><FeishuDocPickerPanel {...merged} /></MemoryRouter></I18nextProvider>); });
    await waitFor(() => expect(getStatus).toHaveBeenCalled());
    return view;
  };
  return { mount, query, addContent, getStatus };
}

describe('Feishu document picker inside a knowledge base', () => {
  test('lists remote documents and one click appends the exact curated doc', async () => {
    const f = fixture();
    const item = doc('doc_real', 'Cloud spec', 'docx', 'https://team.feishu.cn/docx/doc_real');
    f.query.mockImplementation(async query => query.kind === 'documents' ? { ...empty(), items: [item] } : empty());
    const view = await f.mount();
    const addButton = await view.findByRole('button', { name: /Add to base/ });
    fireEvent.click(addButton);
    await waitFor(() => expect(f.addContent).toHaveBeenCalledWith({
      knowledge_base_id: 'kb_test',
      type: 'feishu',
      docs: [{ url: item.url, title: 'Cloud spec' }],
    }));
    await view.findByText(knowledge.detail.feishu.alreadyIn);
    expect(view.queryByRole('button', { name: /Add to base/ })).toBeNull();
  });

  test('search submits the exact term and load more follows the remote cursor', async () => {
    const f = fixture();
    f.query.mockImplementation(async () => ({ ...empty(), has_more: true, next_cursor: 'page-2' }));
    const view = await f.mount();
    await waitFor(() => expect(f.query).toHaveBeenCalledWith(expect.objectContaining({ kind: 'documents', filter: 'document', cursor: null }), expect.anything()));
    fireEvent.change(view.getByLabelText(office.resources.search), { target: { value: 'roadmap' } });
    fireEvent.submit(view.getByLabelText(office.resources.search).closest('form')!);
    await waitFor(() => expect(f.query).toHaveBeenCalledWith(expect.objectContaining({ query: 'roadmap' }), expect.anything()));
    f.query.mockImplementation(async query => 'cursor' in query && query.cursor === 'page-2' ? { ...empty(), items: [doc('d2', 'Page two', 'docx', 'https://team.feishu.cn/docx/d2')] } : empty());
    fireEvent.click(await view.findByRole('button', { name: /Load more/ }));
    await waitFor(() => expect(f.query).toHaveBeenCalledWith(expect.objectContaining({ cursor: 'page-2' }), expect.anything()));
    await view.findByText('Page two');
  });

  test('only doc/docx/wiki rows are addable and link-less rows stay disabled', async () => {
    const f = fixture();
    f.query.mockResolvedValue({ ...empty(), items: [
      doc('sheet_1', 'A sheet', 'sheet', 'https://team.feishu.cn/sheets/sheet_1'),
      doc('doc_1', 'Real doc', 'docx', null),
    ] });
    const view = await f.mount();
    await view.findByText('Real doc');
    expect(view.queryByText('A sheet')).toBeNull();
    const add = view.getByRole('button', { name: /Add to base/ }) as HTMLButtonElement;
    expect(add.disabled).toBe(true);
    fireEvent.click(add);
    expect(f.addContent).not.toHaveBeenCalled();
  });

  test('curated entries render as already in base without another add', async () => {
    const f = fixture();
    const url = 'https://team.feishu.cn/wiki/wiki_curated';
    f.query.mockResolvedValue({ ...empty(), items: [doc('wiki_curated', 'Curated wiki', 'wiki', url)] });
    const view = await f.mount({ curatedUrls: [url] });
    await view.findByText(knowledge.detail.feishu.alreadyIn);
    expect(view.queryByRole('button', { name: /Add to base/ })).toBeNull();
    expect(f.addContent).not.toHaveBeenCalled();
  });

  test('missing browse permission gates with authorization instead of querying or claiming empty', async () => {
    const denied = status();
    denied.permissions = denied.permissions.map(permission => ({ ...permission, granted: false }));
    const f = fixture(denied);
    const view = await f.mount();
    await view.findByRole('button', { name: office.actions.authorizeRead });
    expect(f.query).not.toHaveBeenCalled();
  });

  test('a disconnected connector explains setup instead of showing an empty list', async () => {
    const f = fixture({ ...status(), phase: 'needs_login', actor: null });
    const view = await f.mount();
    await view.findByRole('button', { name: knowledge.detail.feishu.connectAction });
    expect(f.query).not.toHaveBeenCalled();
  });

  test('root destination sends no placement fields and labels the root', async () => {
    const f = fixture();
    f.query.mockResolvedValue({ ...empty(), items: [doc('doc_root', 'Root doc', 'docx', 'https://team.feishu.cn/docx/doc_root')] });
    const view = await f.mount({ destinationPath: '' });
    await view.findByText('Adds to: Root');
    fireEvent.click(await view.findByRole('button', { name: /Add to base/ }));
    await waitFor(() => expect(f.addContent).toHaveBeenCalled());
    const request = f.addContent.mock.calls[0][0];
    expect(request).toMatchObject({
      knowledge_base_id: 'kb_test',
      type: 'feishu',
      docs: [{ url: 'https://team.feishu.cn/docx/doc_root', title: 'Root doc' }],
    });
    expect(request.destination_parent_path).toBeUndefined();
    expect(request.destination_parent_id).toBeUndefined();
  });

  test('a selected folder sends both destination fields and labels the folder', async () => {
    const f = fixture();
    f.query.mockResolvedValue({ ...empty(), items: [doc('doc_notes', 'Notes doc', 'docx', 'https://team.feishu.cn/docx/doc_notes')] });
    const view = await f.mount({
      destinationPath: 'notes',
      destinationEntryId: '0190f5fe-7c00-7a00-8000-000000000002' as Parameters<typeof FeishuDocPickerPanel>[0]['destinationEntryId'],
    });
    await view.findByText('Adds to: notes');
    fireEvent.click(await view.findByRole('button', { name: /Add to base/ }));
    await waitFor(() => expect(f.addContent).toHaveBeenCalledWith({
      knowledge_base_id: 'kb_test',
      type: 'feishu',
      docs: [{ url: 'https://team.feishu.cn/docx/doc_notes', title: 'Notes doc' }],
      destination_parent_path: 'notes',
      destination_parent_id: '0190f5fe-7c00-7a00-8000-000000000002',
    }));
  });

  test('a failed index shows only the warning toast, never success or duplicate', async () => {
    const f = fixture();
    f.addContent.mockResolvedValue({ type: 'feishu', added: 1, duplicates: 0, indexed: 0, failed: 1, errors: ['inspect failed'] });
    f.query.mockResolvedValue({ ...empty(), items: [doc('doc_fail', 'Failing doc', 'docx', 'https://team.feishu.cn/docx/doc_fail')] });
    const view = await f.mount();
    fireEvent.click(await view.findByRole('button', { name: /Add to base/ }));
    await waitFor(() => expect(Message.warning).toHaveBeenCalled());
    expect(Message.success).not.toHaveBeenCalled();
    expect(Message.info).not.toHaveBeenCalled();
  });

  test('duplicate additions report the curated state and never pretend a new index', async () => {
    const f = fixture();
    f.addContent.mockResolvedValue({ type: 'feishu', added: 0, duplicates: 1, indexed: 0, failed: 0, errors: [] });
    f.query.mockResolvedValue({ ...empty(), items: [doc('doc_dup', 'Duplicate', 'docx', 'https://team.feishu.cn/docx/doc_dup')] });
    const view = await f.mount();
    fireEvent.click(await view.findByRole('button', { name: /Add to base/ }));
    await waitFor(() => expect(f.addContent).toHaveBeenCalled());
    expect(Message.info).toHaveBeenCalled();
  });
});
