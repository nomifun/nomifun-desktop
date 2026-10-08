import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
const sendBoxSource = readFileSync(
  new URL('../../../components/chat/SendBox/index.tsx', import.meta.url),
  'utf8'
);
const nomiSendBoxSource = readFileSync(
  new URL('../platforms/nomi/NomiSendBox.tsx', import.meta.url),
  'utf8'
);

describe('edit/resubmit local suffix replacement', () => {

  test('exits edit mode only after a new immutable Turn is accepted', () => {
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
    const invoke = nomiHandler.indexOf('sendMessage.invoke({');
    const clearAttachments = nomiHandler.indexOf('clearFiles();', invoke);

    expect(invoke).toBeGreaterThan(-1);
    expect(nomiHandler.includes('removeMessagesByLocalIds')).toBe(false);
    expect(nomiHandler.includes('editResubmit.invoke')).toBe(false);
    expect(clearAttachments).toBeGreaterThan(invoke);
  });
});
