import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import { parseMessageId, parsePersistedArtifactId } from '@/common/types/ids';
import { PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview/context/PreviewContext';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import { buildTurnFileItems, type TurnDeliverableItem } from '../turnDeliverablesModel';
import TurnDeliverablesCard from './TurnDeliverablesCard';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { messages, preview: { preview: 'Preview' } } } }, initImmediate: false });
const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000001');
const file = (path: string, operation: 'create' | 'modify' | 'delete' = 'create'): TurnDeliverableItem =>
  buildTurnFileItems({ turn_id: turn, complete: true, files: [{ identity: path, relative_path: path,
    file_name: path.split('/').at(-1)!, path: `/workspace/${path}`, operation, size_bytes: 22 }] })[0];
function PreviewProbe() {
  const preview = usePreviewContext();
  return <output data-testid='preview-path'>{preview.activeTab?.metadata?.file_path}</output>;
}
const card = (items: TurnDeliverableItem[], partial = false) => render(
  <I18nextProvider i18n={i18n}><PreviewProvider persistNamespace='turn-files-fixture' subscribeGlobalOpen={false}>
    <TurnDeliverablesCard items={items} workspace='/workspace' partial={partial} />
    <PreviewProbe />
  </PreviewProvider></I18nextProvider>
);
const flush = () => act(async () => {});

beforeEach(() => {
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.fs.getImageBase64, 'invoke').mockResolvedValue('data:image/png;base64,AAAA');
  spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({ name: 'report.docx', path: '/workspace/report.docx', size: 22, type: 'docx', lastModified: 1 });
});

