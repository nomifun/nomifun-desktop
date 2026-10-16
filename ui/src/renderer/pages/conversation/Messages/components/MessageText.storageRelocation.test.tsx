import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import type { ReactNode } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { IMessageText, IMessageToolCall } from '@/common/chat/chatLib';
import { NOMIFUN_FILES_MARKER } from '@/common/config/constants';
import { parseConversationId, parsePersistedArtifactId, parseMessageId } from '@/common/types/ids';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { PreviewProvider } from '@/renderer/pages/conversation/Preview';
import { usePreviewContext } from '@/renderer/pages/conversation/Preview/context/PreviewContext';
import MessageText from './MessageText';
import TurnDeliverablesCard from './TurnDeliverablesCard';
import { collectVerifiedTurnImages } from '../turnDeliverablesModel';
import { resolveMessageFilePath } from './messageFilePath';

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const workspace = 'D:/current/conversations/session';
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { preview: { preview: 'Preview' } } } } });

let restoreSubscriptions: Array<() => void> = [];
beforeEach(() => {
  const subscriptions = [
    spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockReturnValue(() => {}),
  ];
  restoreSubscriptions = subscriptions.map((subscription) => () => subscription.mockRestore());
});
afterEach(() => {
  cleanup();
  restoreSubscriptions.forEach((restore) => restore());
  localStorage.removeItem('storage-relocation-preview-test');
});

function PreviewPathProbe() {
  return <output data-testid='mapped-preview-path'>{usePreviewContext().activeTab?.metadata?.file_path}</output>;
}

const renderInSession = (child: ReactNode) => render(
  <MemoryRouter><I18nextProvider i18n={i18n}><ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', workspace }}>
    <PreviewProvider persistNamespace='storage-relocation-preview-test' subscribeGlobalOpen={false}>{child}</PreviewProvider>
  </ConversationProvider></I18nextProvider></MemoryRouter>
);

describe('canonical managed file locations', () => {
  test('only exact owned map entries relocate absolute history; ordinary relative and external paths retain their rules', () => {
    const original = 'C:/old/conversations/session/image.png';
    const mapped = `${workspace}/image.png`;
    const map = Object.assign(Object.create({ '/inherited.png': '/untrusted.png' }), { [original]: mapped }) as Record<string, string>;
    expect(resolveMessageFilePath(original, workspace, map)).toBe(mapped);
    expect(resolveMessageFilePath(`${original}.copy`, workspace, map)).toBe(`${original}.copy`);
    expect(resolveMessageFilePath('/inherited.png', workspace, map)).toBe('/inherited.png');
    expect(resolveMessageFilePath('uploads/image.png', workspace, map)).toBe(`${workspace}/uploads/image.png`);
    expect(resolveMessageFilePath('\\\\external\\share\\image.png', workspace, map)).toBe('\\\\external\\share\\image.png');
  });

  test('attachment markers and camera observations read mapped current files without changing historical content', async () => {
    const oldAttachment = 'C:/old/conversations/session/uploads/image.png';
    const oldObservation = 'C:/old/conversations/session/camera/scene.jpg';
    const currentAttachment = `${workspace}/uploads/image.png`;
    const currentObservation = `${workspace}/camera/scene.jpg`;
    const external = 'E:/selected/external.png';
    const raw = `Check these images\n\n${NOMIFUN_FILES_MARKER}\n${oldAttachment}\n${external}`;
    const message: IMessageText = { id: 'relocated-files', type: 'text', position: 'right', conversation_id: conversationId,
      content: { content: raw, managed_file_paths: { [oldAttachment]: currentAttachment, [oldObservation]: currentObservation },
        observations: [{ question: 'What is here?', answer: 'A cup', observed_at: 2,
          image: { id: 'camera', path: oldObservation, mime_type: 'image/jpeg', sha256: 'a'.repeat(64) } }] } };
    const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({ size: 8 } as Awaited<ReturnType<typeof ipcBridge.fs.getFileMetadata.invoke>>);
    const image = spyOn(ipcBridge.fs.getImageBase64, 'invoke').mockResolvedValue('');
    try {
      renderInSession(<MessageText message={message} />);
      await waitFor(() => {
        expect(metadata.mock.calls.map(([request]) => request.path)).toEqual([currentAttachment, external, currentObservation]);
        expect(image.mock.calls.map(([request]) => request.path)).toEqual([currentAttachment, external, currentObservation]);
      });
      expect(message.content.content).toBe(raw);
      expect(message.content.observations?.[0].image.path).toBe(oldObservation);
    } finally { cleanup(); metadata.mockRestore(); image.mockRestore(); }
  });

  test('canonical file preview opens the mapped current path while its displayed target remains historical', async () => {
    const original = 'C:/old/conversations/session/report.docx';
    const mapped = `${workspace}/report.docx`;
    const message: IMessageToolCall = { id: 'relocated-edit', type: 'tool_call', conversation_id: conversationId,
      content: { call_id: 'edit', name: 'apply_patch', status: 'completed', description: original,
        managed_file_paths: { [original]: mapped } } };
    const read = spyOn(ipcBridge.fs.readFile, 'invoke').mockResolvedValue('Current file');
    const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockResolvedValue({ size: 8 } as Awaited<ReturnType<typeof ipcBridge.fs.getFileMetadata.invoke>>);
    try {
      const view = renderInSession(<><TurnDeliverablesCard workspace={workspace} items={[{ identity: original, relativePath: 'report.docx', fileName: 'report.docx', absolutePath: mapped, operation: 'modify', tier: 'change', sourceMessageIds: [] }]} /><PreviewPathProbe /></>);
      fireEvent.click(await view.findByText('Preview'));
      await waitFor(() => expect(view.getByTestId('mapped-preview-path').textContent).toBe(mapped));
      expect(message.content.description).toBe(original);
    } finally { cleanup(); read.mockRestore(); metadata.mockRestore(); }
  });

  test('committed tool image artifacts read the exact mapped location while receipts keep their original paths', async () => {
    const original = 'C:/old/conversations/session/output.png';
    const mapped = `${workspace}/output.png`;
    const message: IMessageToolCall = { id: 'relocated-artifact', type: 'tool_call', conversation_id: conversationId,
      content: { call_id: 'write', name: 'write_file', status: 'completed',
        managed_file_paths: { [original]: mapped }, artifacts: [{ id: parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000052'), kind: 'image', path: original,
          relative_path: 'output.png', mime_type: 'image/png', size_bytes: 8, sha256: 'a'.repeat(64) }] } };
    const image = spyOn(ipcBridge.fs.getImageBase64, 'invoke').mockResolvedValue('data:image/png;base64,AA==');
    try {
      const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000053');
      const images = collectVerifiedTurnImages([{ turnId: turn, toolMessages: [message] }], new Map([[turn, { running: false, state: 'completed' }]]), [workspace]);
      renderInSession(<TurnDeliverablesCard workspace={workspace} items={images.get(turn) ?? []} />);
      await waitFor(() => expect(image.mock.calls[0]?.[0].path).toBe(mapped));
      expect(message.content.artifacts?.[0].path).toBe(original);
    } finally { cleanup(); image.mockRestore(); }
  });
});
