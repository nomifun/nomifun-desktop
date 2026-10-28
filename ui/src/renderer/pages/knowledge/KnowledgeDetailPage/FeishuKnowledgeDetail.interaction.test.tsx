import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { Message, Notification } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { feishuConnector } from '@/common/adapter/feishuConnectorBridge';
import type {
  IKnowledgeBase,
  IKnowledgeFileContent,
  IKnowledgeTreeEntry,
} from '@/common/adapter/ipcBridge';
import type { FeishuItem, FeishuQueryResult, FeishuStatus } from '@/common/types/feishuConnector';
import feishu from '@/renderer/services/i18n/locales/en-US/feishu.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import knowledge from '@/renderer/services/i18n/locales/en-US/knowledge.json';
import KnowledgeDetailPage from './index';

const KB_ID = '0190f5fe-7c00-7a00-8000-000000000001' as IKnowledgeBase['knowledge_base_id'];

const locale = createInstance();
const restores: Array<() => void> = [];
const track = <T extends { mockRestore(): void }>(spy: T) => {
  restores.push(() => spy.mockRestore());
  return spy;
};
const noopUnsub = () => () => {};
const actor = { app_id: 'real-app', user_id: 'real-user', name: 'Verified user' };
const empty = (): FeishuQueryResult => ({
  items: [],
  fields: [],
  has_more: false,
  next_cursor: null,
  warnings: [],
});

function status(): FeishuStatus {
  return {
    provider: 'feishu',
    connection: {
      id: 'connection',
      name: 'Work connection',
      enabled: true,
      profile: 'work',
      executable: null,
      created_at: '2026-10-26T00:00:00Z',
      updated_at: '2026-10-26T00:00:00Z',
    },
    runtime: {
      phase: 'ready',
      executable: 'C:/tools/lark-cli.exe',
      version: '1.0.98',
      source: 'external',
      message: null,
      official_install_command: 'npx @larksuite/cli@latest install',
    },
    phase: 'ready',
    actor: { ...actor },
    host_name: 'backend-machine',
    host_os: 'windows',
    message: null,
    permissions: ['documents.browse'].map((capability) => ({
      capability,
      granted: true,
      required_scopes: [capability + '-scope'],
    })),
  };
}

function remoteDoc(id: string, title: string, url: string): FeishuItem {
  return {
    id,
    title,
    kind: 'docx',
    url,
    modified_at: null,
    base_token: null,
    table_id: null,
    pending_operation_id: null,
    pending_status: null,
    fields: {},
  };
}

function base(overrides: Partial<IKnowledgeBase> = {}): IKnowledgeBase {
  return {
    knowledge_base_id: KB_ID,
    name: 'Feishu index',
    description: '',
    root_path: 'C:/data/kb',
    managed: true,
    tree_access: 'editable',
    created_at: 1,
    updated_at: 1,
    file_count: 2,
    total_size: 64,
    root_exists: true,
    tags: [],
    kind: 'feishu',
    source: { kind: 'feishu', mode: 'snapshot', entries: [], revision: 0 },
    ...overrides,
  };
}

function treeFile(name: string): IKnowledgeTreeEntry {
  return { name, rel_path: name, is_dir: false, is_file: true, modified_at: null };
}

function fileContent(path: string): IKnowledgeFileContent {
  return { rel_path: path, content: `# ${path}`, size: 8, modified_at: null };
}

beforeAll(async () => {
  await locale.init({
    lng: 'en-US',
    resources: { 'en-US': { translation: { feishu, common, knowledge } } },
  });
});
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach((restore) => restore());
});

