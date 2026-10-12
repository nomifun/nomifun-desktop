import '../../../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import preview from '@/renderer/services/i18n/locales/en-US/preview.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import { PreviewProvider, usePreviewContext, type PreviewContextValue } from '../../context/PreviewContext';
import PreviewPanel from './PreviewPanel';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { preview, common } } }, interpolation: { escapeValue: false } });
beforeEach(() => {
  spyOn(ipcBridge.previewHistory.list, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
});
afterEach(() => { cleanup(); mock.restore(); localStorage.clear(); });

async function mountPanel() {
  let context!: PreviewContextValue;
  const Harness = () => { context = usePreviewContext(); return <PreviewPanel />; };
  const view = await act(async () => render(<I18nextProvider i18n={i18n}><PreviewProvider persistNamespace='panel-lifecycle-fixture' subscribeGlobalOpen={false}><Harness /></PreviewProvider></I18nextProvider>));
  return { view, context: () => context };
}

test('a mounted panel opens, closes and reopens without changing its hook order', async () => {
  const { view, context } = await mountPanel();
  expect(context().isOpen).toBe(false);
  expect(view.queryByText('server.cjs')).toBeNull();
  await act(async () => { context().openPreview('const value = 1;', 'code', { file_name: 'server.cjs', language: 'javascript', editable: false }); });
  expect(view.getByText('server.cjs')).toBeTruthy();
  await act(async () => { context().closePreview(); });
  expect(view.queryByText('server.cjs')).toBeNull();
  expect(context().tabs).toHaveLength(0);
  await act(async () => { context().openPreview('const value = 2;', 'code', { file_name: 'server.cjs', language: 'javascript', editable: false }); });
  expect(view.getByText('server.cjs')).toBeTruthy();
  expect(context().activeTab?.content).toBe('const value = 2;');
});

test('closing the final tab clears the mounted panel and a new file can still open', async () => {
  const { view, context } = await mountPanel();
  await act(async () => { context().openPreview('const a = 1;', 'code', { file_name: 'first.cjs', editable: false }); });
  await act(async () => { context().openPreview('const b = 2;', 'code', { file_name: 'second.cjs', editable: false }); });
  expect(context().tabs).toHaveLength(2);
  await act(async () => { context().closeTab(context().activeTabId!); });
  expect(context().tabs).toHaveLength(1);
  expect(view.getByText('first.cjs')).toBeTruthy();
  await act(async () => { context().closeTab(context().activeTabId!); });
  expect(context().tabs).toHaveLength(0);
  expect(context().activeTab).toBeNull();
  expect(view.queryByText('first.cjs')).toBeNull();
  await act(async () => { context().openPreview('const c = 3;', 'code', { file_name: 'reopened.cjs', editable: false }); });
  expect(view.getByText('reopened.cjs')).toBeTruthy();
});
