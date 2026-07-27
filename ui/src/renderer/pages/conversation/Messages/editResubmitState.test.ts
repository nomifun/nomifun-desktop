import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

import type { TMessage } from '@/common/chat/chatLib';
import type { ConversationId, MessageId } from '@/common/types/ids';
import {
  removeMessagesByLocalIds,
  snapshotEditSuffixLocalIds,
} from './hooks';

const conversationId = '019fa2b0-6dc2-75c1-9b50-2742e02df27a' as ConversationId;
const targetMessageId = '019fa2b0-6dc2-75c1-9b50-2742e02df27b' as MessageId;
const sendBoxSource = readFileSync(
  new URL('../../../components/chat/SendBox/index.tsx', import.meta.url),
  'utf8'
);
const nomiSendBoxSource = readFileSync(
  new URL('../platforms/nomi/NomiSendBox.tsx', import.meta.url),
  'utf8'
);

const textMessage = (
  id: string,
  position: 'left' | 'right',
  createdAt: number,
  messageId?: MessageId
): TMessage => ({
  id,
  message_id: messageId,
  msg_id: messageId,
  conversation_id: conversationId,
  type: 'text',
  position,
  created_at: createdAt,
  content: { content: id },
});

describe('edit/resubmit local suffix replacement', () => {
  test('uses the durable target identity instead of deleting every same-millisecond row', () => {
    const list = [
      textMessage('same-ms-before-target', 'left', 100),
      textMessage('target', 'right', 100, targetMessageId),
      textMessage('old-assistant-tail', 'left', 101),
    ];

    const captured = snapshotEditSuffixLocalIds(list, targetMessageId, 100);

    expect([...captured]).toEqual(['target', 'old-assistant-tail']);
  });

  test('keeps replacement stream rows that arrive before the HTTP response', () => {
    const oldList = [
      textMessage('stable-prefix', 'left', 99),
      textMessage('target', 'right', 100, targetMessageId),
      textMessage('old-assistant-tail', 'left', 101),
    ];
    const captured = snapshotEditSuffixLocalIds(oldList, targetMessageId, 100);
    const replacement = textMessage('replacement-stream', 'left', 102);

    const result = removeMessagesByLocalIds([...oldList, replacement], captured);

    expect(result.map((message) => message.id)).toEqual([
      'stable-prefix',
      'replacement-stream',
    ]);
  });

  test('clears edit state and the old suffix only after backend acceptance', () => {
    const editSubmitBranch = sendBoxSource.slice(
      sendBoxSource.indexOf('if (editingMsgId && onEditResubmit) {'),
      sendBoxSource.indexOf('// Cancel any pending warmup:')
    );
    const submit = editSubmitBranch.indexOf(
      'onEditResubmit(targetId, targetCreatedAt, finalMessage)'
    );
    const accepted = editSubmitBranch.indexOf('.then(() => {', submit);
    const exitEditMode = editSubmitBranch.indexOf('setEditingMsgId(null);', submit);
    const clearInput = editSubmitBranch.indexOf("setInput('');", submit);

    expect(submit).toBeGreaterThan(-1);
    expect(accepted).toBeGreaterThan(submit);
    expect(exitEditMode).toBeGreaterThan(accepted);
    expect(clearInput).toBeGreaterThan(accepted);

    const nomiHandler = nomiSendBoxSource.slice(
      nomiSendBoxSource.indexOf('const handleEditResubmit = useCallback('),
      nomiSendBoxSource.indexOf('// Steering injects into the turn')
    );
    const invoke = nomiHandler.indexOf('editResubmit.invoke({');
    const removeOldSuffix = nomiHandler.indexOf(
      'removeMessagesByLocalIds(oldSuffixLocalIds);'
    );
    const clearAttachments = nomiHandler.indexOf('clearFiles();', invoke);

    expect(invoke).toBeGreaterThan(-1);
    expect(removeOldSuffix).toBeGreaterThan(invoke);
    expect(clearAttachments).toBeGreaterThan(invoke);
  });
});