function fixture(initial: IKnowledgeBase) {
  const treeRoot: IKnowledgeTreeEntry[] = [treeFile('README.md'), treeFile('Alpha.md')];
  track(spyOn(Message, 'success').mockReturnValue(() => {}));
  track(spyOn(Message, 'info').mockReturnValue(() => {}));
  track(spyOn(Message, 'warning').mockReturnValue(() => {}));
  track(spyOn(Message, 'error').mockReturnValue(() => {}));
  track(spyOn(Notification, 'warning').mockReturnValue({} as never));
  const listTree = track(
    spyOn(ipcBridge.knowledge.listTree, 'invoke').mockImplementation(
      async ({ path }: { knowledge_base_id: string; path?: string }) =>
        path ? [] : [...treeRoot],
    ),
  );
  const getBase = track(
    spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue(initial),
  );
  track(spyOn(ipcBridge.knowledge.listFiles, 'invoke').mockResolvedValue([]));
  track(spyOn(ipcBridge.knowledge.listTags, 'invoke').mockResolvedValue([]));
  track(spyOn(ipcBridge.knowledge.listConsumers, 'invoke').mockResolvedValue([]));
  track(
    spyOn(ipcBridge.knowledge.readFile, 'invoke').mockImplementation(
      async ({ path }: { knowledge_base_id: string; path: string }) =>
        fileContent(path),
    ),
  );
  const addContent = track(
    spyOn(ipcBridge.knowledge.addContent, 'invoke').mockImplementation(async () => {
      treeRoot.push(treeFile('Beta.md'));
      return {
        type: 'feishu' as const,
        added: 1,
        duplicates: 0,
        indexed: 1,
        failed: 0,
        errors: [],
        first_file: 'Beta.md',
      };
    }),
  );
  track(spyOn(ipcBridge.knowledge.onBaseUpdated, 'on').mockImplementation(noopUnsub));
  track(spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(noopUnsub));
  track(spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(noopUnsub));
  track(spyOn(ipcBridge.knowledge.onTagChanged, 'on').mockImplementation(noopUnsub));
  track(spyOn(ipcBridge.knowledge.onBindingChanged, 'on').mockImplementation(noopUnsub));
  track(spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]));
  track(spyOn(ipcBridge.mode.onProvidersChanged, 'on').mockImplementation(noopUnsub));
  track(spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(noopUnsub));
  const getStatus = track(
    spyOn(feishuConnector, 'status').mockImplementation(async () => status()),
  );
  const query = track(spyOn(feishuConnector, 'query').mockImplementation(async () => empty()));

  const mount = async () => {
    let view!: ReturnType<typeof render>;
    await act(async () => {
      view = render(
        <I18nextProvider i18n={locale}>
          <MemoryRouter initialEntries={[`/knowledge/${KB_ID}`]}>
            <Routes>
              <Route path='/knowledge/:id' element={<KnowledgeDetailPage />} />
            </Routes>
          </MemoryRouter>
        </I18nextProvider>,
      );
    });
    await waitFor(() => expect(getBase).toHaveBeenCalled());
    return view;
  };
  return { mount, query, addContent, getBase, listTree, getStatus };
}

describe('Feishu knowledge base detail page', () => {
  test('opens on the picker even when the base already has files', async () => {
    const f = fixture(base());
    f.query.mockResolvedValue({
      ...empty(),
      items: [remoteDoc('TokBeta', 'Beta doc', 'https://t.feishu.cn/docx/TokBeta')],
    });
    const view = await f.mount();
    await view.findByText(knowledge.detail.feishu.pickerTitle);
    await view.findByText('Adds to: Root');
    expect(view.queryByRole('button', { name: knowledge.detail.docs.preview })).toBeNull();
    // The left tree still lists the base's files.
    await view.findByText('README.md');
    await view.findByText('Alpha.md');
  });

  test('adding a remote doc keeps the picker and reveals the new stub', async () => {
    const f = fixture(base());
    f.query.mockResolvedValue({
      ...empty(),
      items: [remoteDoc('TokBeta', 'Beta doc', 'https://t.feishu.cn/docx/TokBeta')],
    });
    const view = await f.mount();
    fireEvent.click(await view.findByRole('button', { name: knowledge.detail.feishu.add }));
    await waitFor(() =>
      expect(f.addContent).toHaveBeenCalledWith({
        knowledge_base_id: KB_ID,
        type: 'feishu',
        docs: [{ url: 'https://t.feishu.cn/docx/TokBeta', title: 'Beta doc' }],
      }),
    );
    // The picker stays open; the base is re-fetched and the stub is revealed.
    await view.findByText('Beta.md');
    await view.findByText(knowledge.detail.feishu.pickerTitle);
    expect(f.getBase.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(view.queryByRole('button', { name: knowledge.detail.docs.preview })).toBeNull();
  });

  test('clicking a tree file shows the preview and the toolbar button returns to the picker', async () => {
    const f = fixture(base());
    const view = await f.mount();
    await view.findByText(knowledge.detail.feishu.pickerTitle);
    fireEvent.click(await view.findByText('Alpha.md'));
    await waitFor(() =>
      expect(view.queryByText(knowledge.detail.feishu.pickerTitle)).toBeNull(),
    );
    const browse = await view.findByRole('button', {
      name: knowledge.detail.feishu.browseDocs,
    });
    await view.findByRole('button', { name: knowledge.detail.docs.preview });
    fireEvent.click(browse);
    await view.findByText(knowledge.detail.feishu.pickerTitle);
  });

  test('a blank base never shows the picker', async () => {
    const f = fixture(base({ kind: 'blank', source: undefined }));
    const view = await f.mount();
    await view.findByRole('button', { name: knowledge.detail.docs.preview });
    expect(view.queryByText(knowledge.detail.feishu.pickerTitle)).toBeNull();
    expect(view.queryByRole('button', { name: knowledge.detail.feishu.browseDocs })).toBeNull();
  });
});