test('published DOCX preview/open/reveal use the original source extension and never claim mutable-source integrity', async () => {
  const desktopWindow = window as Window & { __backendPort?: number };
  const previousPort = desktopWindow.__backendPort;
  const open = spyOn(ipcBridge.shell.openFile, 'invoke').mockResolvedValue(undefined as never);
  const reveal = spyOn(ipcBridge.shell.showItemInFolder, 'invoke').mockResolvedValue(undefined as never);
  try {
    desktopWindow.__backendPort = 8787;
    const items = buildTurnFileItems({ turn_id: turn, complete: true, files: [{
      identity: 'workspace:original.docx', relative_path: 'original.docx', file_name: 'original.docx',
      path: '/workspace/original.docx', operation: 'create', size_bytes: 22,
      published: { artifact_id: 'a'.repeat(64), path: `/workspace/.nomifun-artifacts/${'a'.repeat(64)}`,
        mime_type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', size_bytes: 22, sha256: 'a'.repeat(64) },
    }] });
    const view = card(items);
    await flush();
    expect(view.queryByTitle('Integrity verified (SHA-256)')).toBeNull();
    fireEvent.click(view.getByText('Preview'));
    await flush();
    expect(view.getByTestId('preview-path').textContent).toBe('/workspace/original.docx');
    fireEvent.click(view.getByTitle('Open'));
    fireEvent.click(view.getByTitle('Reveal in folder'));
    expect(open).toHaveBeenCalledWith('/workspace/original.docx');
    expect(reveal).toHaveBeenCalledWith('/workspace/original.docx');
  } finally {
    if (previousPort === undefined) delete desktopWindow.__backendPort;
    else desktopWindow.__backendPort = previousPort;
  }
});
afterEach(() => { cleanup(); mock.restore(); localStorage.clear(); });

test('renders command-generated files immediately and retains the existing list layout', async () => {
  const view = card([file('build_docx.py'), file('report.docx')]);
  expect(view.getByText('This turn involves 2 files')).toBeTruthy();
  expect(view.getByText('build_docx.py')).toBeTruthy();
  expect(view.getByText('report.docx')).toBeTruthy();
  expect(view.getByTestId('turn-deliverables-card').className).toBe('turn-deliverables');
  const row = view.container.querySelector('[data-deliverable-path="build_docx.py"]')!;
  expect(row.className).toBe('group flex items-center justify-between gap-8px px-12px py-6px hover:bg-3 transition-colors');
  await flush();
  expect(view.getAllByText('Added · 22 B')).toHaveLength(2);
});

test('deleted and missing files keep their historical rows and count, but hide unusable actions', async () => {
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue(null as never);
  const view = card([file('deleted.docx', 'delete'), file('missing.docx', 'modify')]);
  await flush();
  expect(view.getByText('This turn involves 2 files')).toBeTruthy();
  expect(view.getByText('Deleted')).toBeTruthy();
  expect(view.getByText('missing.docx')).toBeTruthy();
  expect(view.queryAllByText('Preview')).toHaveLength(0);
  expect(metadata).toHaveBeenCalledTimes(1);
  expect(metadata.mock.calls[0][0].path).toBe('/workspace/missing.docx');
});

test('an unresolved historical workspace never opens a same-name file from the current workspace', async () => {
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke');
  const items = buildTurnFileItems({ turn_id: turn, complete: false, files: [{
    identity: 'old-workspace:report.docx', relative_path: 'report.docx', file_name: 'report.docx', operation: 'modify', size_bytes: 22,
  }] });
  const view = card(items, true);
  await flush();
  expect(view.getByText('report.docx')).toBeTruthy();
  expect(metadata).not.toHaveBeenCalled();
  expect(view.queryByText('Preview')).toBeNull();
  expect(view.queryByTitle('Open')).toBeNull();
  expect(view.queryByTitle('Reveal in folder')).toBeNull();
});

test('availability failure cannot hide a file and focus retries a formerly missing file', async () => {
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockRejectedValueOnce(new Error('temporarily unavailable'))
    .mockResolvedValue({ name: 'report.docx', path: '/workspace/report.docx', size: 22, type: 'docx', lastModified: 1 });
  const view = card([file('report.docx')]);
  await flush();
  expect(view.getByText('report.docx')).toBeTruthy();
  expect(view.queryByText('Preview')).toBeNull();
  act(() => { window.dispatchEvent(new Event('focus')); });
  await flush();
  expect(metadata).toHaveBeenCalledTimes(2);
  expect(view.getByText('Preview')).toBeTruthy();
});

test('keeps the three-file fold and shows every remaining file on demand', async () => {
  const view = card(['a.docx', 'b.docx', 'c.docx', 'd.docx', 'e.docx'].map(path => file(path)));
  await flush();
  expect(view.getByText('This turn involves 5 files')).toBeTruthy();
  expect(view.container.querySelectorAll('[data-deliverable-path]')).toHaveLength(3);
  fireEvent.click(view.getByText('Show 2 more files'));
  await flush();
  expect(view.container.querySelectorAll('[data-deliverable-path]')).toHaveLength(5);
  fireEvent.click(view.getByText('Show less'));
  await flush();
  expect(view.container.querySelectorAll('[data-deliverable-path]')).toHaveLength(3);
});

test('hidden rows avoid metadata requests and expanded rows limit request concurrency', async () => {
  const pending: Array<() => void> = [];
  let active = 0;
  let maximum = 0;
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockImplementation(({ path }) => {
    active += 1;
    maximum = Math.max(maximum, active);
    return new Promise(resolve => pending.push(() => {
      active -= 1;
      resolve({ name: path.split('/').at(-1)!, path, size: 22, type: 'docx', lastModified: 1 });
    }));
  });
  const view = card(Array.from({ length: 10 }, (_, index) => file(`file-${index}.docx`)));
  expect(metadata).toHaveBeenCalledTimes(3);
  await act(async () => { pending.splice(0).forEach(resolve => resolve()); });
  fireEvent.click(view.getByText('Show 7 more files'));
  expect(active).toBe(4);
  while (pending.length) {
    await act(async () => { pending.splice(0).forEach(resolve => resolve()); });
  }
  expect(maximum).toBe(4);
  expect(view.container.querySelectorAll('[data-deliverable-path]')).toHaveLength(10);
});

test('multiple mounted Turn cards share the same metadata request budget', async () => {
  const pending: Array<() => void> = [];
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockImplementation(({ path }) =>
    new Promise(resolve => pending.push(() => resolve({ name: 'file.docx', path, size: 22, type: 'docx', lastModified: 1 }))));
  card(['a.docx', 'b.docx', 'c.docx'].map(path => file(path)));
  card(['d.docx', 'e.docx', 'f.docx'].map(path => file(path)));
  expect(metadata).toHaveBeenCalledTimes(4);
  while (pending.length) {
    await act(async () => { pending.splice(0).forEach(resolve => resolve()); });
  }
  expect(metadata).toHaveBeenCalledTimes(6);
});

test('partial status comes from canonical coverage rather than instructions to load messages', async () => {
  const view = card([file('a.docx')], true);
  await flush();
  expect(view.getByText('This turn involves 1 files (list incomplete)')).toBeTruthy();
  expect(view.getByTestId('turn-deliverables-card').getAttribute('data-partial')).toBe('true');
});

test('an incomplete empty summary shows the existing heading while confirmed empty files show no card', async () => {
  const partial = card([], true);
  await flush();
  expect(partial.getByText('This turn involves 0 files (list incomplete)')).toBeTruthy();
  expect(partial.container.querySelectorAll('[data-deliverable-path]')).toHaveLength(0);
  expect(partial.container.querySelector('.turn-deliverables__files-title')).toBeTruthy();
  partial.unmount();
  const complete = card([]);
  await flush();
  expect(complete.queryByTestId('turn-deliverables-card')).toBeNull();
});

test('ordinary image observations remain rows and verified image receipts retain the existing gallery', async () => {
  const image: TurnDeliverableItem = { identity: 'artifact', relativePath: 'generated.png', fileName: 'generated.png',
    absolutePath: '/workspace/generated.png', artifactId: parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000031'),
    artifactKind: 'image', mimeType: 'image/png', tier: 'receipt', sourceMessageIds: [], sizeBytes: 100 };
  const view = card([image, file('ordinary.png')]);
  await flush();
  expect(view.getByTestId('turn-deliverables-images').className).toBe('turn-deliverables__media-layout');
  expect(view.getByTestId('verified-image-artifact-card')).toBeTruthy();
  expect(view.container.querySelectorAll('[data-deliverable-path]')).toHaveLength(1);
  expect(view.getByText('ordinary.png')).toBeTruthy();
});

test('same-name files in different directories keep distinct existing path presentation', async () => {
  const view = card([file('a/report.docx'), file('b/report.docx')]);
  await flush();
  expect(view.container.querySelectorAll('[data-deliverable-path]')).toHaveLength(2);
  expect(view.getAllByText('report.docx')).toHaveLength(2);
  expect(view.getByText('a/')).toBeTruthy();
  expect(view.getByText('b/')).toBeTruthy();
});
